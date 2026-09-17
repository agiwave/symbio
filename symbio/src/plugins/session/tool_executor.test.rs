//! `tool_executor` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `tool_executor.rs` 只保留生产代码，测试全部放本文件。

use super::*;
use crate::symbio_core::SimpleRequest;
use std::sync::atomic::AtomicBool;

fn test_ctx() -> Arc<dyn InvokeRequest> {
    Arc::new(SimpleRequest::new(None, None))
}

/// 需求 2 回归：工具调用 id 缺失/非法时不得跳过，必须作为工具调用失败处理——
/// 生成错误结果子节点（喂回 LLM）+ 父节点失败补丁，保证已落库的 ToolCall
/// 节点不会成为"无结果 tool_call"（下轮请求 400）。
#[tokio::test]
async fn missing_tool_call_id_is_recorded_as_failure() {
    let (_host, mut plugin_chan) = PluginChannel::pair(64);
    let abort = Arc::new(AtomicBool::new(false));
    let tcs = vec![ToolCallInfo {
        id: None,
        name: Some("vdfs_list".into()),
        arguments: json!({ "path": "." }),
        parse_error: None,
    }];

    let (msgs, updates) =
        process_tool_calls_async(tcs, &None, &mut plugin_chan, &abort, test_ctx()).await;

    assert_eq!(msgs.len(), 1, "必须生成失败结果子节点（而非跳过）");
    assert_eq!(msgs[0].role, Some(MessageRole::Tool));
    assert_eq!(msgs[0].status, Some(MessageStatus::Completed));
    assert!(
        msgs[0]
            .content
            .as_ref()
            .map(|c| c.to_text().contains("Error:"))
            .unwrap_or(false),
        "结果内容应携带错误信息"
    );
    assert!(!msgs[0].parent_id.as_deref().unwrap_or("").is_empty());

    assert_eq!(updates.len(), 1, "必须生成父节点失败补丁");
    assert_eq!(updates[0].status, Some(MessageStatus::Completed));
    assert_eq!(
        updates[0]
            .meta
            .as_ref()
            .and_then(|m| m.get("failure_kind"))
            .and_then(|v| v.as_str()),
        Some("error")
    );
}

/// 空串 id 与缺失同等对待（"不合法"）。
#[tokio::test]
async fn empty_tool_call_id_is_recorded_as_failure() {
    let (_host, mut plugin_chan) = PluginChannel::pair(64);
    let abort = Arc::new(AtomicBool::new(false));
    let tcs = vec![ToolCallInfo {
        id: Some(String::new()),
        name: Some("vdfs_list".into()),
        arguments: json!({}),
        parse_error: None,
    }];

    let (msgs, updates) =
        process_tool_calls_async(tcs, &None, &mut plugin_chan, &abort, test_ctx()).await;

    assert_eq!(msgs.len(), 1);
    assert_eq!(updates.len(), 1);
}

/// 工具名缺失/非法 → 同样作为失败处理（结果挂到已知 id 上，结构完整）。
#[tokio::test]
async fn missing_tool_name_is_recorded_as_failure() {
    let (_host, mut plugin_chan) = PluginChannel::pair(64);
    let abort = Arc::new(AtomicBool::new(false));
    let tcs = vec![ToolCallInfo {
        id: Some("tc-known".into()),
        name: Some(String::new()),
        arguments: json!({}),
        parse_error: None,
    }];

    let (msgs, updates) =
        process_tool_calls_async(tcs, &None, &mut plugin_chan, &abort, test_ctx()).await;

    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0].parent_id.as_deref(), Some("tc-known"));
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].id, "tc-known");
}

