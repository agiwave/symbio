//! `symbio/src/plugins/reply/compose.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 这里只测**不需要模型**的部分（纯函数 + 「没有模型服务」这条失败方向）：
//! 生成路径本身由 e2e `t22-reply.mjs` 在真实边界上验（mock LLM）。

use super::*;
use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole,
};

fn node(role: MessageRole, text: &str) -> ChatMessage {
    ChatMessage {
        id: llm_short_id(),
        role: Some(role),
        content: Some(MessageContent::Text(text.to_string())),
        ..Default::default()
    }
}

fn request(context: Vec<ChatMessage>) -> ComposeRequest {
    ComposeRequest {
        session_id: "s1".to_string(),
        verdict: crate::symbio_core::schemas::dialog::Verdict::Answered {
            reason: "from_context".to_string(),
        },
        context,
        snapshot: crate::symbio_core::schemas::dialog::RunSnapshot::default(),
    }
}

/// 消息列表**原样**转发投影：顺序与角色都不动。
///
/// 这里刻意不重排、不合并、不插"以下是历史"——投影已经是模型要的形状，
/// 再加工一次就是把同一份事实变成两种形状。
#[test]
fn messages_follow_the_projection_verbatim() {
    let context = vec![
        node(MessageRole::User, "第一个问题"),
        node(MessageRole::Assistant, "第一个回答"),
        node(MessageRole::User, "第二个问题"),
    ];
    let messages = build_messages(&context);

    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0].role, Some(MessageRole::User));
    assert_eq!(messages[1].role, Some(MessageRole::Assistant));
    assert_eq!(text_of(&messages[2]), "第二个问题");
}

/// 空正文节点剔除：投影只保证"是文本节点"，不保证"有正文"。
///
/// 留着它们会让模型收到一条空消息——部分协议直接 400，其余协议也会让它
/// 把"空白"当成一次发言。
#[test]
fn blank_text_nodes_are_dropped() {
    let context = vec![
        node(MessageRole::User, "有内容"),
        node(MessageRole::Assistant, "   "),
        node(MessageRole::User, ""),
    ];
    let messages = build_messages(&context);

    assert_eq!(messages.len(), 1);
    assert_eq!(text_of(&messages[0]), "有内容");
}

/// `non_empty` 去首尾空白后判空：只含空白的生成结果等于"什么都没说"，
/// 它比退回模板更糟——用户会看到一条空消息。
#[test]
fn only_non_blank_generation_survives() {
    assert_eq!(non_empty("  你好  ").as_deref(), Some("你好"));
    assert_eq!(non_empty(""), None);
    assert_eq!(non_empty("  \n\t "), None);
}

/// **没有模型服务 ⇒ 生成不了**（返回 `None`，由调用方退回模板）。
///
/// 这条是失败方向的可执行形式：本插件的调用点拿不到能力访问器（`CAPABILITY_VISITOR`
/// 缺席），必须**安静地**返回 `None`，而不是 panic 或造一句凭空的话。
#[tokio::test]
async fn generation_without_a_model_service_returns_none() {
    let ctx: Arc<dyn PluginInvokeRequest> =
        Arc::new(crate::symbio_core::PluginSimpleRequest::new(None, None));

    let out = generate(
        &ctx,
        &request(vec![node(MessageRole::User, "我们刚才聊了什么")]),
        &ReplyConfig::default(),
    )
    .await;
    assert!(out.is_none(), "没有模型服务时应返回 None");
}

/// 对话线里**没有用户发言** ⇒ 没有"要回答的那句话"，生成路径跳过。
///
/// 仓外调用方直呼本路由时可能给出这样的载荷（契约字段全部 `#[serde(default)]`）。
#[tokio::test]
async fn generation_without_a_user_turn_returns_none() {
    let ctx: Arc<dyn PluginInvokeRequest> =
        Arc::new(crate::symbio_core::PluginSimpleRequest::new(None, None));

    let only_assistant = vec![node(MessageRole::Assistant, "我先说一句")];
    let out = generate(&ctx, &request(only_assistant), &ReplyConfig::default()).await;
    assert!(out.is_none(), "没有用户发言时不该生成");

    let empty: Vec<ChatMessage> = Vec::new();
    let out = generate(&ctx, &request(empty), &ReplyConfig::default()).await;
    assert!(out.is_none(), "空对话线时不该生成");
}
