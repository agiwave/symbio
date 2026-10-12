//! [`parse_sse_stream`] 的流级守卫判据。
//!
//! 主判据：**单响应工具调用数上限**（`MAX_TOOL_CALLS_PER_RESPONSE`）。
//! `6cf9520d` 号会话里模型在一条响应内流式输出了 56 个工具调用（同一批 9 个调用
//! 原样复读 6 遍），批执行器要等流结束才开工 ⇒ 60 余秒零结果，用户只能手动终止。
//! 这组用例钉住：超过上限的响应必须**当场以协议错误终止**（响亮失败），
//! 而不是把病态响应原样交给执行器。

use super::{dispatch_and_track, MAX_TOOL_CALLS_PER_RESPONSE};
use crate::plugins::model::protocols::ModelProtocolEvent;
use crate::plugins::model::tool_accumulator::TurnToolCallAccumulator;
use crate::symbio_core::{ExecEventSink, TurnOutput};

async fn feed_call(acc: &mut TurnToolCallAccumulator, out: &mut TurnOutput, idx: usize, args: &str) -> Result<(), String> {
    let sink = ExecEventSink::silent();
    dispatch_and_track(
        ModelProtocolEvent::ToolCallDelta(idx, Some(format!("call_{idx}")), Some("cmd".into()), Some(args.into())),
        "turn-root",
        &sink,
        out,
        acc,
        &mut false,
        std::time::Instant::now(),
    )
    .await
}

/// 复读形态（`6cf9520d` 实录）：同一批调用原样重发，第 33 个调用节点诞生时必须终止。
#[tokio::test]
async fn a_response_repeating_tool_calls_beyond_the_cap_fails_loudly() {
    let mut acc = TurnToolCallAccumulator::default();
    let mut out = TurnOutput::default();

    // 上限内的正常批次全部放行
    for idx in 0..MAX_TOOL_CALLS_PER_RESPONSE {
        feed_call(&mut acc, &mut out, idx, &format!(r#"{{"command":"job-{idx}"}}"#))
            .await
            .expect("上限内的调用不得报错");
    }

    // 第 33 个调用（复读的第 1 个）→ 协议错误，且错误信息可读、可指认成因
    let err = feed_call(&mut acc, &mut out, MAX_TOOL_CALLS_PER_RESPONSE, r#"{"command":"job-0"}"#)
        .await
        .expect_err("超限必须响亮失败");
    assert!(
        err.contains("工具调用数超过上限") && err.contains("32"),
        "错误信息要点明上限与成因：{err}"
    );

    // 超限的那个调用在判据触发前已被登记（判据在节点诞生后），但整条响应已被
    // 拒绝——执行器拿到的不是这批调用，而是本轮 Failed。
    assert_eq!(
        acc.call_count(),
        MAX_TOOL_CALLS_PER_RESPONSE + 1,
        "第 33 个调用登记后立即触发上限判据"
    );
}

/// 反向：上限内的多调用响应（含同名不同参）不受影响——守卫只拦数量，不拦正常批次。
#[tokio::test]
async fn a_healthy_multi_call_response_passes() {
    let mut acc = TurnToolCallAccumulator::default();
    let mut out = TurnOutput::default();

    for idx in 0..10 {
        feed_call(&mut acc, &mut out, idx, &format!(r#"{{"command":"check-{idx}"}}"#))
            .await
            .expect("10 个调用的健康批次不得报错");
    }
    assert_eq!(acc.call_count(), 10);
}