/// 回归（卡思考根因）：参数 JSON 解析失败（截断残破）的调用必须被拒绝执行，
/// 走协议失败路径——生成 role=Tool 错误子节点 + 父节点失败补丁，
/// 而非静默以 `{}` 执行（那会让模型看到「缺少必填参数」后原样重试）。
#[tokio::test]
async fn unparseable_arguments_are_refused_not_executed() {
    let (_host, mut plugin_chan) = PluginChannel::pair(64);
    let abort = Arc::new(AtomicBool::new(false));
    let tcs = vec![ToolCallInfo {
        id: Some("tc-broken".into()),
        name: Some("cmd".into()),
        arguments: json!({}),
        parse_error: Some(r#"{"command": "cargo test"#.into()),
    }];

    let (msgs, updates) =
        process_tool_calls_async(tcs, &None, &mut plugin_chan, &abort, test_ctx()).await;

    assert_eq!(msgs.len(), 1, "必须生成失败结果子节点（而非跳过）");
    assert_eq!(msgs[0].role, Some(MessageRole::Tool));
    assert_eq!(msgs[0].status, Some(MessageStatus::Completed));
    assert_eq!(msgs[0].parent_id.as_deref(), Some("tc-broken"));
    let text = msgs[0]
        .content
        .as_ref()
        .map(|c| c.to_text())
        .unwrap_or_default();
    assert!(
        text.contains("Error:") && text.contains("参数 JSON 解析失败"),
        "结果应说明参数 JSON 解析失败，实际：{text}"
    );

    assert_eq!(updates.len(), 1, "必须生成父节点失败补丁");
    assert_eq!(updates[0].id, "tc-broken");
    assert_eq!(updates[0].status, Some(MessageStatus::Completed));
}

/// 回归（真实事故）：中文参数摘要不得在字节边界上 panic。
///
/// 事故现场：`write_file` 传入含中文的 `content`，`args_summary(.., 200)`
/// 的 `&s[..200]` 落在 `'的'`（bytes 199..202）内部 →
/// `tokio-rt-worker` panic → ChatLoop join 失败 → 会话异常终止。
#[test]
fn args_summary_handles_multibyte_args() {
    use serde_json::json;

    // 构造必然跨越 200 字节边界的中文参数（'的' 3 字节）
    let payload = "参数摘要：截断到 max_chars，用于日志打印（避免超长参数刷屏）。".repeat(8);
    let args = json!({ "path": "a.rs", "content": payload });

    // 任意 max 都必须安全返回（旧实现会在部分 max 上 panic）
    for max in [1usize, 2, 3, 199, 200, 201, 512] {
        let s = args_summary(&args, max);
        assert!(
            s.chars().count() <= max + 32,
            "max={max} 摘要过长：{} 字节",
            s.len()
        );
    }

    // 超长时带长度标注，便于日志排查
    let long = args_summary(&args, 200);
    assert!(long.ends_with(&format!("(len={})", args.to_string().len())));
    assert!(long.contains('…'));

    // 短参数原样返回
    assert_eq!(args_summary(&json!({"a": 1}), 200), "{\"a\":1}");
}

/// S20.1 不变量：**本批每个 ToolCall 都必须以终态收场**。
///
/// 中止（`is_aborted`）时循环在第一个工具之前就 `break`，本批一个都没执行——
/// 若不收口，这些节点会停在参数流式阶段广播出的 `Streaming` 上：
/// 前端永远转「运行中」（把"卡住"伪装成"在跑"），重启后还会被
/// `cleanup_crashed_sessions` 误判为崩溃遗留。
#[tokio::test]
async fn aborted_batch_terminates_every_tool_call() {
    let (_host, mut plugin_chan) = PluginChannel::pair(64);
    let abort = Arc::new(AtomicBool::new(true)); // 已中止：本批一个都不执行
    let tcs = vec![
        ToolCallInfo {
            id: Some("tc1".into()),
            name: Some("vdfs_list".into()),
            arguments: json!({ "path": "." }),
            parse_error: None,
        },
        ToolCallInfo {
            id: Some("tc2".into()),
            name: Some("vdfs_list".into()),
            arguments: json!({ "path": "." }),
            parse_error: None,
        },
    ];

    let (msgs, updates) =
        process_tool_calls_async(tcs, &None, &mut plugin_chan, &abort, test_ctx()).await;

    assert!(msgs.is_empty(), "中止时不应产出任何工具结果子节点");
    assert_eq!(updates.len(), 2, "两个未执行的调用都必须收到终态补丁");
    for u in &updates {
        assert_eq!(u.status, Some(MessageStatus::Completed));
        assert_eq!(
            u.meta
                .as_ref()
                .and_then(|m| m.get("failure_kind"))
                .and_then(|v| v.as_str()),
            Some("not_executed"),
            "未执行必须留下可区分的标记，否则与「跑完了」无从分辨"
        );
        assert!(u.error.is_none(), "未执行不是错误：挂 error 会让前端渲染 ⚠");
    }
}

/// 未执行的终态必须是 `Completed` 而非 `Failed`。
///
/// `Failed` 会让 `get_context_messages` 过滤掉父 ToolCall 节点，留下"孤儿 tool 结果"
/// → 下一轮 LLM 请求携带非法 `tool_call_id`。与既有口径一致：工具失败属**信息性**，
/// 差异由 `meta.failure_kind` 承载。
#[test]
fn not_executed_patch_is_completed_not_failed() {
    let p = not_executed_patch("tc1", "not_executed");
    assert_eq!(p.id, "tc1");
    assert_eq!(p.status, Some(MessageStatus::Completed));
    assert!(p.error.is_none());
    assert_eq!(
        p.meta.as_ref().and_then(|m| m.get("success")),
        Some(&json!(false))
    );
    assert_eq!(
        p.meta.as_ref().and_then(|m| m.get("failure_kind")),
        Some(&json!("not_executed"))
    );
    // 成因可区分：「被钩子拦下」与「批次没轮到」不是同一件事，
    // 共用一个标记会让事后排查只能靠猜。
    assert_eq!(
        not_executed_patch("tc1", "blocked")
            .meta
            .as_ref()
            .and_then(|m| m.get("failure_kind")),
        Some(&json!("blocked"))
    );
}

// ==================== 工具执行期间的中止感知 ====================

/// `handle_abort` 投递的正是这条帧：主通道上的 `{"type":"abort"}`。
///
/// 非流式工具（`read_file` / `web_search` / MCP / `codebase_search` …，即绝大多数
/// 工具）在执行期间是**单次 await**，没有任何帧可观测。若不与中止信号 select，
/// 用户按下停止后工具照跑（最长硬超时 600s），整个 Turn 继续推进——用户以为停了，
/// 其实没停。这条测试锁定「中止帧能让等待立即结束」。
#[tokio::test]
async fn wait_tool_abort_returns_on_abort_frame() {
    let (host, mut plugin_chan) = PluginChannel::pair(64);
    let aborted = AtomicBool::new(false);

    host.tx
        .send(PluginFrame::Data(json!({ "type": "abort" })))
        .await
        .unwrap();

    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        wait_tool_abort(&mut plugin_chan, &aborted),
    )
    .await
    .expect("中止帧到达后应立即返回，而不是继续等工具");
    assert!(aborted.load(Ordering::Relaxed), "返回时必须置位中止标志");
}

