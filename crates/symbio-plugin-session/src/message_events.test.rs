//! 消息变更**写方**的判据（S2.7-b1，D-053）。
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
//! （`docs/104-module-layout.md` §2）。本文件只钉「事实本身」。

use super::{drain_parked, edited, removed, AppendOutcome};
use crate::store::EventWalStore;
use symbio_core::chat_node::{ChatNode, MessageContent, NodeKind};
use symbio_core::{
    Entity, Seq, Store, Verb, EVENT_MESSAGE_EDITED, EVENT_MESSAGE_REMOVED, EVENT_USER_MESSAGE,
};

/// 造一个临时会话目录 + 一条 `user.message`（承载节点 `u-1`，正是要被改的那条）。
///
/// 返回 `(目录, 那条用户格的 seq)`——seq 就是编辑事实的溯源锚。
fn seeded_dir(tag: &str) -> (std::path::PathBuf, u64) {
    let dir = std::env::temp_dir().join(format!("symbio-v2msg-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("临时会话目录");
    let store = EventWalStore::open(dir.join(crate::paths::V2_WAL_FILE))
        .expect("打开事实源");
    let seq = store
        .append(
            symbio_core::Event::pending(
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

/// 会话 id：只用于落格时的告警署名（寄存区是**每个会话目录一份**，不再按 id 分桶）。
/// 各用例的目录名本来就唯一，直接取末段。
fn sid_of(dir: &std::path::Path) -> String {
    dir.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// 打开事实源的一个实例：没有别的持有者时它就是写者；已有持有者时降级成只读
/// 实例（`append` 恒 `NotTheWriter`）——后者正是「令牌不在本进程」的形态。
fn open_wal(dir: &std::path::Path) -> EventWalStore {
    EventWalStore::open(dir.join(crate::paths::V2_WAL_FILE)).expect("打开事实源")
}

/// 断言一次变更**落了格**（不是寄存、不是豁免、更不是失败）。
fn assert_landed(outcome: AppendOutcome, what: &str) {
    assert!(
        matches!(outcome, AppendOutcome::Landed),
        "{what} 应当落格，实际：{outcome:?}"
    );
}

fn read_facts(dir: &std::path::Path) -> Vec<symbio_core::Event> {
    let store = EventWalStore::open(dir.join(crate::paths::V2_WAL_FILE))
        .expect("重开事实源");
    store
        .range(Seq::new(0))
        .into_iter()
        .filter(|e| e.entity == Entity::Message)
        .collect()
}

/// 一条被改过的用户消息（形状与 `patch_message` 给出的那份同源）。
fn patched_user(id: &str, text: &str) -> ChatNode {
    ChatNode {
        id: id.into(),
        principal: Some(symbio_core::principal::Principal::from_legacy(
            symbio_core::PRINCIPAL_USER,
        )),
        msg_type: Some(NodeKind::Body),
        content: Some(MessageContent::Text(text.into())),
        timestamp: Some(1_500),
        ..Default::default()
    }
}

/// 编辑：落**一条** `message.edited`，载荷是整条消息、溯源指向被改的那条事实。
#[test]
fn an_edit_lands_one_fact_with_the_patched_message() {
    let (dir, user_seq) = seeded_dir("edit");
    assert_landed(
        edited(
            &open_wal(&dir),
            &sid_of(&dir),
            &patched_user("u-1", "改过的问题"),
        ),
        "编辑",
    );

    let facts = read_facts(&dir);
    assert_eq!(facts.len(), 1, "一次编辑落一条事实");
    let e = &facts[0];
    assert_eq!(e.kind, EVENT_MESSAGE_EDITED);
    assert_eq!((e.entity, e.verb), (Entity::Message, Verb::Asserted));
    assert_eq!(e.turn, 0, "变更不属于任何一轮（与 memory.* 同约定）");
    assert_eq!(e.actor, "person/local", "编辑是用户对列表的操作");
    assert_eq!(
        e.produced_by,
        Some(user_seq),
        "溯源 = **被改的那条事实**，不是「此刻」的号"
    );
    // 载荷是**补丁应用后**的整条（写侧与读侧共用同一条序列化路径）。
    let msg: ChatNode =
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
    assert_landed(
        removed(
            &open_wal(&dir),
            &sid_of(&dir),
            &["u-1".into(), "T1".into(), "T1-text".into()],
        ),
        "截断",
    );

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
    assert!(
        matches!(
            removed(&open_wal(&dir), &sid_of(&dir), &[]),
            AppendOutcome::Nothing
        ),
        "空列表没有事实可落——既不是失败，也不该被当成落格"
    );
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

    // 无锚是**豁免**而不是失败：这类会话的读侧回落 `messages.json`（D-049）。
    assert!(
        matches!(
            edited(&open_wal(&dir), &sid_of(&dir), &patched_user("u-1", "改")),
            AppendOutcome::NoAnchor
        ),
        "v1 遗留会话没有溯源锚 ⇒ 豁免，而不是报错"
    );
    assert!(matches!(
        removed(&open_wal(&dir), &sid_of(&dir), &["u-1".into()]),
        AppendOutcome::NoAnchor
    ));

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
    assert_landed(
        edited(
            &open_wal(&dir),
            &sid_of(&dir),
            &patched_user("u-1", "第一版"),
        ),
        "第一次编辑",
    );
    assert_landed(
        edited(
            &open_wal(&dir),
            &sid_of(&dir),
            &patched_user("u-1", "第二版"),
        ),
        "第二次编辑",
    );

    let facts = read_facts(&dir);
    assert_eq!(facts.len(), 2, "两次编辑 = 两条事实");
    assert_ne!(facts[0].event_id, facts[1].event_id, "幂等键不得撞");
    // 两种变更**共用**同一个计数 ⇒ 交错也不会撞。
    assert_landed(
        removed(&open_wal(&dir), &sid_of(&dir), &["u-1".into()]),
        "截断",
    );
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
    let legacy = symbio_core::Event::pending(
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

    let later = symbio_core::Event::pending(
        "a-1",
        symbio_core::EVENT_ASSISTANT_FINAL,
        Entity::Turn,
        Verb::Closed,
        1,
        "session/main",
    )
    .with_ts(2_000)
    .with_payload(serde_json::json!({
        "text": "回答", "turn_id": "T1", "text_id": "T1-text",
    }));
    let mut later_json = serde_json::to_value(&later).expect("序列化后续事件");
    later_json["seq"] = serde_json::json!(1);

    std::fs::write(
        dir.join(crate::paths::V2_WAL_FILE),
        format!("{legacy_json}\n{later_json}\n"),
    )
    .expect("写旧 WAL");

    assert_landed(
        edited(
            &open_wal(&dir),
            &sid_of(&dir),
            &patched_user("u-1", "改过的问题"),
        ),
        "承载事件缺 seq 的编辑",
    );

    let facts = read_facts(&dir);
    assert_eq!(facts.len(), 1, "承载事件缺 seq 不该把整条编辑事实丢掉");
    assert_eq!(
        facts[0].produced_by,
        Some(1),
        "退化到切片末尾的 seq（仍回答「站在哪份状态上」）"
    );

    std::fs::remove_dir_all(&dir).ok();
}

// ── ：写者令牌被占时**寄存**，由持令牌方补落 ─────────────────────────

/// ** 的承重判据**：令牌被占时编辑**不丢**——寄存，补落后仍在。
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
    let holder = EventWalStore::open(dir.join(crate::paths::V2_WAL_FILE))
        .expect("持令牌打开");

    assert!(
        matches!(
            edited(
                &open_wal(&dir),
                &sid_of(&dir),
                &patched_user("u-1", "跑着的时候改的")
            ),
            AppendOutcome::Parked
        ),
        "令牌被占 ⇒ 寄存（不是失败、也不是静默丢弃）"
    );

    // 此刻**还没落**（令牌不在编辑这条链上）。
    assert!(
        read_facts(&dir).is_empty(),
        "令牌被占时不得就地落格（就地落会拿到 NotTheWriter、静默丢弃）"
    );

    // 持令牌方走「下一个落格窗口」：补落——**复用它自己那个持令牌的实例**。
    drain_parked(&holder, &sid_of(&dir));

    let facts = read_facts(&dir);
    assert_eq!(facts.len(), 1, "补落后这条编辑必须真的进网格，而不是消失");
    assert_eq!(facts[0].kind, EVENT_MESSAGE_EDITED);
    let msg: ChatNode =
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
    let holder = EventWalStore::open(dir.join(crate::paths::V2_WAL_FILE))
        .expect("持令牌打开");
    assert!(matches!(
        edited(&open_wal(&dir), &sid_of(&dir), &patched_user("u-1", "一次")),
        AppendOutcome::Parked
    ));

    // 连续两次补落窗口（生产上「一轮结束后又开一轮」）：第二次必须是空操作。
    drain_parked(&holder, &sid_of(&dir));
    drain_parked(&holder, &sid_of(&dir));

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
    assert_landed(
        edited(
            &open_wal(&dir),
            &sid_of(&dir),
            &patched_user("u-1", "直接落的"),
        ),
        "就地落格",
    );
    let before = read_facts(&dir).len();
    let store = EventWalStore::open(dir.join(crate::paths::V2_WAL_FILE))
        .expect("打开事实源");
    drain_parked(&store, &sid_of(&dir));
    assert_eq!(read_facts(&dir).len(), before, "空寄存区补落应什么都不做");
    std::fs::remove_dir_all(&dir).ok();
}

/// 截断（`removed`）同样走寄存——两类变更共用同一条寄存通道。
#[test]
fn a_truncate_while_the_token_is_held_is_parked_not_dropped() {
    let (dir, _user_seq) = seeded_dir("parked-remove");
    let holder = EventWalStore::open(dir.join(crate::paths::V2_WAL_FILE))
        .expect("持令牌打开");

    assert!(matches!(
        removed(&open_wal(&dir), &sid_of(&dir), &["u-1".into(), "T1".into()]),
        AppendOutcome::Parked
    ));
    assert!(read_facts(&dir).is_empty(), "令牌被占 ⇒ 寄存");

    drain_parked(&holder, &sid_of(&dir));
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
    let holder = EventWalStore::open(dir.join(crate::paths::V2_WAL_FILE))
        .expect("持令牌打开");

    // 令牌被占 ⇒ 编辑寄存（此刻锚还没算）。
    // `ghost` 无承载 ⇒ 若此刻算锚会是切片末尾 = 0。
    assert!(matches!(
        edited(&open_wal(&dir), &sid_of(&dir), &patched_user("ghost", "改")),
        AppendOutcome::Parked
    ));
    assert!(read_facts(&dir).is_empty());

    // 令牌持有方又落了本轮的开轮事件（切片长长了一条：末尾从 0 变 1）——这是生产上
    // 常态：寄存的编辑在**同一轮**的落格窗口被补落，而那时切片已经含本轮事件。
    // 落格必须走**持令牌的那个实例**（另开一个拿不到锁）。
    let new_seq = holder
        .append(
            symbio_core::Event::pending(
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

    drain_parked(&holder, &sid_of(&dir));

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

// ── 项三：寄存区在**盘上**（跨进程兜底才成立）──────────────────────────

/// 寄存区里**已寄存**的那几件（`.json`），按名升序 = 寄存先后。
///
/// 复用生产侧同一套可见性判据（[`super::is_parked`] / [`super::pending_listing`]）：
/// 「什么算一件寄存」只有一处答案，用例不许自己再猜一遍扩展名。
fn parked_entries(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    super::pending_listing(&super::pending_dir(dir))
        .into_iter()
        .filter(|p| super::is_parked(p))
        .collect()
}

/// **盘上寄存的承重判据**：寄存者那个进程**不在了**，变更仍然落得下去。
///
/// 这正是把寄存区从进程内静态表搬到文件系统的唯一理由：令牌被**另一个进程**持着时，
/// 没令牌的那个进程里的内存寄存，持令牌的进程永远看不见；那个进程一退出（CLI 一条
/// 命令就是短命进程），寄存跟着消失——旧形态不是「崩溃才丢」，是**触发即丢**。
///
/// 这里用「释放令牌后重开一个从未参与过寄存的句柄」复现跨进程：锁随句柄关闭自动释放，
/// 新句柄就是那个持令牌的进程，它与写寄存件的那个实例之间**没有任何共享内存**。
#[test]
fn a_parked_change_is_on_disk_and_lands_for_a_fresh_token_holder() {
    let (dir, _) = seeded_dir("park-on-disk");
    let wal = dir.join(crate::paths::V2_WAL_FILE);
    let holder = EventWalStore::open(&wal).expect("持令牌打开");

    assert!(matches!(
        edited(
            &open_wal(&dir),
            &sid_of(&dir),
            &patched_user("u-1", "换个进程来落")
        ),
        AppendOutcome::Parked
    ));
    // 寄存件在**盘上**，不在哪个进程的内存里（这一步是整条判据的支点）。
    assert_eq!(parked_entries(&dir).len(), 1, "寄存必须看得见");
    assert!(read_facts(&dir).is_empty(), "还没落格");

    // 持令牌的那个「进程」换成了一个新句柄：它没寄存过任何东西，也不认识那次编辑。
    drop(holder);
    let fresh = EventWalStore::open(&wal).expect("重开拿到令牌");
    drain_parked(&fresh, &sid_of(&dir));

    let facts = read_facts(&dir);
    assert_eq!(facts.len(), 1, "寄存者不在了也必须补落得下去");
    let msg: ChatNode =
        serde_json::from_value(facts[0].payload["message"].clone()).expect("载荷里是整条消息");
    assert_eq!(
        msg.content.as_ref().map(MessageContent::to_text).as_deref(),
        Some("换个进程来落")
    );
    assert!(parked_entries(&dir).is_empty(), "落一条删一条");

    std::fs::remove_dir_all(&dir).ok();
}

/// **两件寄存按寄存先后落格**——文件名就是顺序键。
///
/// 折叠顺序一旦错开，「同一条消息编辑两次」就变成倒放（后一次的内容被前一次盖掉），
/// 而且**看不出来**：两条事实都在，末态是错的那个。
#[test]
fn parked_changes_land_in_park_order() {
    let (dir, _) = seeded_dir("park-order");
    let holder = EventWalStore::open(dir.join(crate::paths::V2_WAL_FILE))
        .expect("持令牌打开");
    let second = open_wal(&dir);

    assert!(matches!(
        edited(&second, &sid_of(&dir), &patched_user("u-1", "先寄存的")),
        AppendOutcome::Parked
    ));
    assert!(matches!(
        edited(
            &open_wal(&dir),
            &sid_of(&dir),
            &patched_user("u-1", "后寄存的")
        ),
        AppendOutcome::Parked
    ));
    assert_eq!(parked_entries(&dir).len(), 2, "两件各一份");

    drain_parked(&holder, &sid_of(&dir));

    let texts: Vec<String> = read_facts(&dir)
        .iter()
        .map(|e| {
            let msg: ChatNode = serde_json::from_value(e.payload["message"].clone()).expect("消息");
            msg.content
                .as_ref()
                .map(MessageContent::to_text)
                .unwrap_or_default()
        })
        .collect();
    assert_eq!(
        texts,
        vec!["先寄存的".to_string(), "后寄存的".to_string()],
        "落格顺序 = 寄存顺序（倒放会静默给出错的末态）"
    );

    drop(holder);
    std::fs::remove_dir_all(&dir).ok();
}

/// `.staged` 半件**不在队列里**，且被补落方顺手清掉。
///
/// 一次寄存的可见顺序键只有 `.json`：崩在改名之前留下的是半件，它不是「半条变更」，
/// 而是**那次寄存没发生过**。让它进队列就等于让一次可能只有半个 JSON 的写入去参与
/// 折叠——那才是真正会静默改坏历史的路径。
#[test]
fn a_staged_half_piece_never_lands_and_is_cleaned_up() {
    let (dir, _) = seeded_dir("park-staged");
    let holder = EventWalStore::open(dir.join(crate::paths::V2_WAL_FILE))
        .expect("持令牌打开");
    assert!(matches!(
        edited(
            &open_wal(&dir),
            &sid_of(&dir),
            &patched_user("u-1", "完整的这件")
        ),
        AppendOutcome::Parked
    ));

    // 手工造半件：写一半、不 sync、不改名（正是 `park` 被中断留下的形状）。
    let pending = super::pending_dir(&dir);
    std::fs::write(
        pending.join("9999999999999-0000000-000000.staged"),
        b"{\"kind\":\"message.edited\"",
    )
    .expect("造半件");

    drain_parked(&holder, &sid_of(&dir));

    assert_eq!(read_facts(&dir).len(), 1, "半件不许进事实源");
    assert!(
        !pending.join("9999999999999-0000000-000000.staged").exists(),
        "半件由补落方顺手清掉（否则会永远堆在会话目录里）"
    );

    drop(holder);
    std::fs::remove_dir_all(&dir).ok();
}

/// 寄存区**满界丢最旧**：上界存在且真的生效，多出来的那件写得进、最旧那件让位。
///
/// 为什么必须有界：寄存只在「另一个进程持着令牌」这个窗口里发生，而这个窗口本该
/// 很快结束。堆到上界说明**持令牌的窗口一直没出现**——那是比溢出更严重的缺陷，
/// 所以先丢最旧并告警（`park` 里的 `plugin_warn!`），而不是无声地撑爆磁盘。
#[test]
fn a_full_park_drops_the_oldest_entry() {
    let (dir, _) = seeded_dir("park-cap");
    let holder = EventWalStore::open(dir.join(crate::paths::V2_WAL_FILE))
        .expect("持令牌打开");

    // 预置满寄存器：名字全部**早于**真实时钟读数的寄存件（首位字符 `0`）。
    let pending = super::pending_dir(&dir);
    std::fs::create_dir_all(&pending).expect("寄存区目录");
    for i in 0..super::PARK_CAP {
        std::fs::write(
            pending.join(format!("0000-{i:04}.{}", super::PARKED_SUFFIX)),
            br#"{"kind":"message.edited"}"#,
        )
        .expect("预置寄存件");
    }
    let oldest = pending.join(format!("0000-{:04}.{}", 0, super::PARKED_SUFFIX));
    assert!(oldest.exists());

    assert!(
        matches!(
            edited(
                &open_wal(&dir),
                &sid_of(&dir),
                &patched_user("u-1", "第 N+1 件")
            ),
            AppendOutcome::Parked
        ),
        "满了也照样寄存这一件（丢的是最旧的，不是新的）"
    );

    assert_eq!(
        parked_entries(&dir).len(),
        super::PARK_CAP,
        "有界：寄存件数停在上界，不会一直涨"
    );
    assert!(!oldest.exists(), "让位的是最旧的一件");

    drop(holder);
    std::fs::remove_dir_all(&dir).ok();
}

/// **寄存写不进去 ⇒ 真失败**（无副本不许回成功）。
///
/// 这条判据把盘上寄存与旧的内存寄存区分开：寄存一旦是一次 IO，它就可能失败，
/// 而失败时这条变更**没有任何副本**——此刻回执必须是 `Failed`（调用方在写缓存之前
/// 把它变成可见错误），绝不能是 `Parked`。
/// 「回执说改好了、事实源里没有」正是这一族缺陷的原形，不能由寄存这条路自己再犯一次。
///
/// 手法：把寄存区的**父段**做成普通文件 ⇒ `transcripts/pending` 这条路建不起来。
#[test]
fn a_park_that_cannot_be_written_reports_failure() {
    let (dir, _) = seeded_dir("park-fails");
    let holder = EventWalStore::open(dir.join(crate::paths::V2_WAL_FILE))
        .expect("持令牌打开");
    // 寄存区目录的父段被一个普通文件占了 ⇒ create_dir_all 必失败。
    std::fs::write(
        dir.join(crate::paths::TRANSCRIPTS_SUBDIR),
        b"an ordinary file, not a directory",
    )
    .expect("占位文件");

    let outcome = edited(
        &open_wal(&dir),
        &sid_of(&dir),
        &patched_user("u-1", "落不下去的编辑"),
    );
    assert!(
        matches!(outcome, AppendOutcome::Failed(_)),
        "寄存写不进去必须冒泡成真失败，实际：{outcome:?}"
    );
    assert!(read_facts(&dir).is_empty(), "确实一条都没落");

    drop(holder);
    std::fs::remove_dir_all(&dir).ok();
}

// ── actor / ts 的归属（2026-10-13 从 `d781a6c7` 号会话复查出来）──────────────
//
// 那次会话 363 条 `message.edited` 里 **role=user 的是 0 条**——全是助手流式正文与
// 工具结果，却一条不落地被记成 `actor: "user"`，548 个事件的 `ts` 又全是 0。假的
// 主体 + 假的时刻，任何按主体过滤/计数/排序的投影都得错，且**错得看不出来**。

/// 助手正文的编辑 ⇒ 落 `agent:main`，不是 user。
#[test]
fn an_assistant_edit_is_attributed_to_the_agent_not_the_user() {
    let (dir, _) = seeded_dir("actor-assistant");
    assert_landed(
        edited(
            &open_wal(&dir),
            &sid_of(&dir),
            &patched_msg("u-1", "流式累积的一段", Some("session/main"), 1_500),
        ),
        "助手正文编辑",
    );

    let edit = read_facts(&dir)
        .into_iter()
        .find(|e| e.kind == EVENT_MESSAGE_EDITED)
        .expect("编辑事实");
    assert_eq!(
        edit.actor, "session/main",
        "助手产出署本会话主体——署成 user 是冒用身份"
    );
    assert_eq!(edit.ts, 1_500, "时刻取自消息本身，不是一律 0");

    std::fs::remove_dir_all(&dir).ok();
}

/// 工具结果的编辑 ⇒ 也不是 user（`d781a6c7` 里这类占了 181/363）。
#[test]
fn a_tool_result_edit_is_attributed_to_the_agent_not_the_user() {
    let (dir, _) = seeded_dir("actor-tool");
    assert_landed(
        edited(
            &open_wal(&dir),
            &sid_of(&dir),
            &patched_msg("u-1", "找不到文件", Some("session/main"), 1_700),
        ),
        "工具结果编辑",
    );

    let edit = read_facts(&dir)
        .into_iter()
        .find(|e| e.kind == EVENT_MESSAGE_EDITED)
        .expect("编辑事实");
    assert_eq!(edit.actor, "session/main");
    assert_eq!(edit.ts, 1_700);

    std::fs::remove_dir_all(&dir).ok();
}

/// 只有用户自己的消息才署 `user`——这是唯一该出现 PRINCIPAL_USER 的地方。
#[test]
fn only_a_user_message_is_attributed_to_the_user() {
    let (dir, _) = seeded_dir("actor-user");
    assert_landed(
        edited(
            &open_wal(&dir),
            &sid_of(&dir),
            &patched_msg(
                "u-1",
                "改过的问题",
                Some(symbio_core::PRINCIPAL_USER),
                1_600,
            ),
        ),
        "用户消息编辑",
    );

    let edit = read_facts(&dir)
        .into_iter()
        .find(|e| e.kind == EVENT_MESSAGE_EDITED)
        .expect("编辑事实");
    assert_eq!(edit.actor, "person/local", "用户的话才署用户");
    assert_eq!(edit.ts, 1_600);

    std::fs::remove_dir_all(&dir).ok();
}

/// 消息自带 `principal` 时**认它**——转播进来的别的会话/子智能体不能被本会话覆盖
/// （`plugin::attributed` 的同一条纪律，写侧不许例外）。
#[test]
fn an_explicit_principal_is_not_rewritten_to_the_session() {
    let (dir, _) = seeded_dir("actor-relayed");
    assert_landed(
        edited(
            &open_wal(&dir),
            &sid_of(&dir),
            &patched_msg("u-1", "别人说的话", Some("session/sub-1"), 1_800),
        ),
        "转播消息编辑",
    );

    let edit = read_facts(&dir)
        .into_iter()
        .find(|e| e.kind == EVENT_MESSAGE_EDITED)
        .expect("编辑事实");
    assert_eq!(
        edit.actor, "session/sub-1",
        "自带主体优先于本会话兜底——不许串主体"
    );

    std::fs::remove_dir_all(&dir).ok();
}

/// 造一条待编辑的消息（身份 / 时刻由用例指定）。
/// `principal` 用**遗留字符串**入参，测试内部一次性折成 `Principal` 结构体——
/// 让用例继续用可读的字面量（`"session/sub-1"`）而不必到处写 `Principal::from_legacy`。
fn patched_msg(id: &str, text: &str, principal: Option<&str>, ts: i64) -> ChatNode {
    ChatNode {
        id: id.into(),
        principal: principal.map(symbio_core::principal::Principal::from_legacy),
        msg_type: Some(NodeKind::Body),
        content: Some(MessageContent::Text(text.into())),
        timestamp: Some(ts),
        ..Default::default()
    }
}

/// 消息**自带**的时刻是事实，优先用它。
#[test]
fn the_messages_own_timestamp_wins() {
    let (dir, _) = seeded_dir("ts-given");
    assert_landed(
        edited(
            &open_wal(&dir),
            &sid_of(&dir),
            &patched_msg("u-1", "带时刻", Some("session/main"), 1_234_567),
        ),
        "带时刻的编辑",
    );

    let edit = read_facts(&dir)
        .into_iter()
        .find(|e| e.kind == EVENT_MESSAGE_EDITED)
        .expect("编辑事实");
    assert_eq!(edit.ts, 1_234_567, "消息自带时刻 ⇒ 原样用作事实的时刻");

    std::fs::remove_dir_all(&dir).ok();
}

/// 消息**没带**时刻 ⇒ 退到落格时刻，**不是 0**。
///
/// `d781a6c7` 里 548 个事件的 ts 全为 0：上游 9 处 assistant 构造点只有 1 处填了
/// timestamp，其余全裸。0 不是「未知」而是假话（假装都发生在纪元起点），时序因此
/// 无从谈起。这里守住的是「宁可用落格时刻，也不许写 0」。
#[test]
fn a_missing_timestamp_falls_back_to_now_never_to_zero() {
    let (dir, _) = seeded_dir("ts-missing");
    let mut m = patched_msg("u-1", "没填时刻", None, 0);
    m.timestamp = Some(0); // 上游把「没填」写成 0，一并兜住
    m.principal = None;
    assert_landed(edited(&open_wal(&dir), &sid_of(&dir), &m), "缺时刻的编辑");

    let edit = read_facts(&dir)
        .into_iter()
        .find(|e| e.kind == EVENT_MESSAGE_EDITED)
        .expect("编辑事实");
    assert_ne!(edit.ts, 0, "没带时刻也不许写 0——那是谎报，不是未知");
    assert!(
        edit.ts > 1_700_000_000_000,
        "退到的是真实时钟读数：{ts}",
        ts = edit.ts
    );

    std::fs::remove_dir_all(&dir).ok();
}
