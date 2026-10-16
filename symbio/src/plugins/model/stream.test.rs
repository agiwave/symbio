//! [`parse_sse_stream`] 的流级守卫判据。
//!
//! 主判据：**单响应工具调用数上限**（`MAX_TOOL_CALLS_PER_RESPONSE`）。
//! `6cf9520d` 号会话里模型在一条响应内流式输出了 56 个工具调用（同一次复读里 9 个调用
//! 原样复读 6 遍），批执行器要等流结束才开工 ⇒ 60 余秒零结果，用户只能手动终止。
//!
//! 这组用例钉住**反馈语义**（整轮 `Err` 的旧方案已否决——模型必须能读到发生了
//! 什么，而不是流程无故中断）：
//! 1. 超限的调用**不失败**、不上线 UI 节点（防刷屏）；
//! 2. 收口时上限之外的调用带 `overflow_notice`（执行层据此落反馈结果）；
//! 3. 上限内的健康批次完全不受影响。

use super::{
    classify_stream_error, dispatch_and_track, mark_overflow_dropped, StreamFailure,
    MAX_TOOL_CALLS_PER_RESPONSE,
};
use crate::plugins::model::protocols::ModelProtocolEvent;
use crate::plugins::model::tool_accumulator::TurnToolCallAccumulator;
use crate::symbio_core::chat_node::ChatNode;
use crate::symbio_core::{ExecEventSink, ExecTranscriptWriter, TurnOutput};
use std::sync::Mutex;

/// 录制帧的转写写入器：数「ToolCall 节点」的快照帧用了几个不同 id。
struct RecordingWriter(pub Mutex<Vec<ChatNode>>);

#[async_trait::async_trait]
impl ExecTranscriptWriter for RecordingWriter {
    async fn apply(&self, message: ChatNode) {
        self.0.lock().expect("测试锁").push(message);
    }
}

async fn feed_call(
    acc: &mut TurnToolCallAccumulator,
    out: &mut TurnOutput,
    sink: &ExecEventSink,
    idx: usize,
    args: &str,
) -> Result<(), StreamFailure> {
    dispatch_and_track(
        ModelProtocolEvent::ToolCallDelta(
            idx,
            Some(format!("call_{idx}")),
            Some("cmd".into()),
            Some(args.into()),
        ),
        "turn-root",
        sink,
        out,
        acc,
        &mut false,
        std::time::Instant::now(),
        &mut std::collections::HashSet::new(),
        &mut None,
    )
    .await
}

