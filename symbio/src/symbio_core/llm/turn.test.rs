//! `symbio/src/symbio_core/llm/turn.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! **只留契约用例**：工具调用的分片累积过程（10 条）已随 `TurnToolCallAccumulator`
//! 迁到 `plugins/model/tool_accumulator.test.rs`——它是 model 的实现细节。这里测的是
//! `TurnOutput` 这个**产物契约**的形状不变式。

use super::*;

fn out(text: &str, reasoning: &str) -> TurnOutput {
    TurnOutput {
        text: text.to_string(),
        reasoning: reasoning.to_string(),
        ..Default::default()
    }
}

fn tool_call(name: &str) -> TurnToolCallInfo {
    TurnToolCallInfo {
        id: Some("tc-1".to_string()),
        wire_id: None,
        name: Some(name.to_string()),
        arguments: serde_json::json!({ "path": "." }),
        parse_error: None,
    }
}

#[test]
fn is_reasoning_only_requires_empty_text_and_no_tools() {
    // 只有 reasoning、无正文、无工具 → reasoning-only
    assert!(out("", "思考").is_reasoning_only());
    // 空白正文同样视为「无文本回复」
    assert!(out("  \n ", "思考").is_reasoning_only());
    // 有正文 → 非 reasoning-only
    assert!(!out("回复", "思考").is_reasoning_only());
    // 有工具调用 → 非 reasoning-only（reasoning 需保留为独立子节点）
    let mut with_tool = out("", "思考");
    with_tool.tool_calls.push(tool_call("vdfs_list"));
    assert!(!with_tool.is_reasoning_only());
    // 无 reasoning → 非 reasoning-only
    assert!(!out("", "").is_reasoning_only());
}

#[test]
fn effective_text_falls_back_to_reasoning_only_when_reasoning_only() {
    assert_eq!(out("", "思考").effective_text(), "思考");
    assert_eq!(out("回复", "思考").effective_text(), "回复");
    // 有工具时不回退，正文为空即为空
    let mut with_tool = out("", "思考");
    with_tool.tool_calls.push(tool_call("vdfs_list"));
    assert_eq!(with_tool.effective_text(), "");
}

/// 端到端（纯内存）：reasoning-only 的 TurnOutput 落库消息里
/// 同一段 reasoning 只出现一次，且没有 Reasoning 子节点。
#[test]
fn into_messages_reasoning_only_has_no_duplicate_content() {
    let reasoning = "让我想想这个问题的关键点。";
    let msgs = out("", reasoning).into_messages("turn-x");

    assert_eq!(msgs.len(), 2, "应为 Turn + 单个 Text 子节点");
    assert_eq!(msgs[0].msg_type, Some(MessageType::Turn));
    assert!(
        !msgs
            .iter()
            .any(|m| m.msg_type == Some(MessageType::Reasoning)),
        "reasoning-only 不得产生 Reasoning 子节点"
    );

    let occurrences = msgs
        .iter()
        .filter(|m| {
            m.content
                .as_ref()
                .map(|c| c.to_text().contains(reasoning))
                .unwrap_or(false)
        })
        .count();
    assert_eq!(occurrences, 1, "同一段 reasoning 只能落库一份（factor=1）");
}

#[test]
fn into_messages_reasoning_with_reply_keeps_two_children() {
    let msgs = out("这是回复", "这是思考").into_messages("turn-y");
    assert_eq!(msgs.len(), 3);
    assert_eq!(
        msgs.iter()
            .filter(|m| m.msg_type == Some(MessageType::Reasoning))
            .count(),
        1
    );
    assert_eq!(
        msgs.iter()
            .filter(|m| m.msg_type == Some(MessageType::Text))
            .count(),
        1
    );
}

/// 形状不变式：`tool_calls` 是**结果形态**的唯一来源——`into_messages` 必须把字段里
/// 的工具调用原样落成 `ToolCall` 子节点（节点 id 与流式帧一致、参数落 JSON 文本）。
///
/// 这条锁的是「过程 → 结果」那次收口的接线：此前 `into_messages` 现调累积器的
/// `get_completed()`，工具调用的 id 由**另一次**调用产生，两处不一致即孤儿节点。
#[test]
fn into_messages_carries_tool_calls_from_the_result_field() {
    let mut o = out("", "");
    o.tool_calls.push(tool_call("vdfs_list"));
    let msgs = o.into_messages("turn-z");

    let calls: Vec<_> = msgs
        .iter()
        .filter(|m| m.msg_type == Some(MessageType::ToolCall))
        .collect();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "tc-1", "落库 id 必须等于产物里的节点 id");
    assert_eq!(calls[0].parent_id.as_deref(), Some("turn-z"));
    assert_eq!(calls[0].name.as_deref(), Some("vdfs_list"));
    assert_eq!(
        calls[0].content.as_ref().map(|c| c.to_text()).as_deref(),
        Some(r#"{"path":"."}"#)
    );
}
