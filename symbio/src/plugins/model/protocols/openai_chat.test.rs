//! OpenAI Chat 协议的增量提取：路径判定 + 与完整行的一致性。

use super::super::partial_json::{
    assert_partial_matches_full_line, assert_tool_args_match_full_line, feed_chunked,
};
use super::*;

const CONTENT: &str = r#"data: {"id":"1","choices":[{"index":0,"delta":{"content":"你好，世界"},"finish_reason":null}]}"#;
const REASONING: &str = r#"data: {"choices":[{"delta":{"reasoning_content":"先想一下\n再答"}}]}"#;
const TOOL_ARGS: &str = r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"vdfs_read","arguments":"{\"path\":\"a\\nb\"}"}}]}}]}"#;

#[test]
fn content_delta_matches_full_line_at_every_chunk_boundary() {
    assert_partial_matches_full_line(&OpenaiChatProtocol, CONTENT);
}

#[test]
fn reasoning_delta_matches_full_line_at_every_chunk_boundary() {
    assert_partial_matches_full_line(&OpenaiChatProtocol, REASONING);
}

#[test]
fn tool_arguments_match_full_line_at_every_chunk_boundary() {
    assert_tool_args_match_full_line(&OpenaiChatProtocol, TOOL_ARGS);
}

/// 同一行里有多个工具调用时，增量必须落在各自的下标上。
#[test]
fn tool_arguments_carry_the_array_index() {
    let line = r#"data: {"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"aa"}},{"index":1,"function":{"arguments":"bb"}}]}}]}"#;
    let mut ext = OpenaiChatProtocol.open_partial_line(line).unwrap();
    let evs = feed_chunked(ext.as_mut(), line, 3);
    assert_eq!(
        super::super::partial_json::tool_args_of(&evs),
        vec![(0, "aa".to_string()), (1, "bb".to_string())]
    );
}

/// 不相关的字段（`role` / `finish_reason` / `usage`）不得被当成增量吐出去。
#[test]
fn unrelated_fields_are_not_extracted() {
    let line = r#"data: {"choices":[{"index":0,"delta":{"role":"assistant"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3}}"#;
    let mut ext = OpenaiChatProtocol.open_partial_line(line).unwrap();
    assert!(feed_chunked(ext.as_mut(), line, 1).is_empty());
}

/// `[DONE]` 与非 data 行没有增量可吐。
#[test]
fn non_data_lines_produce_nothing() {
    for line in ["data: [DONE]", ": keep-alive", "event: ping"] {
        let mut ext = OpenaiChatProtocol.open_partial_line(line).unwrap();
        assert!(
            feed_chunked(ext.as_mut(), line, 1).is_empty(),
            "line={line}"
        );
    }
}
