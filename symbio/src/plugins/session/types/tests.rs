//! `types` 模块的单元测试。
//!
//! 与实现分文件（约定同 `store/tests.rs` / `chat_session/tests.rs`）：
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
fn derive_title_takes_first_user_line() {
    let msgs = vec![user_msg("帮我分析一下这个报错\n第二行"), user_msg("第二条")];
    assert_eq!(
        derive_session_title(&msgs).as_deref(),
        Some("帮我分析一下这个报错")
    );
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
