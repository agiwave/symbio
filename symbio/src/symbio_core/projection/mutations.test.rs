//! 变更索引的**收集**判据（S2.7-b1，[ADR-053](../../../../docs/decisions/session.md)）。
//!
//! 折叠的**语义**（编辑换正文 / 删除丢条目）由 `transcript.test.rs` 从投影那一侧钉；
//! 本文件只钉「收」这一半：载荷怎么读、坏载荷怎么办、同一节点被改多次谁说话。
//! 两半分开是因为它们是两种失效：**收错**（索引里没有那条）与**用错**（索引对了但
//! 投影没查）——合成一个文件时，第二种会被第一种的症状盖住。

use super::MessageMutations;
use crate::symbio_core::event::{Entity, Event, Verb, EVENT_MESSAGE_EDITED, EVENT_MESSAGE_REMOVED};

/// 一条 `message.edited`（载荷 = 补丁应用后的整条消息）。
fn edited(node: &str, role: &str, text: &str) -> Event {
    Event::pending(
        format!("m-edit-{node}"),
        EVENT_MESSAGE_EDITED,
        Entity::Message,
        Verb::Asserted,
        0,
        "user",
    )
    .with_payload(serde_json::json!({
        "message": { "id": node, "role": role, "content": text }
    }))
}

/// 一条 `message.removed`（载荷 = 被删 id 列表）。
fn removed(ids: &[&str]) -> Event {
    Event::pending(
        "m-del-0",
        EVENT_MESSAGE_REMOVED,
        Entity::Message,
        Verb::Closed,
        0,
        "user",
    )
    .with_payload(serde_json::json!({ "ids": ids }))
}

/// 收得进来：编辑给出新读法，删除给出 id 集合。
#[test]
fn collect_reads_both_mutation_payloads() {
    let m = MessageMutations::collect(&[edited("u-0", "user", "改过的"), removed(&["f-0", "f-1"])]);

    let ed = m.edited("u-0").expect("编辑必须收得进来");
    assert_eq!(ed.role.as_deref(), Some("user"));
    assert_eq!(ed.text.as_deref(), Some("改过的"));
    assert!(m.edited("u-1").is_none(), "没编辑过的节点不该有读法");
    assert!(m.is_removed("f-0") && m.is_removed("f-1"), "两条 id 都要收");
    assert!(!m.is_removed("f-2"), "没删的不该被判死");
}

/// **最后一条说了算**：同一条消息被编辑两次，读法是后一次。
///
/// 这与 `patch_message` 的语义一致（每次编辑都写一份新的整条），也与重建一致
/// （重建按事件顺序逐条覆盖）。三者不一致时，「prompt 里是哪一版」就成了
/// 取决于谁先跑的问题——而那没有任何判据能看见。
#[test]
fn the_last_edit_wins() {
    let m = MessageMutations::collect(&[
        edited("u-0", "user", "第一版"),
        edited("u-0", "user", "第二版"),
    ]);
    assert_eq!(
        m.edited("u-0").and_then(|e| e.text.clone()).as_deref(),
        Some("第二版")
    );
}

/// 坏载荷**跳过**，不炸、也不误收：读侧必须对任意历史事件容错。
///
/// 三种坏法各一条：没有 `message` 键、`message` 不是对象、`id` 是空串。
/// 它们都由「手工改过的 WAL / 未来版本写的载荷」产生，而不是生产路径。
#[test]
fn malformed_payloads_are_skipped_not_fatal() {
    let no_key = Event::pending(
        "m-edit-x",
        EVENT_MESSAGE_EDITED,
        Entity::Message,
        Verb::Asserted,
        0,
        "user",
    )
    .with_payload(serde_json::json!({ "nope": 1 }));
    let not_object = Event::pending(
        "m-edit-y",
        EVENT_MESSAGE_EDITED,
        Entity::Message,
        Verb::Asserted,
        0,
        "user",
    )
    .with_payload(serde_json::json!({ "message": "我不是一条消息" }));
    let empty_id = Event::pending(
        "m-edit-z",
        EVENT_MESSAGE_EDITED,
        Entity::Message,
        Verb::Asserted,
        0,
        "user",
    )
    .with_payload(serde_json::json!({ "message": { "id": "", "role": "user" } }));

    let m = MessageMutations::collect(&[no_key, not_object, empty_id]);
    assert!(m.edited("").is_none(), "空 id 不是一条可定位的消息");
    assert!(m.edited("m-edit-x").is_none(), "缺 message 键 ⇒ 不收");
}

/// 非变更格的事件一律不收（索引只认两个格子的坐标）。
#[test]
fn only_mutation_cells_are_collected() {
    let lookalike = Event::pending(
        "u-0",
        crate::symbio_core::EVENT_USER_MESSAGE,
        Entity::Turn,
        Verb::Opened,
        0,
        "user",
    )
    .with_payload(
        serde_json::json!({ "message": { "id": "u-0", "role": "user", "content": "x" } }),
    );
    let m = MessageMutations::collect(&[lookalike]);
    assert!(
        m.edited("u-0").is_none(),
        "坐标不对 ⇒ 不收（载荷长得像不算）"
    );
}
