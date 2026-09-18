//! `types` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `types.rs` 只保留生产代码，测试全部放本文件。

use super::*;
use serde_json::json;

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

// ==================== merge_metadata_object ====================
//
// 会话 metadata 有**两个**写入入口：`session/update` 路由（CLI 用）与
// `VdfsProvider::write`（前端用）。下面这几例锁住的是**语义**本身——两处共用
// 同一份实现，因此这些断言同时是两条路径的契约。

#[test]
fn merge_metadata_is_shallow_and_keeps_untouched_keys() {
    let mut s = Session::new("s1");
    s.metadata = json!({ "workdir": "/a", "agent_id": "x" });

    s.merge_metadata_object(&json!({ "metadata": { "workdir": "/b" } }));

    // 提到的键被覆盖，没提到的键**保持不变**（这正是"浅合并"的全部含义）
    assert_eq!(s.metadata["workdir"], json!("/b"));
    assert_eq!(s.metadata["agent_id"], json!("x"));
}

#[test]
fn merge_metadata_absent_field_leaves_metadata_alone() {
    let mut s = Session::new("s1");
    s.metadata = json!({ "workdir": "/a" });

    // 只给 title、不给 metadata ⇒ metadata 一个键都不该被动
    s.merge_metadata_object(&json!({ "title": "新名字" }));

    assert_eq!(s.metadata["workdir"], json!("/a"));
    assert_eq!(s.metadata["title"], json!("新名字"));
}

#[test]
fn merge_metadata_non_object_replaces_wholesale() {
    let mut s = Session::new("s1");
    s.metadata = json!({ "workdir": "/a" });

    // 非对象无法"逐键合并"⇒ 整体替换（与旧 `invoke_update` 的兜底分支一致）
    s.merge_metadata_object(&json!({ "metadata": null }));
    assert_eq!(s.metadata, json!(null));

    // 反向：自身不是对象时也整体替换（否则合并无处落笔）
    let mut s2 = Session::new("s2");
    s2.metadata = json!("不是对象");
    s2.merge_metadata_object(&json!({ "metadata": { "k": 1 } }));
    assert_eq!(s2.metadata, json!({ "k": 1 }));
}

#[test]
fn merge_metadata_writes_empty_title_verbatim() {
    let mut s = Session::new("s1");
    s.metadata = json!({ "title": "旧名字" });

    // 本方法**不做**空串判定：空标题是"显式清空"还是"忽略"由使用方决定，
    // 它只负责忠实写入。（新建会话时"路径名 vs 显式 title"的优先级是另一件事，
    // 在 `VdfsProvider::write` 的 create 分支里。）
    s.merge_metadata_object(&json!({ "title": "" }));
    assert_eq!(s.metadata["title"], json!(""));

    // 非字符串 title（如 null）不写 —— 类型不对就不该污染 metadata
    let mut s2 = Session::new("s2");
    s2.merge_metadata_object(&json!({ "title": null }));
    assert!(s2.metadata.get("title").is_none());
}

#[test]
fn merge_metadata_empty_object_is_a_noop() {
    let mut s = Session::new("s1");
    s.metadata = json!({ "workdir": "/a", "title": "旧" });

    s.merge_metadata_object(&json!({}));

    // 什么都不给的写入不该有任何副作用（否则"空写入"会变成清空）
    assert_eq!(s.metadata, json!({ "workdir": "/a", "title": "旧" }));
}
