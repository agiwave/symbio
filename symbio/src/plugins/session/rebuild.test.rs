//! `rebuild` 的判据（轨 S2.7 第一步）。
//!
//! 这个模块的失效形态是**静默的形状漂移**：重建出的树与落库树长得像但不一样
//! （父节点接错、`msg_type` 差一档、工具结果挂到 Turn 根而不是 ToolCall 上）。
//! 它不报错、不告警，前端只是把工具卡片渲染成一条普通正文。所以这里逐条钉
//! **结构**而不是内容：
//!
//! 1. 每条事实落到**该落的位置**（id / parent_id / type / role 四件套）；
//! 2. 同一批调用在两个载荷里各记一次时**不重复建卡**；
//! 3. 同输入 ⇒ 逐字同输出（纯函数，N1）；
//! 4. 载荷没记身份的格子**合成 id 必须稳定**（否则每次重开都多一批新节点）。

use super::rebuild;
use crate::symbio_core::chat_message::{MessageContent, MessageRole, MessageStatus, MessageType};
use crate::symbio_core::{Entity, Event, Verb};
use crate::symbio_core::{
    EVENT_ARTIFACT_ADDED, EVENT_ASSISTANT_FINAL, EVENT_TURN_SUPPLEMENTED, EVENT_USER_MESSAGE,
};

/// 一条事件的**可断言摘要**——`ChatMessage` 没有 `PartialEq`（它是增量帧的形状，
/// 逐字段比较才有意义），所以判据比的是这四件套 + 正文 + 状态。
#[derive(Debug, PartialEq)]
struct Shape {
    id: String,
    parent: Option<String>,
    ty: Option<MessageType>,
    role: Option<MessageRole>,
    text: String,
    status: Option<MessageStatus>,
}

fn shape(m: &crate::symbio_core::chat_message::ChatMessage) -> Shape {
    Shape {
        id: m.id.clone(),
        parent: m.parent_id.clone(),
        ty: m.msg_type.clone(),
        role: m.role.clone(),
        text: m.content.as_ref().map(|c| c.to_text()).unwrap_or_default(),
        status: m.status.clone(),
    }
}

fn shapes(msgs: &[crate::symbio_core::chat_message::ChatMessage]) -> Vec<Shape> {
    msgs.iter().map(shape).collect()
}

/// 用户格（载荷带节点 id）。
fn user_event(id: &str, text: &str) -> Event {
    Event::pending(
        "u-1",
        EVENT_USER_MESSAGE,
        Entity::Turn,
        Verb::Opened,
        1,
        "user",
    )
    .with_ts(1_000)
    .with_payload(serde_json::json!({ "text": text, "tier": "full", "id": id }))
}

/// 收束格（产出形状见 `turn_runner::produced_payload`）。
fn final_event(payload: serde_json::Value) -> Event {
    Event::pending(
        "f-1",
        EVENT_ASSISTANT_FINAL,
        Entity::Turn,
        Verb::Closed,
        1,
        "agent:main",
    )
    .with_ts(2_000)
    .with_payload(payload)
}

// ── ① 一轮对话的树 ────────────────────────────────────────────────────

#[test]
fn final_builds_turn_root_with_reasoning_and_text_children() {
    let events = vec![
        user_event("u-node", "问 A"),
        final_event(serde_json::json!({
            "text": "答 A",
            "model": "m",
            "turn_id": "T1",
            "text_id": "T1-text",
            "reasoning_id": "T1-reason",
            "reasoning": "想了一下",
            "tool_calls": [],
            "rounds": [],
            "status": "completed",
        })),
    ];

    let got = shapes(&rebuild(&events));
    assert_eq!(
        got,
        vec![
            Shape {
                id: "u-node".into(),
                parent: None,
                ty: Some(MessageType::Text),
                role: Some(MessageRole::User),
                text: "问 A".into(),
                status: Some(MessageStatus::Completed),
            },
            Shape {
                id: "T1".into(),
                parent: None,
                ty: Some(MessageType::Turn),
                role: Some(MessageRole::Assistant),
                text: String::new(),
                status: Some(MessageStatus::Completed),
            },
            Shape {
                id: "T1-reason".into(),
                parent: Some("T1".into()),
                ty: Some(MessageType::Reasoning),
                role: Some(MessageRole::Assistant),
                text: "想了一下".into(),
                status: Some(MessageStatus::Completed),
            },
            Shape {
                id: "T1-text".into(),
                parent: Some("T1".into()),
                ty: Some(MessageType::Text),
                role: Some(MessageRole::Assistant),
                text: "答 A".into(),
                status: Some(MessageStatus::Completed),
            },
        ]
    );
}

/// reasoning-only：思考与正文同一段内容且无独立文本回复 ⇒ 只出一个 Text 子节点，
/// **不另立 Reasoning 子**（与落库侧 `llm_build_assistant_messages` 同一判据）。
#[test]
fn reasoning_only_does_not_duplicate_the_paragraph() {
    let events = vec![
        user_event("u-node", "问"),
        final_event(serde_json::json!({
            "text": "同一段",
            "turn_id": "T1",
            "text_id": "T1-text",
            "reasoning_id": "T1-reason",
            "reasoning": "同一段",
            "status": "completed",
        })),
    ];
    let got = shapes(&rebuild(&events));
    assert_eq!(got.len(), 3, "user + Turn + 一条 Text，实得 {got:?}");
    assert_eq!(got[2].id, "T1-reason", "reasoning-only 时复用推理子节点 id");
    assert_eq!(got[2].ty, Some(MessageType::Text));
}