/// 已置位的标志（上游已判定）→ 不必等任何帧。
#[tokio::test]
async fn wait_tool_abort_returns_immediately_when_already_aborted() {
    let (_host, mut plugin_chan) = PluginChannel::pair(64);
    let aborted = AtomicBool::new(true);
    tokio::time::timeout(
        std::time::Duration::from_millis(200),
        wait_tool_abort(&mut plugin_chan, &aborted),
    )
    .await
    .expect("标志已置位时应立即返回");
}

/// `cancel_token` 取消（会话销毁 / 消费循环超时兜底）→ 同样视为中止。
#[tokio::test]
async fn wait_tool_abort_returns_on_cancelled_token() {
    let (host, mut plugin_chan) = PluginChannel::pair(64);
    let aborted = AtomicBool::new(false);
    host.cancel_token.cancel();

    tokio::time::timeout(
        std::time::Duration::from_secs(2),
        wait_tool_abort(&mut plugin_chan, &aborted),
    )
    .await
    .expect("cancel_token 取消后应立即返回");
    assert!(aborted.load(Ordering::Relaxed));
}

/// **通道关闭 ≠ 中止**——回归测试。
///
/// 消费循环消失（会话正常收尾 / 被丢弃）会让 `rx.recv()` 返回 `None`。把 `None`
/// 直接当成中止返回，会在会话正常收尾时把**仍在跑**的工具误报为"被用户中止"，
/// 并让 `select!` 立刻 drop 掉工具 future。正确行为是：此后只保留
/// `is_aborted` / `cancel_token` 两条来源继续等。
#[tokio::test]
async fn wait_tool_abort_ignores_closed_channel() {
    let (host, mut plugin_chan) = PluginChannel::pair(64);
    let aborted = AtomicBool::new(false);
    drop(host); // 对端消失 → rx 关闭

    let r = tokio::time::timeout(
        std::time::Duration::from_millis(300),
        wait_tool_abort(&mut plugin_chan, &aborted),
    )
    .await;
    assert!(
        r.is_err(),
        "通道关闭被误判成了中止：正在跑的工具会被无故丢弃并报成「用户中止」"
    );
    assert!(!aborted.load(Ordering::Relaxed), "通道关闭不得置位中止标志");
}

/// 非 abort 的业务帧不得被当成中止。
#[tokio::test]
async fn wait_tool_abort_ignores_other_frames() {
    let (host, mut plugin_chan) = PluginChannel::pair(64);
    let aborted = AtomicBool::new(false);
    host.tx
        .send(PluginFrame::Data(
            json!({ "type": "status", "status": "idle" }),
        ))
        .await
        .unwrap();

    let r = tokio::time::timeout(
        std::time::Duration::from_millis(300),
        wait_tool_abort(&mut plugin_chan, &aborted),
    )
    .await;
    assert!(r.is_err(), "普通帧不是中止信号");
    assert!(!aborted.load(Ordering::Relaxed));
}
