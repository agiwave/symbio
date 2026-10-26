//! Gemini API 协议的增量提取。
//!
//! Gemini 的行不带 `data:` 前缀（整行就是 JSON，可能被数组包裹），
//! 且 `functionCall.args` 每次都是**完整对象**，没有「参数在增长」这回事。

use super::super::partial_json::{assert_partial_matches_full_line, feed_chunked};
use super::*;

const TEXT: &str = r#"{"candidates":[{"content":{"parts":[{"text":"你好，世界"}],"role":"model"},"finishReason":"STOP"}]}"#;

#[test]
fn text_delta_matches_full_line_at_every_chunk_boundary() {
    assert_partial_matches_full_line(&GeminiProtocol, TEXT);
}

/// 一行里多个 `parts` 都要吐，且顺序与完整行一致。
#[test]
fn multiple_parts_are_all_extracted_in_order() {
    let line = r#"{"candidates":[{"content":{"parts":[{"text":"aa"},{"text":"bb"}]}}]}"#;
    assert_partial_matches_full_line(&GeminiProtocol, line);
}

/// 前导逗号不影响增量提取（扫描器自己找 JSON 起点）。
///
/// 这里只覆盖**真实的行形状**：Gemini 的 `streamGenerateContent` 不带 `alt=sse`，
/// 响应是逐个元素流出的 JSON 数组——`[` 与 `]` 各占一行，每个对象单独一行、
/// 元素之间用行首逗号分隔，所以行只有 `{...}` 与 `,{...}` 两种。
/// （`[{...}]` 挤在一行不是真实形状：`parse_line` 对它不产出任何事件，
/// 增量路径若产出就会与完整行路径不一致。）
#[test]
fn wrapped_lines_still_stream() {
    let bare = r#"{"candidates":[{"content":{"parts":[{"text":"hi"}]}}]}"#;
    for line in [bare.to_string(), format!(",{bare}")] {
        assert_partial_matches_full_line(&GeminiProtocol, &line);
    }
}

/// `functionCall.args` 是完整对象（不是增长的字符串），不做增量；
/// 完整行路径会一次性给出参数，因此这里吐任何东西都会变成重复。
#[test]
fn function_call_args_are_not_extracted_incrementally() {
    let line = r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"vdfs_read","args":{"path":"a"}}}]}}]}"#;
    let mut ext = GeminiProtocol.open_partial_line(line).unwrap();
    assert!(feed_chunked(ext.as_mut(), line, 1).is_empty());
}
