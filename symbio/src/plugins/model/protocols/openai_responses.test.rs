//! OpenAI Responses 协议的增量提取。
//!
//! 本协议的特殊之处：内容 / 推理 / 工具参数**共用同一个 `delta` 字段名**，
//! 靠顶层 `type` 区分；且有两个「携带全量文本」的事件必须排除。

use super::super::partial_json::{
    assert_partial_matches_full_line, assert_tool_args_match_full_line, feed_chunked,
};
use super::*;

const TEXT: &str = r#"data: {"type":"response.output_text.delta","item_id":"i1","output_index":0,"content_index":0,"delta":"你好，世界"}"#;
const REASONING: &str =
    r#"data: {"type":"response.reasoning_text.delta","item_id":"i1","delta":"想一下"}"#;
const TOOL_ARGS: &str = r#"data: {"type":"response.function_call_arguments.delta","item_id":"i2","output_index":2,"delta":"{\"path\":\"a\\nb\"}"}"#;

#[test]
fn text_delta_matches_full_line_at_every_chunk_boundary() {
    assert_partial_matches_full_line(&OpenaiResponsesProtocol, TEXT);
}

#[test]
fn reasoning_delta_matches_full_line_at_every_chunk_boundary() {
    assert_partial_matches_full_line(&OpenaiResponsesProtocol, REASONING);
}

#[test]
fn tool_arguments_match_full_line_at_every_chunk_boundary() {
    assert_tool_args_match_full_line(&OpenaiResponsesProtocol, TOOL_ARGS);
}

/// `delta` 的含义由顶层 `type` 决定——同一个字段名在不同事件里必须落到不同事件类型。
#[test]
fn delta_kind_follows_the_top_level_type() {
    let mut ext = OpenaiResponsesProtocol.open_partial_line(TEXT).unwrap();
    let evs = feed_chunked(ext.as_mut(), TEXT, 5);
    assert!(!evs.is_empty());
    assert!(evs
        .iter()
        .all(|e| matches!(e, ProtocolEvent::ContentDelta(_))));

    let mut ext = OpenaiResponsesProtocol
        .open_partial_line(REASONING)
        .unwrap();
    let evs = feed_chunked(ext.as_mut(), REASONING, 5);
    assert!(!evs.is_empty());
    assert!(evs
        .iter()
        .all(|e| matches!(e, ProtocolEvent::ReasoningDelta(_))));
}

/// **携带全量文本的事件必须排除**：增量吐出去前端会重复一遍，而完整行路径对这些
/// 事件根本不产出文本事件 ⇒ 没有前缀截断来兜底。
#[test]
fn full_text_events_are_never_extracted_incrementally() {
    for etype in ["response.completed", "response.output_item.done"] {
        let line = format!(r#"data: {{"type":"{etype}","delta":"全量文本"}}"#);
        let mut ext = OpenaiResponsesProtocol.open_partial_line(&line).unwrap();
        assert!(
            feed_chunked(ext.as_mut(), &line, 1).is_empty(),
            "etype={etype} 不应被增量提取"
        );
    }
}

/// `output_index` 尚未出现时**放弃**本次增量，而不是用错下标产出事件——
/// 下标错了会把参数接到别的工具调用上，比「等换行」糟糕得多。
#[test]
fn tool_args_are_skipped_while_output_index_is_unknown() {
    let line =
        r#"data: {"type":"response.function_call_arguments.delta","delta":"xx","output_index":1}"#;
    let mut ext = OpenaiResponsesProtocol.open_partial_line(line).unwrap();
    assert!(feed_chunked(ext.as_mut(), line, 1).is_empty());
}
