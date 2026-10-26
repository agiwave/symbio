//! Anthropic Messages 协议的增量提取。
//!
//! 与 `parse_line` 不同，提取器看不到 SSE 的 `event:` 行（那是**上一行**），
//! 只能靠 JSON 自带的顶层 `type` 判别。

use super::super::partial_json::{
    assert_partial_matches_full_line, assert_tool_args_match_full_line, feed_chunked,
};
use super::*;

const TEXT: &str = r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"你好，世界"}}"#;
const THINKING: &str = r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"想一下"}}"#;
const PARTIAL_JSON: &str = r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"path\":\"a\\nb\"}"}}"#;

#[test]
fn text_delta_matches_full_line_at_every_chunk_boundary() {
    assert_partial_matches_full_line(&AnthropicProtocol::new(), TEXT);
}

#[test]
fn thinking_delta_matches_full_line_at_every_chunk_boundary() {
    assert_partial_matches_full_line(&AnthropicProtocol::new(), THINKING);
}

#[test]
fn partial_json_matches_full_line_at_every_chunk_boundary() {
    assert_tool_args_match_full_line(&AnthropicProtocol::new(), PARTIAL_JSON);
}

/// `text` / `partial_json` 这些字段名在别的 `type` 下含义完全不同：
/// `message_delta.delta` 里若出现 `text`，按字段名匹配就会凭空吐出正文
/// （完整行路径不产出任何文本 ⇒ 前端多出一段内容）。顶层 `type` 是唯一护栏。
#[test]
fn other_event_types_are_guarded_by_top_level_type() {
    let line = r#"data: {"type":"message_delta","delta":{"text":"不该出现"}}"#;
    let mut ext = AnthropicProtocol::new().open_partial_line(line).unwrap();
    assert!(feed_chunked(ext.as_mut(), line, 1).is_empty());
}

/// `content_block_start` 里的 `content_block.text` 不是增量（文本块起始处为空），
/// 路径不匹配就不该吐。
#[test]
fn content_block_start_is_not_extracted() {
    let line = r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":"泄露"}}"#;
    let mut ext = AnthropicProtocol::new().open_partial_line(line).unwrap();
    assert!(feed_chunked(ext.as_mut(), line, 1).is_empty());
}
