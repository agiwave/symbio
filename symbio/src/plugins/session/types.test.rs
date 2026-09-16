//! `types` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `types.rs` 只保留生产代码，测试全部放本文件。

use super::*;

#[test]
fn from_metadata_missing_returns_disabled_default() {
    let m = serde_json::json!({ "title": "x" });
    let hb = HeartbeatConfig::from_metadata(&m);
    assert!(!hb.enabled);
    assert_eq!(hb.interval_seconds, 300);
    assert!(hb.include_history);
    assert!(hb.prompt.is_empty());
}

#[test]
fn from_metadata_parses_full_config() {
    let m = serde_json::json!({
        "heartbeat": {
            "enabled": true,
            "interval_seconds": 120,
            "prompt": "请检查待办",
            "include_history": false
        }
    });
    let hb = HeartbeatConfig::from_metadata(&m);
    assert!(hb.enabled);
    assert_eq!(hb.interval_seconds, 120);
    assert_eq!(hb.prompt, "请检查待办");
    assert!(!hb.include_history);
}

#[test]
fn from_metadata_missing_fields_use_defaults() {
    let m = serde_json::json!({ "heartbeat": { "enabled": true } });
    let hb = HeartbeatConfig::from_metadata(&m);
    assert!(hb.enabled);
    assert_eq!(hb.interval_seconds, 300);
    assert!(hb.include_history);
}

fn user_msg(text: &str) -> ChatMessage {
    use crate::symbio_core::schemas::session::chat_message as cm;
    ChatMessage {
        id: "m1".into(),
        role: Some(cm::MessageRole::User),
        content: Some(cm::MessageContent::Text(text.into())),
        ..Default::default()
    }
}

#[test]
fn derive_title_takes_last_user_line() {
    // 取**最后一条**用户消息（列表名跟随最近在聊什么），且取它的**首行**
    let msgs = vec![
        user_msg("第一条消息\n它的第二行"),
        user_msg("最后一条消息\n它的第二行"),
    ];
    assert_eq!(derive_session_title(&msgs).as_deref(), Some("最后一条消息"));
}

#[test]
fn derive_title_truncates_long_text() {
    let long = "这是一个特别特别长的用户首条消息用来验证截断逻辑是否正常工作";
    let t = derive_session_title(&[user_msg(long)]).unwrap();
    assert!(t.ends_with('…'));
    assert!(t.chars().count() <= 25);
}

#[test]
fn derive_title_skips_empty_and_non_user() {
    use crate::symbio_core::schemas::session::chat_message as cm;
    let empty = ChatMessage {
        id: "m0".into(),
        role: Some(cm::MessageRole::User),
        content: Some(cm::MessageContent::Text("   \n  ".into())),
        ..Default::default()
    };
    assert_eq!(derive_session_title(&[empty]), None);
}

#[test]
fn display_title_prefers_metadata_then_content_then_default() {
    let mut s = Session::new("sess-123");
    assert_eq!(s.display_title(), "新对话");
    s.messages = vec![user_msg("帮我查天气")];
    assert_eq!(s.display_title(), "帮我查天气");
    s.metadata = serde_json::json!({ "title": "自定义名" });
    assert_eq!(s.display_title(), "自定义名");
}

fn assistant_msg(text: &str) -> ChatMessage {
    use crate::symbio_core::schemas::session::chat_message as cm;
    ChatMessage {
        id: "a1".into(),
        role: Some(cm::MessageRole::Assistant),
        content: Some(cm::MessageContent::Text(text.into())),
        ..Default::default()
    }
}

#[test]
fn summary_takes_last_assistant_reply() {
    // 摘要 = 最新一条**助手**回复（标题给用户最后说的话，摘要给助手最后的回答）
    let msgs = vec![
        assistant_msg("早先的回答"),
        user_msg("新的问题"),
        assistant_msg("最新的回答\n它的第二行"),
    ];
    assert_eq!(derive_session_summary(&msgs).as_deref(), Some("最新的回答"));
}

#[test]
fn summary_ignores_user_messages() {
    // 只有提问、还没有回答 ⇒ 没有摘要可给（那句提问已经在标题里了）
    assert_eq!(derive_session_summary(&[user_msg("只有提问")]), None);
}

#[test]
fn summary_skips_textless_assistant_messages() {
    use crate::symbio_core::schemas::session::chat_message as cm;
    // 纯工具调用 / 推理消息没有正文：继续往前找最近一条**有文本**的回复
    let tool_only = ChatMessage {
        id: "t1".into(),
        role: Some(cm::MessageRole::Assistant),
        content: None,
        ..Default::default()
    };
    let msgs = vec![assistant_msg("有文本的回复"), user_msg("再问"), tool_only];
    assert_eq!(
        derive_session_summary(&msgs).as_deref(),
        Some("有文本的回复")
    );
}