// ── ② 工具卡：调用与结果是两张卡，结果挂在调用下 ────────────────────────

#[test]
fn artifact_builds_call_and_result_under_the_call() {
    let events = vec![
        user_event("u-node", "读文件"),
        Event::pending(
            "a-1-0",
            EVENT_ARTIFACT_ADDED,
            Entity::Artifact,
            Verb::Asserted,
            1,
            "agent:main",
        )
        .with_ts(1_500)
        .with_payload(serde_json::json!({
            "tool": "read",
            "text": "文件内容",
            "tool_call_id": "C1",
            "wire_id": "call_wire_1",
            "args": "{\"path\":\"a.md\"}",
            "result_id": "C1-res",
            "success": true,
            "turn_id": "T1",
        })),
    ];

    let got = shapes(&rebuild(&events));
    assert_eq!(got.len(), 3);
    // 调用卡：ToolCall，挂在 Turn 根下，正文是**已定稿的参数文本**
    assert_eq!(got[1].id, "C1");
    assert_eq!(got[1].parent, Some("T1".into()));
    assert_eq!(got[1].ty, Some(MessageType::ToolCall));
    assert_eq!(got[1].role, Some(MessageRole::Assistant));
    assert_eq!(got[1].text, "{\"path\":\"a.md\"}");
    // 结果卡：role=Tool，挂在**调用**下（不是 Turn 根）
    assert_eq!(got[2].id, "C1-res");
    assert_eq!(got[2].parent, Some("C1".into()));
    assert_eq!(got[2].role, Some(MessageRole::Tool));
    assert_eq!(got[2].text, "文件内容");
    assert_eq!(got[2].status, Some(MessageStatus::Completed));
}

/// 收束载荷的 `tool_calls[]` 是**引用**（结果正文归 `artifact.added`）——
/// 同 id 不得重复建卡，否则前端会看到两张一样的工具卡。
#[test]
fn tool_calls_reference_does_not_duplicate_the_card() {
    let events = vec![
        Event::pending(
            "a-1-0",
            EVENT_ARTIFACT_ADDED,
            Entity::Artifact,
            Verb::Asserted,
            1,
            "agent:main",
        )
        .with_payload(serde_json::json!({
            "tool": "read", "text": "内容", "tool_call_id": "C1", "wire_id": "w1",
            "args": "{}", "result_id": "C1-res", "success": true, "turn_id": "T1",
        })),
        final_event(serde_json::json!({
            "text": "读完了",
            "turn_id": "T1",
            "text_id": "T1-text",
            "tool_calls": [{ "id": "C1", "name": "read", "args": "{}", "wire_id": "w1", "result_id": "C1-res" }],
            "status": "completed",
        })),
    ];
    let got = shapes(&rebuild(&events));
    let calls = got.iter().filter(|s| s.id == "C1").count();
    assert_eq!(calls, 1, "同一张工具卡只能有一份，实得 {calls}");
}

// ── ③ 纯函数：同输入 ⇒ 同输出 ─────────────────────────────────────────

#[test]
fn rebuild_is_deterministic() {
    let events = vec![
        user_event("u-node", "问"),
        final_event(serde_json::json!({
            "text": "答", "turn_id": "T1", "text_id": "t", "status": "completed",
        })),
    ];
    assert_eq!(shapes(&rebuild(&events)), shapes(&rebuild(&events)));
}

// ── ④ 载荷没记身份 ⇒ 合成 id 必须稳定 ─────────────────────────────────

#[test]
fn supplement_gets_a_stable_synthetic_id() {
    let ev = Event::pending(
        "s-1-0",
        EVENT_TURN_SUPPLEMENTED,
        Entity::Turn,
        Verb::Asserted,
        1,
        "user",
    )
    .with_ts(1_200)
    .with_payload(serde_json::json!({ "text": "插一句", "count": 1 }));

    let a = shapes(&rebuild(std::slice::from_ref(&ev)));
    let b = shapes(&rebuild(&[ev]));
    assert_eq!(a, b, "同一条事件每次重建必须得同一个 id");
    assert_eq!(a[0].id, "s-1-0", "合成 id 取事件 id");
    assert_eq!(a[0].role, Some(MessageRole::User));
    assert_eq!(a[0].text, "插一句");
}

// ── ⑤ 状态是产出事实的状态，落到根与正文上 ────────────────────────────

#[test]
fn aborted_status_lands_on_the_turn_and_its_text() {
    let events = vec![
        user_event("u-node", "问"),
        final_event(serde_json::json!({
            "text": "说到一半", "turn_id": "T1", "text_id": "t",
            "status": "aborted",
        })),
    ];
    let got = shapes(&rebuild(&events));
    assert_eq!(got[1].status, Some(MessageStatus::Aborted));
    assert_eq!(got[2].status, Some(MessageStatus::Aborted));
    assert_eq!(got[2].text, "说到一半", "半截正文照入格（ADR-052）");
}

/// 正文用 `MessageContent::Text` 承载（不是 `Parts`）——重建给的是纯文本形态，
/// 与落库侧一致。
#[test]
fn text_content_is_plain_text() {
    let events = vec![user_event("u-node", "问")];
    let msgs = rebuild(&events);
    assert!(matches!(msgs[0].content, Some(MessageContent::Text(_))));
}
