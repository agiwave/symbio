//! `session/context/conversation_view.rs` 的单元测试 —— C-D4 的强制点。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! C-D4 说：**对话投影不含 `ToolCall` / `role = tool` / Turn 容器**。违反它的后果
//! 不是"少了一条消息"，而是**工具结果被当成对话喂给判决 / 措辞插件**——那可能含
//! 几万 token 的源码或命令输出，既贵（每判一次都付）又错（模型会把工具输出读成
//! "用户说的话"）。因此本文件的核心是**逐类排除**，而不是"能取到用户消息"。

use super::*;
use crate::symbio_core::schemas::session::chat_message::{MessageContent, MessageStatus};

fn msg(
    id: &str,
    role: MessageRole,
    ty: Option<MessageType>,
    parent: Option<&str>,
    text: &str,
) -> ChatMessage {
    ChatMessage {
        id: id.to_string(),
        parent_id: parent.map(str::to_string),
        role: Some(role),
        msg_type: ty,
        content: Some(MessageContent::Text(text.to_string())),
        status: Some(MessageStatus::Completed),
        ..Default::default()
    }
}

/// 一份真实形状的转写：用户 → Turn（含 Reasoning / Text / ToolCall→tool 结果）→ 首响
fn transcript() -> Vec<ChatMessage> {
    vec![
        msg("u1", MessageRole::User, None, None, "读一下 README"),
        msg(
            "turn1",
            MessageRole::Assistant,
            Some(MessageType::Turn),
            None,
            "",
        ),
        msg(
            "r1",
            MessageRole::Assistant,
            Some(MessageType::Reasoning),
            Some("turn1"),
            "先看文件",
        ),
        msg(
            "t1",
            MessageRole::Assistant,
            Some(MessageType::Text),
            Some("turn1"),
            "文件已读完",
        ),
        msg(
            "tc1",
            MessageRole::Assistant,
            Some(MessageType::ToolCall),
            Some("turn1"),
            "{\"path\":\"README.md\"}",
        ),
        msg(
            "tr1",
            MessageRole::Tool,
            Some(MessageType::Text),
            Some("tc1"),
            "# Symbio\n很长的文件正文……",
        ),
        msg(
            "f1",
            MessageRole::Assistant,
            Some(MessageType::Text),
            None,
            "好的，我去看一下。",
        ),
        msg(
            "c1",
            MessageRole::Assistant,
            Some(MessageType::Compression),
            None,
            "正在压缩上下文…",
        ),
        msg(
            "s1",
            MessageRole::System,
            Some(MessageType::Text),
            None,
            "系统注入的框架文本",
        ),
    ]
}

/// C-D4 正面判据：投影里**只有**用户消息与根级 assistant 文本
#[test]
fn projection_keeps_only_the_conversation_line() {
    let ids: Vec<String> = conversation_view(&transcript(), 0)
        .iter()
        .map(|m| m.id.clone())
        .collect();
    assert_eq!(ids, vec!["u1", "f1"]);
}

/// C-D4 的逐类排除：三类节点一个都不能漏进来
#[test]
fn tool_and_turn_and_reasoning_are_excluded() {
    let view = conversation_view(&transcript(), 0);
    for m in &view {
        assert_ne!(
            m.msg_type,
            Some(MessageType::ToolCall),
            "ToolCall 不得进对话线"
        );
        assert_ne!(m.msg_type, Some(MessageType::Turn), "Turn 容器不得进对话线");
        assert_ne!(
            m.msg_type,
            Some(MessageType::Reasoning),
            "推理节点不得进对话线"
        );
        assert_ne!(m.role, Some(MessageRole::Tool), "role=tool 不得进对话线");
    }
}

/// 工具结果的**正文**一个字节都不该出现在投影里（不是"标记排除"，是根本不产出）
#[test]
fn tool_result_payload_never_leaks_into_the_projection() {
    let view = conversation_view(&transcript(), 0);
    let joined: String = view
        .iter()
        .filter_map(|m| m.content.as_ref().map(|c| c.to_text()))
        .collect();
    assert!(
        !joined.contains("很长的文件正文"),
        "工具结果正文泄漏进了对话线"
    );
}

/// 窗口：只保留**最后** N 条（判决 / 措辞要的是最近的对话，不是全量历史）
#[test]
fn limit_keeps_the_tail() {
    let all = conversation_view(&transcript(), 0);
    assert_eq!(all.len(), 2);
    let one = conversation_view(&transcript(), 1);
    assert_eq!(one.len(), 1);
    assert_eq!(one[0].id, "f1", "窗口必须保留最新的那条");
}

/// 空转写 ⇒ 空投影（不 panic、不补占位）
#[test]
fn empty_input_yields_empty_view() {
    assert!(conversation_view(&[], 5).is_empty());
}

/// 纯工具转写 ⇒ 空投影（对话线可以为空，这不是错误）
#[test]
fn transcript_without_conversation_nodes_yields_empty_view() {
    let only_tools = vec![
        msg(
            "turn1",
            MessageRole::Assistant,
            Some(MessageType::Turn),
            None,
            "",
        ),
        msg(
            "tc1",
            MessageRole::Assistant,
            Some(MessageType::ToolCall),
            Some("turn1"),
            "{}",
        ),
    ];
    assert!(conversation_view(&only_tools, 0).is_empty());
}

/// 投影是**副本**：改写它不影响存储（转写只有一个写入者，投影不得成为第二个）
#[test]
fn projection_is_a_copy() {
    let source = transcript();
    let mut view = conversation_view(&source, 0);
    view[0].content = Some(MessageContent::Text("被改写".to_string()));
    assert_eq!(
        source[0].content.as_ref().map(|c| c.to_text()).unwrap(),
        "读一下 README"
    );
}
