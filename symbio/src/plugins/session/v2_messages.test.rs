//! 消息变更**写方**的判据（S2.7-b1，[ADR-053](../../../../docs/decisions/session.md)）。
//!
//! 写侧的失效形态与读侧相反：读侧是「折不出来」，写侧是**落不下去**——而且它
//! **静默**（缓存照样写成功、用户照样看到「改好了」，事实源里却没有这一笔）。
//! 所以这里逐条钉**落下去的那条事实长什么样**：坐标、载荷、溯源、幂等键。
//!
//! ## 为什么这些用例敢直接调写方
//!
//! 生产路径是 `commands::patch_message` / `truncate_messages` → 本模块（见那两个
//! 方法的文档）。两条 `append` 调用点各一行、没有分支，而 VDFS 边界的回执 / 变更帧
//! 已由 `vdfs_provider.test.rs` 覆盖——同一段语义不在两处各钉一遍
//! （`docs/module-layout.md` §2）。本文件只钉「事实本身」。

use super::{drain_parked, edited, removed};
use crate::symbio_core::chat_message::{ChatMessage, MessageContent, MessageRole, MessageType};
use crate::symbio_core::{
    Entity, EventWalStore, Seq, Store, Verb, EVENT_MESSAGE_EDITED, EVENT_MESSAGE_REMOVED,
    EVENT_USER_MESSAGE,
};

/// 造一个临时会话目录 + 一条 `user.message`（承载节点 `u-1`，正是要被改的那条）。
///
/// 返回 `(目录, 那条用户格的 seq)`——seq 就是编辑事实的溯源锚。
fn seeded_dir(tag: &str) -> (std::path::PathBuf, u64) {
    let dir = std::env::temp_dir().join(format!("symbio-v2msg-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时会话目录");
    let store = EventWalStore::open(dir.join(crate::plugins::session::paths::V2_WAL_FILE))
        .expect("打开事实源");
    let seq = store
        .append(
            crate::symbio_core::Event::pending(
                "u-1",
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                0,
                "user",
            )
            .with_ts(1_000)
            .with_payload(serde_json::json!({
                "text": "原始问题", "tier": "full", "id": "u-1",
            })),
        )
        .expect("落用户格")
        .value();
    (dir, seq)
}

fn read_facts(dir: &std::path::Path) -> Vec<crate::symbio_core::Event> {
    let store = EventWalStore::open(dir.join(crate::plugins::session::paths::V2_WAL_FILE))
        .expect("重开事实源");
    store
        .range(Seq::new(0))
        .into_iter()
        .filter(|e| e.entity == Entity::Message)
        .collect()
}

/// 一条被改过的用户消息（形状与 `patch_message` 给出的那份同源）。
fn patched_user(id: &str, text: &str) -> ChatMessage {
    ChatMessage {
        id: id.into(),
        role: Some(MessageRole::User),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text(text.into())),
        timestamp: Some(1_500),
        ..Default::default()
    }
}

