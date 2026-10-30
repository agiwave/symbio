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

// ── 思考分片的字段兼容（AMD MiMo 实测形状，2026-10-13）──────────────────
//
// AMD Radeon AI 开发者平台的 MiMo-V2.6-Flash 用 `delta.reasoning` 吐思考分片，
// 且首帧带 `reasoning_content: null`。此前只认 `reasoning_content`，
// 思考内容被静默丢弃，Reason 节点从未出现。

/// 真实端点首帧：`reasoning_content: null` + `content: ""`，不得产出思考增量。
/// （`ContentDelta("")` 是存量行为，不在此契约内。）
#[test]
fn amd_mimo_first_frame_with_null_reasoning_content_is_silent() {
    let line = r#"data: {"id":"f004","choices":[{"index":0,"delta":{"reasoning_content":null,"role":"assistant","content":""},"finish_reason":null}]}"#;
    assert!(
        !OpenaiChatProtocol
            .parse_line(line)
            .iter()
            .any(|e| matches!(e, ModelProtocolEvent::ReasoningDelta(_))),
        "reasoning_content 为 null 不得产出思考增量"
    );
}

/// 思考分片落在 `reasoning` 字段上时必须被提取（流式完整行路径）。
#[test]
fn reasoning_field_deltas_are_extracted() {
    let line = r#"data: {"choices":[{"delta":{"role":"assistant","reasoning":"This is a classic trick question."}}]}"#;
    let evs = OpenaiChatProtocol.parse_line(line);
    assert_eq!(evs.len(), 1, "恰好一条事件：{evs:?}");
    assert!(
        matches!(&evs[0], ModelProtocolEvent::ReasoningDelta(t) if t == "This is a classic trick question."),
        "应为 ReasoningDelta（正文=思考分片），实际 {evs:?}"
    );
}

/// `reasoning` 字段的增量提取与完整行一致（边收边吐路径，逐字节切块）。
#[test]
fn reasoning_field_matches_full_line_at_every_chunk_boundary() {
    let line = r#"data: {"choices":[{"delta":{"role":"assistant","reasoning":"先想一下\n再答"}}]}"#;
    assert_partial_matches_full_line(&OpenaiChatProtocol, line);
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

// ── 出厂只发一条 system──────────────────────────────────────────
//
// 部分 OpenAI 兼容网关只认「消息数组第一条是 system **且只有一条**」，两条 system
// 直接 400（`System message must be at the beginning.`）——那类 provider 上 `full`
// 档第一轮就走不出去。故前缀段并进基础 system，空行分段。

use crate::types::{MessageContent, NativeMessage, NodeRole};

fn sys(text: &str) -> NativeMessage {
    NativeMessage {
        role: NodeRole::System,
        content: Some(MessageContent::Text(text.to_string())),
        ..Default::default()
    }
}

/// 基础 system + 一条前缀 ⇒ 一条消息，文本按「基础 → 前缀」空行分段。
#[test]
fn leading_system_is_merged_into_one_message() {
    let (text, eaten) = merge_leading_system("基础", &[sys("记忆段"), sys("就绪段")]);
    assert_eq!(eaten, 2, "最前的连续 system 段全部并掉");
    assert_eq!(text, "基础\n\n记忆段\n\n就绪段", "顺序不变，空行分段");
}

/// 吃掉 0 条时行为与改前逐字相同（基础 system 原样，不产生前导空行）。
#[test]
fn no_leading_system_leaves_the_baseline_untouched() {
    let msgs = [NativeMessage {
        role: NodeRole::User,
        ..Default::default()
    }];
    let (text, eaten) = merge_leading_system("基础", &msgs);
    assert_eq!(eaten, 0);
    assert_eq!(text, "基础");
}

/// 基础 system 为空时不产生前导空行——「空 + 前缀」不得拼成 "\n\n前缀"。
#[test]
fn empty_baseline_does_not_leave_a_leading_blank_line() {
    let (text, eaten) = merge_leading_system("", &[sys("前缀")]);
    assert_eq!(eaten, 1);
    assert_eq!(text, "前缀");
}

/// 边界：遇到非纯文本的 system 段（Parts）就停下不再吃——拼错比多一条 system 更糟。
#[test]
fn non_text_system_segment_stops_the_merge() {
    let parts = NativeMessage {
        role: NodeRole::System,
        content: Some(MessageContent::Parts(vec![])),
        ..Default::default()
    };
    let (text, eaten) = merge_leading_system("基础", &[parts, sys("后面这条")]);
    assert_eq!(eaten, 0, "Parts 形态不猜，停下");
    assert_eq!(text, "基础");
}

/// 端到端：请求体里 system 消息**只有一条**，且前缀文本没有丢。
#[test]
fn prepared_request_carries_exactly_one_system_message() {
    let cfg = crate::model_providers::ModelProviderConfig {
        model: "m".to_string(),
        api_base: "http://localhost".to_string(),
        ..Default::default()
    };
    let msgs = vec![
        symbio_plugin_api::chat_node::ChatNode {
            id: "s1".to_string(),
            principal: Some(symbio_plugin_api::principal::Principal::from_legacy(
                "system/local",
            )),
            content: Some(MessageContent::Text("记忆段".to_string())),
            ..Default::default()
        },
        symbio_plugin_api::chat_node::ChatNode {
            id: "u1".to_string(),
            principal: Some(symbio_plugin_api::principal::Principal::from_legacy(
                symbio_plugin_api::PRINCIPAL_USER,
            )),
            content: Some(MessageContent::Text("问".to_string())),
            ..Default::default()
        },
    ];
    let req = OpenaiChatProtocol.prepare_request(&cfg, "基础", &msgs, &[]);
    let arr = req["messages"].as_array().expect("messages 数组");
    let n = arr.iter().filter(|m| m["role"] == "system").count();
    assert_eq!(n, 1, "出厂只发一条 system（两条会被部分网关 400）");
    assert_eq!(arr[0]["role"], "system", "它在最前");
    assert_eq!(arr[0]["content"], "基础\n\n记忆段", "两段文本都在，没丢");
    assert_eq!(arr[1]["role"], "user", "后续消息原序跟进");
}