/// 主判据（反馈语义）：超限的调用不失败、不上 UI；收口时带丢弃标记。
#[tokio::test]
async fn calls_beyond_the_cap_are_marked_dropped_with_feedback_not_failed() {
    let mut acc = TurnToolCallAccumulator::default();
    let mut out = TurnOutput::default();
    let sink = ExecEventSink::silent();

    // 超限批次（上限 + 5）全部放行：不再整轮 Err——模型必须拿到反馈而不是断流。
    for idx in 0..MAX_TOOL_CALLS_PER_RESPONSE + 5 {
        feed_call(
            &mut acc,
            &mut out,
            &sink,
            idx,
            &format!(r#"{{"command":"job-{idx}"}}"#),
        )
        .await
        .expect("超限是丢弃 + 反馈，不是失败");
    }
    assert_eq!(
        acc.call_count(),
        MAX_TOOL_CALLS_PER_RESPONSE + 5,
        "超限调用照常入账（provider 契约要求每个 tool_call_id 都有结果）"
    );

    // 收口（parse_sse_stream 的同名步骤）：上限之外的调用带反馈正文。
    let mut tool_calls = acc.finish();
    mark_overflow_dropped(&mut tool_calls);
    assert_eq!(tool_calls.len(), MAX_TOOL_CALLS_PER_RESPONSE + 5);
    for (pos, tc) in tool_calls.iter().enumerate() {
        if pos < MAX_TOOL_CALLS_PER_RESPONSE {
            assert!(
                tc.overflow_notice.is_none(),
                "上限内的第 {pos} 个调用不得带丢弃标记"
            );
        } else {
            let notice = tc.overflow_notice.as_ref().expect("超限调用必须带丢弃标记");
            assert!(
                notice.contains("上限") && notice.contains("32"),
                "反馈正文要点明上限数字与处置，实际：{notice}"
            );
        }
    }
}

/// 超限调用的**UI 抑制**：33 个调用的响应只上线 32 个 ToolCall 节点（防刷屏）。
#[tokio::test]
async fn ui_nodes_are_not_created_for_calls_beyond_the_cap() {
    let mut acc = TurnToolCallAccumulator::default();
    let mut out = TurnOutput::default();
    let writer = std::sync::Arc::new(RecordingWriter(Mutex::new(Vec::new())));
    let sink = ExecEventSink::direct(writer.clone());

    for idx in 0..MAX_TOOL_CALLS_PER_RESPONSE + 3 {
        feed_call(
            &mut acc,
            &mut out,
            &sink,
            idx,
            &format!(r#"{{"command":"job-{idx}"}}"#),
        )
        .await
        .expect("超限不得报错");
    }

    let mut tool_call_node_ids = std::collections::HashSet::new();
    for m in writer.0.lock().expect("测试锁").iter() {
        if m.is_tool_call() {
            tool_call_node_ids.insert(m.id.clone());
        }
    }
    assert_eq!(
        tool_call_node_ids.len(),
        MAX_TOOL_CALLS_PER_RESPONSE,
        "UI 只上线上限内的调用节点；超限的静默丢弃（结果反馈在执行层落）"
    );
}

/// 反向：上限内的多调用响应（含同名不同参）不受影响——守卫只拦数量，不拦正常批次。
#[tokio::test]
async fn a_healthy_multi_call_response_passes() {
    let mut acc = TurnToolCallAccumulator::default();
    let mut out = TurnOutput::default();
    let sink = ExecEventSink::silent();

    for idx in 0..10 {
        feed_call(
            &mut acc,
            &mut out,
            &sink,
            idx,
            &format!(r#"{{"command":"check-{idx}"}}"#),
        )
        .await
        .expect("10 个调用的健康批次不得报错");
    }
    assert_eq!(acc.call_count(), 10);
    let mut tool_calls = acc.finish();
    mark_overflow_dropped(&mut tool_calls);
    assert!(
        tool_calls.iter().all(|tc| tc.overflow_notice.is_none()),
        "健康批次不得带丢弃标记"
    );
}

// ==================== 流中途断线的分类与重发判据 ====================
//
// 主判据：`c2a87a21` 号会话的形态——5 个工具跑完后，**后续**请求在流已建立之后被
// 上游网关掐断（`UND_ERR_SOCKET` / 502）。请求级重试（`http.rs`）只覆盖「响应头
// 之前」，因此这一层必须自己给出「值得原样重发」的结论，否则整轮失败、会话停在
// 错误态，用户只能手动重发。

/// 会话 `c2a87a21` 里用户实际看到的错误体（原样）。
const GATEWAY_STREAM_ERROR: &str = r#"{"code":"streaming_error","details":{"cause":"SocketError: other side closed (code: UND_ERR_SOCKET)","errorCode":"UND_ERR_SOCKET","errorName":"TypeError","statusCode":502,"statusText":"Upstream Stream Terminated"},"message":"Upstream stream terminated unexpectedly before completion","param":null,"responseText":"\n\n","type":"gateway_error"}"#;

/// 网关的流中途断线必须判为**瞬时**（调用方据此原样重发），而不是整轮失败。
#[test]
fn gateway_stream_termination_is_classified_transient() {
    let f = classify_stream_error(GATEWAY_STREAM_ERROR);
    assert!(
        f.transient,
        "上游流中途终止必须判为瞬时——否则会话停在错误态，用户只能手动重发"
    );
}

/// 错误条不得把网关的原始 JSON 糊给用户：只留人读的 `message` 一行。
#[test]
fn gateway_error_json_is_not_echoed_verbatim() {
    let f = classify_stream_error(GATEWAY_STREAM_ERROR);
    assert!(
        !f.message.contains("UND_ERR_SOCKET"),
        "`details.cause` 是机器可读字段，不该出现在面向用户的文案里"
    );
    assert!(
        !f.message.contains("responseText"),
        "网关的空 `responseText` 对用户毫无意义"
    );
    assert!(
        f.message
            .contains("Upstream stream terminated unexpectedly before completion"),
        "人读原因（message 字段）必须保留，实际：{}",
        f.message
    );
}

/// 反向：鉴权/上下文类错误**不得**判为瞬时——原样重发只会再被拒，白等两次。
#[test]
fn authentication_failure_is_not_transient() {
    for raw in [
        r#"{"error":{"message":"Invalid API key provided"}}"#,
        r#"{"error":{"message":"context_length_exceeded"}}"#,
        r#"{"error":{"message":"401 unauthorized"}}"#,
    ] {
        assert!(
            !classify_stream_error(raw).transient,
            "鉴权/上下文类错误不该重发（重试注定失败）：{raw}"
        );
    }
}