/// 编辑：落**一条** `message.edited`，载荷是整条消息、溯源指向被改的那条事实。
#[test]
fn an_edit_lands_one_fact_with_the_patched_message() {
    let (dir, user_seq) = seeded_dir("edit");
    edited(&dir, &patched_user("u-1", "改过的问题"));

    let facts = read_facts(&dir);
    assert_eq!(facts.len(), 1, "一次编辑落一条事实");
    let e = &facts[0];
    assert_eq!(e.kind, EVENT_MESSAGE_EDITED);
    assert_eq!((e.entity, e.verb), (Entity::Message, Verb::Asserted));
    assert_eq!(
        e.turn, 0,
        "变更不属于任何一轮（与 memory.* / session.reconciled 同约定）"
    );
    assert_eq!(e.actor, "user", "编辑是用户对列表的操作");
    assert_eq!(
        e.produced_by,
        Some(user_seq),
        "溯源 = **被改的那条事实**，不是「此刻」的号"
    );
    // 载荷是**补丁应用后**的整条（写侧与读侧共用同一条序列化路径）。
    let msg: ChatMessage =
        serde_json::from_value(e.payload["message"].clone()).expect("载荷里是整条消息");
    assert_eq!(msg.id, "u-1");
    assert_eq!(
        msg.content.as_ref().map(MessageContent::to_text).as_deref(),
        Some("改过的问题")
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// 截断：落**一条**事实，载荷是 id 列表（不是 n 条事实）。
#[test]
fn a_removal_lands_one_fact_carrying_every_id() {
    let (dir, user_seq) = seeded_dir("remove");
    removed(&dir, &["u-1".into(), "T1".into(), "T1-text".into()]);

    let facts = read_facts(&dir);
    assert_eq!(facts.len(), 1, "一次截断落一条事实（区间用 ids 表达）");
    let e = &facts[0];
    assert_eq!(e.kind, EVENT_MESSAGE_REMOVED);
    assert_eq!((e.entity, e.verb), (Entity::Message, Verb::Closed));
    assert_eq!(
        e.produced_by,
        Some(user_seq),
        "溯源锚取**第一条被删的**（截断的起点）"
    );
    assert_eq!(
        e.payload["ids"],
        serde_json::json!(["u-1", "T1", "T1-text"])
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// 空列表**不落格**：「什么都没删」不该在事实源上留下痕迹
/// （与 `truncate_messages` 不发任何变更帧同一条判据）。
#[test]
fn an_empty_removal_lands_nothing() {
    let (dir, _) = seeded_dir("empty-remove");
    removed(&dir, &[]);
    assert!(read_facts(&dir).is_empty());
    std::fs::remove_dir_all(&dir).ok();
}

/// 事实源**为空**（v1 时代建的会话 / 从未走过 v2 轮）⇒ 不落格，也不 panic。
///
/// 不落是因为**给不出溯源锚**——而 I2 要求断言类事件必带溯源。硬造一个号会让
/// 「这次编辑源自哪条发言」变成一句假话；宁可不落（缓存此刻仍兜着）。
#[test]
fn a_missing_source_of_truth_lands_nothing_and_does_not_panic() {
    let dir = std::env::temp_dir().join(format!("symbio-v2msg-{}-novel", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时会话目录");

    edited(&dir, &patched_user("u-1", "改"));
    removed(&dir, &["u-1".into()]);

    assert!(read_facts(&dir).is_empty(), "无事实源 ⇒ 无变更事实");
    std::fs::remove_dir_all(&dir).ok();
}

/// 幂等键**逐次递增**：同一条消息改两次落两条事实，不撞 `Duplicate`。
///
/// 撞键的后果是**静默丢一次编辑**（`append` 返回 `Duplicate`，写方只记日志）——
/// 而用户以为自己改了两次。
#[test]
fn repeated_edits_get_distinct_event_ids() {
    let (dir, _) = seeded_dir("twice");
    edited(&dir, &patched_user("u-1", "第一版"));
    edited(&dir, &patched_user("u-1", "第二版"));

    let facts = read_facts(&dir);
    assert_eq!(facts.len(), 2, "两次编辑 = 两条事实");
    assert_ne!(facts[0].event_id, facts[1].event_id, "幂等键不得撞");
    // 两种变更**共用**同一个计数 ⇒ 交错也不会撞。
    removed(&dir, &["u-1".into()]);
    let facts = read_facts(&dir);
    assert_eq!(facts.len(), 3);
    let ids: std::collections::HashSet<&str> = facts.iter().map(|e| e.event_id.as_str()).collect();
    assert_eq!(ids.len(), 3, "三种前缀/序号组合互不相同：{facts:?}");

    std::fs::remove_dir_all(&dir).ok();
}

/// 承载事件**没有 `seq`**（2026-09-30 修复前落下的旧 WAL 行：`seq: null`）⇒
/// 退化到**切片末尾**的 seq，而不是把整条事实丢掉。
///
/// 为什么要有这一条：`provenance` 只在「切片整个为空」时才该给不出锚。若把
/// 「承载事件缺 seq」也当成给不出，后果是**静默丢一次编辑**——而且丢得比
/// 「切片为空」更隐蔽（切片明明是满的，日志却说给不出溯源）。
#[test]
fn a_carrier_without_seq_falls_back_to_the_slice_tail() {
    let dir = std::env::temp_dir().join(format!("symbio-v2msg-{}-noseq", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时会话目录");

    // 手工写一份「旧版本落下的 WAL」：承载 `u-1` 的那条没有 seq，其后一条有。
    let legacy = crate::symbio_core::Event::pending(
        "u-1",
        EVENT_USER_MESSAGE,
        Entity::Turn,
        Verb::Opened,
        0,
        "user",
    )
    .with_ts(1_000)
    .with_payload(serde_json::json!({ "text": "原始问题", "tier": "full", "id": "u-1" }));
    let mut legacy_json = serde_json::to_value(&legacy).expect("序列化旧事件");
    legacy_json["seq"] = serde_json::Value::Null;

    let later = crate::symbio_core::Event::pending(
        "a-1",
        crate::symbio_core::EVENT_ASSISTANT_FINAL,
        Entity::Turn,
        Verb::Closed,
        1,
        "agent:main",
    )
    .with_ts(2_000)
    .with_payload(serde_json::json!({
        "text": "回答", "turn_id": "T1", "text_id": "T1-text",
    }));
    let mut later_json = serde_json::to_value(&later).expect("序列化后续事件");
    later_json["seq"] = serde_json::json!(1);

    std::fs::write(
        dir.join(crate::plugins::session::paths::V2_WAL_FILE),
        format!("{legacy_json}\n{later_json}\n"),
    )
    .expect("写旧 WAL");

    edited(&dir, &patched_user("u-1", "改过的问题"));

    let facts = read_facts(&dir);
    assert_eq!(facts.len(), 1, "承载事件缺 seq 不该把整条编辑事实丢掉");
    assert_eq!(
        facts[0].produced_by,
        Some(1),
        "退化到切片末尾的 seq（仍回答「站在哪份状态上」）"
    );

    std::fs::remove_dir_all(&dir).ok();
}

// ── S2.7-c：写者令牌被占时**寄存**，由持令牌方补落 ─────────────────────────

/// **S2.7-c 的承重判据**：令牌被占时编辑**不丢**——寄存，补落后仍在。
///
/// 这条就是「编辑偶发不生效」的对偶：不寄存时，`append` 拿 `NotTheWriter`、
/// 出声后丢弃，用户以为改了、切源后一看没改，而**没有任何机制会纠正**。
///
/// 复现手法：**持有一个活的 `EventWalStore`**（它就是令牌持有方），此刻调
/// `edited` —— 进程内两条 `open` 抢同一把文件锁，第二把拿不到（`NotTheWriter`）。
/// 这与「一轮正在跑时用户编辑」在生产上是同一个形态。
///
/// 补落**必须传那个持令牌的实例**：令牌是文件锁，`drain_parked` 自己 `open`
/// 出来的第二个实例同样不是写者（那正是本任务修的形态本身）。
#[test]
fn an_edit_while_the_token_is_held_is_parked_not_dropped() {
    let (dir, _user_seq) = seeded_dir("parked");
    // 令牌持有方：它一活着，后来的 `EventWalStore::open` 就拿不到写者令牌。
    let holder = EventWalStore::open(dir.join(crate::plugins::session::paths::V2_WAL_FILE))
        .expect("持令牌打开");

    edited(&dir, &patched_user("u-1", "跑着的时候改的"));

    // 此刻**还没落**（令牌不在编辑这条链上）。
    assert!(
        read_facts(&dir).is_empty(),
        "令牌被占时不得就地落格（就地落会拿到 NotTheWriter、静默丢弃）"
    );

    // 持令牌方走「下一个落格窗口」：补落——**复用它自己那个持令牌的实例**。
    drain_parked(&dir, &holder);

    let facts = read_facts(&dir);
    assert_eq!(facts.len(), 1, "补落后这条编辑必须真的进网格，而不是消失");
    assert_eq!(facts[0].kind, EVENT_MESSAGE_EDITED);
    let msg: ChatMessage =
        serde_json::from_value(facts[0].payload["message"].clone()).expect("载荷里是整条消息");
    assert_eq!(
        msg.content.as_ref().map(MessageContent::to_text).as_deref(),
        Some("跑着的时候改的"),
        "补落的必须是那条被寄存的编辑，逐字不差"
    );

    drop(holder);
    std::fs::remove_dir_all(&dir).ok();
}

/// 补落之后寄存区**清空**——同一份变更不会被落两次。
#[test]
fn draining_the_park_is_idempotent_across_windows() {
    let (dir, _) = seeded_dir("park-drain-once");
    let holder = EventWalStore::open(dir.join(crate::plugins::session::paths::V2_WAL_FILE))
        .expect("持令牌打开");
    edited(&dir, &patched_user("u-1", "一次"));

    // 连续两次补落窗口（生产上「一轮结束后又开一轮」）：第二次必须是空操作。
    drain_parked(&dir, &holder);
    drain_parked(&dir, &holder);

    assert_eq!(read_facts(&dir).len(), 1, "同一份寄存变更只落一次");

    drop(holder);
    std::fs::remove_dir_all(&dir).ok();
}

/// 令牌**没被占**时，谁也不会走寄存这条路（`drain_parked` 是空操作）。
///
/// 防止「顺手调 drain」变成常规路径：它只在寄存区非空时才有事做，
/// 而寄存区只在令牌被占时才被写入。
///
/// 这里没有令牌持有方（`edited` 是就地落格的）⇒ 补落**在读寄存区之前就返回**，
/// 因此传进来的 `store` 是什么状态都无所谓（传一个刚开的即可）。
#[test]
fn draining_an_empty_park_is_a_noop() {
    let (dir, _) = seeded_dir("park-empty");
    edited(&dir, &patched_user("u-1", "直接落的"));
    let before = read_facts(&dir).len();
    let store = EventWalStore::open(dir.join(crate::plugins::session::paths::V2_WAL_FILE))
        .expect("打开事实源");
    drain_parked(&dir, &store);
    assert_eq!(read_facts(&dir).len(), before, "空寄存区补落应什么都不做");
    std::fs::remove_dir_all(&dir).ok();
}

/// 截断（`removed`）同样走寄存——两类变更共用同一条寄存通道。
#[test]
fn a_truncate_while_the_token_is_held_is_parked_not_dropped() {
    let (dir, _user_seq) = seeded_dir("parked-remove");
    let holder = EventWalStore::open(dir.join(crate::plugins::session::paths::V2_WAL_FILE))
        .expect("持令牌打开");

    removed(&dir, &["u-1".into(), "T1".into()]);
    assert!(read_facts(&dir).is_empty(), "令牌被占 ⇒ 寄存");

    drain_parked(&dir, &holder);
    let facts = read_facts(&dir);
    assert_eq!(facts.len(), 1);
    assert_eq!(facts[0].kind, EVENT_MESSAGE_REMOVED);
    assert_eq!(facts[0].payload["ids"], serde_json::json!(["u-1", "T1"]));

    drop(holder);
    std::fs::remove_dir_all(&dir).ok();
}

/// 补落**重新算溯源锚**：锚必须按「落格那一刻」的切片算，而不是寄存那一刻。
///
/// 寄存发生在编辑的链路（切片里可能只有旧事件），补落发生在执行口（切片已经长出
/// 了本轮的开轮事件）。用旧锚会把这条事实指到一个**过期状态**上——I2 要的
/// 「这次变更站在哪份状态上」就成了一句关于过去的话。
///
/// ## 怎么让「补落那一刻」与「寄存那一刻」可区分
///
/// 被改的节点必须是**事实源里没有承载**的那种（这里用一个 v1 遗留的节点 id
/// `ghost`）——那样 `provenance` 会退化到**切片末尾的 seq**。于是：
/// 寄存时切片末尾 = seq 0（seed 的那条），补落时末尾 = seq 1（新落的 u-2）。
/// 断言拿到 1 就证明锚是补落时算的；拿到 0 就说明用了寄存时的旧锚（本用例要
/// 抓的回归）。
#[test]
fn a_parked_change_re_resolves_its_anchor_at_drain_time() {
    let (dir, _first_seq) = seeded_dir("park-anchor");
    let holder = EventWalStore::open(dir.join(crate::plugins::session::paths::V2_WAL_FILE))
        .expect("持令牌打开");

    // 令牌被占 ⇒ 编辑寄存（此刻锚还没算）。
    // `ghost` 无承载 ⇒ 若此刻算锚会是切片末尾 = 0。
    edited(&dir, &patched_user("ghost", "改"));
    assert!(read_facts(&dir).is_empty());

    // 令牌持有方又落了本轮的开轮事件（切片长长了一条：末尾从 0 变 1）——这是生产上
    // 常态：寄存的编辑在**同一轮**的落格窗口被补落，而那时切片已经含本轮事件。
    // 落格必须走**持令牌的那个实例**（另开一个拿不到锁）。
    let new_seq = holder
        .append(
            crate::symbio_core::Event::pending(
                "u-2",
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                1,
                "user",
            )
            .with_ts(2_000)
            .with_payload(serde_json::json!({ "text": "第二轮", "id": "u-2" })),
        )
        .expect("落第二轮用户格")
        .value();

    drain_parked(&dir, &holder);

    let facts = read_facts(&dir);
    let edit = facts
        .iter()
        .find(|e| e.kind == EVENT_MESSAGE_EDITED)
        .expect("编辑事实必须在");
    assert_eq!(
        edit.produced_by,
        Some(new_seq),
        "锚按**补落时刻**的切片算（无承载节点退化到切片末尾 = 新落的 u-2），\
         而不是寄存那一刻的末尾（0）"
    );

    drop(holder);
    std::fs::remove_dir_all(&dir).ok();
}
