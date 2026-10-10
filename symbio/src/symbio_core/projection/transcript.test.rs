//! 转写的**折叠**判据（S2.7-b1，[ADR-053](../../../../docs/decisions/session.md)）。
//!
//! 与「四格 → 三角色」的基本口径分开成文件：那些是**追加**，折叠是**改写**。
//! 混在一个文件里，后来的人会以为折叠只是又一种投影臂——而它补的是一个**正确性**
//! 缺口（少了它，一次截断在 prompt 面上等于没发生过）。

use super::transcript;
use crate::symbio_core::event::{
    Entity, Event, Verb, EVENT_ASSISTANT_FINAL, EVENT_MESSAGE_EDITED, EVENT_MESSAGE_REMOVED,
    EVENT_USER_MESSAGE,
};
use crate::symbio_core::Budget;

/// 投影成 `(role, text)` 序列（折叠关心的就是这两格）。
fn entries(events: &[Event]) -> Vec<(String, String)> {
    transcript()
        .apply(events, i64::MAX, Budget::generous())
        .value
        .entries
        .into_iter()
        .map(|e| (e.role, e.text))
        .collect()
}

fn user(turn: u64, node: &str, text: &str) -> Event {
    Event::pending(
        format!("u-{turn}"),
        EVENT_USER_MESSAGE,
        Entity::Turn,
        Verb::Opened,
        turn,
        "user",
    )
    .with_payload(serde_json::json!({ "text": text, "tier": "deep", "id": node }))
}

/// 一条收束格。`text_id` 就是它在读侧的**节点身份**（与写侧 `produced_payload` 同源）。
fn assistant(turn: u64, text_id: &str, text: &str) -> Event {
    Event::pending(
        format!("f-{turn}"),
        EVENT_ASSISTANT_FINAL,
        Entity::Turn,
        Verb::Closed,
        turn,
        "agent:main",
    )
    .with_payload(serde_json::json!({
        "text": text,
        "model": "deep",
        "turn_id": format!("t-{turn}"),
        "text_id": text_id,
    }))
}

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

/// 编辑：**换正文，不动位置**。
///
/// 「不动位置」是判据的一半：转写按事件顺序出，编辑若把条目挪到末尾，多轮 prompt
/// 的对话次序就乱了——而次序错乱会让模型把后说的话当成先说的。
#[test]
fn an_edit_replaces_the_text_in_place() {
    let events = vec![
        user(0, "u-0", "原始问题"),
        assistant(0, "f-0-text", "原始回答"),
        edited("u-0", "user", "改过的问题"),
    ];
    assert_eq!(
        entries(&events),
        vec![
            ("user".to_string(), "改过的问题".to_string()),
            ("assistant".to_string(), "原始回答".to_string()),
        ],
        "编辑只换正文、不动位置"
    );
}

/// 编辑助手那条同理（节点身份走 `text_id`，与重建同一套 id）。
#[test]
fn an_edit_reaches_the_assistant_entry() {
    let events = vec![
        user(0, "u-0", "问"),
        assistant(0, "f-0-text", "原始回答"),
        edited("f-0-text", "assistant", "改过的回答"),
    ];
    assert_eq!(
        entries(&events),
        vec![
            ("user".to_string(), "问".to_string()),
            ("assistant".to_string(), "改过的回答".to_string()),
        ]
    );
}

/// 删除：条目**没了**（截断的读侧后果）。
///
/// 这是本批最重要的一条：少了折叠，模型下一轮仍看得见被删掉的那几句——
/// 而用户以为自己删掉了。
#[test]
fn a_removal_drops_the_entry() {
    let events = vec![
        user(0, "u-0", "第一问"),
        assistant(0, "f-0-text", "第一答"),
        user(1, "u-1", "第二问"),
        assistant(1, "f-1-text", "第二答"),
        // 从第二问起截断（起点 + 其后全部）
        removed(&["u-1", "t-1", "f-1-text"]),
    ];
    assert_eq!(
        entries(&events),
        vec![
            ("user".to_string(), "第一问".to_string()),
            ("assistant".to_string(), "第一答".to_string()),
        ],
        "被截断的那一轮不得留在转写里"
    );
}

/// 折叠**不依赖事件顺序**：变更格排在它改的那条之前也照折。
///
/// 生产里变更格恒在后面（先有事实才有「它变了」），但切片可能被裁过 / 手工改过。
/// 依赖顺序的实现在那些情形下会静默漏折——而「静默」正是本条要消灭的形态。
#[test]
fn folding_does_not_depend_on_event_order() {
    let events = vec![
        removed(&["f-0-text"]),
        edited("u-0", "user", "改过的问题"),
        user(0, "u-0", "原始问题"),
        assistant(0, "f-0-text", "原始回答"),
    ];
    assert_eq!(
        entries(&events),
        vec![("user".to_string(), "改过的问题".to_string())]
    );
}

/// 平凡值：没有变更格 ⇒ 逐字等于旧行为（本批的零行为变更承诺）。
#[test]
fn without_mutations_nothing_changes() {
    let events = vec![user(0, "u-0", "问"), assistant(0, "f-0-text", "答")];
    assert_eq!(
        entries(&events),
        vec![
            ("user".to_string(), "问".to_string()),
            ("assistant".to_string(), "答".to_string()),
        ]
    );
}

/// 定位不上的编辑是**空操作**，不是「多出一条」。
///
/// 载荷里的 id 与任何条目都对不上（v1 时代的历史消息 / 切片被裁过）⇒ 什么也不做。
/// 「硬插一条」会让一段历史在错误的时刻冒出一句话。
#[test]
fn an_edit_for_an_unknown_node_is_a_no_op() {
    let events = vec![user(0, "u-0", "问"), edited("ghost", "user", "谁都不是")];
    assert_eq!(
        entries(&events),
        vec![("user".to_string(), "问".to_string())]
    );
}
