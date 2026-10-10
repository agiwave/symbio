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

use super::{edited, removed};
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
