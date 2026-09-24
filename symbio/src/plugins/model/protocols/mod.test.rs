//! `symbio/src/plugins/model/protocols/mod.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

/// SSE 载荷解析必须容忍「那个空格在不在」与行尾 `\r`——
/// 严格匹配 `"data: "` 的后果不是报错而是**静默空流**（连 `[DONE]` 都匹配不上）。
#[test]
fn sse_data_tolerates_missing_space_and_trailing_cr() {
    assert_eq!(sse_data("data: {\"a\":1}"), Some("{\"a\":1}"));
    assert_eq!(sse_data("data:{\"a\":1}"), Some("{\"a\":1}"));
    assert_eq!(sse_data("data:\t{\"a\":1}"), Some("{\"a\":1}"));
    assert_eq!(sse_data("data: {\"a\":1}\r"), Some("{\"a\":1}"));
    assert_eq!(sse_data("data: [DONE]"), Some("[DONE]"));
    assert_eq!(sse_data("data:[DONE]"), Some("[DONE]"));
}

/// 非数据行必须仍然返回 None（否则会被当成 JSON 去解析）
#[test]
fn sse_data_rejects_non_data_lines() {
    assert_eq!(sse_data("event: message_start"), None);
    assert_eq!(sse_data(""), None);
    assert_eq!(sse_data(": keep-alive"), None);
    assert_eq!(sse_data("{\"error\":1}"), None);
    assert_eq!(sse_data("data"), None);
}

#[test]
fn protocol_aliases_resolve_to_registered_ids() {
    assert_eq!(resolve_protocol_id("chat"), MODEL_PROTOCOL_OPENAI_CHAT);
    assert_eq!(
        resolve_protocol_id("responses"),
        MODEL_PROTOCOL_OPENAI_RESPONSES
    );
    assert_eq!(
        resolve_protocol_id("anthropic"),
        MODEL_PROTOCOL_ANTHROPIC_MESSAGES
    );
    assert_eq!(resolve_protocol_id("gemini"), MODEL_PROTOCOL_GEMINI_API);
    // 未识别值兜底 openai_chat
    assert_eq!(resolve_protocol_id("bogus"), MODEL_PROTOCOL_OPENAI_CHAT);
}
