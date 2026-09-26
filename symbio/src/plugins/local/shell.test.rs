//! `shell.rs` 的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）。
//!
//! 出口是 `ExecEventSink`，因此断言方式从「消费通道里的帧」变成「读记录型写入点里的
//! 消息帧」——**不再需要构造通道**，也就不再有「谁先返回」的顺序问题。

use super::*;
use crate::symbio_core::{ExecTranscriptWriter, PluginSimpleRequest};

/// 记录型写入点：把所有消息帧收进 `Vec`。
struct RecordingWriter {
    ops: Arc<std::sync::Mutex<Vec<ChatMessage>>>,
}

#[async_trait]
impl ExecTranscriptWriter for RecordingWriter {
    async fn apply(&self, message: ChatMessage) {
        self.ops.lock().unwrap().push(message);
    }
}

fn recording_sink() -> (ExecEventSink, Arc<std::sync::Mutex<Vec<ChatMessage>>>) {
    let ops = Arc::new(std::sync::Mutex::new(Vec::new()));
    (
        ExecEventSink::direct(Arc::new(RecordingWriter { ops: ops.clone() })),
        ops,
    )
}

/// 构造执行期上下文。`target` 为 `None` ⇒ 模拟 `route()` 直连调用（无占位节点）。
fn make_ctx(
    command: &str,
    sink: ExecEventSink,
    target: Option<(&str, &str)>,
) -> Arc<dyn PluginInvokeRequest> {
    let req = PluginSimpleRequest::new(None, None);
    req.set(crate::symbio_core::WORKDIR, ".".to_string());
    if let Some((msg_id, tool_call_id)) = target {
        req.set(crate::symbio_core::RESULT_MSG_ID, msg_id.to_string());
        req.set(crate::symbio_core::TOOL_CALL_ID, tool_call_id.to_string());
    }
    req.set(crate::symbio_core::EVENT_SINK, sink);
    req.set(crate::symbio_core::ABORT_SIGNAL, ExecAbortSignal::new());
    // 用裸字面量而非 PLUGIN_PAYLOAD_KEY：本行同时验证「桶名就是 "payload"」这一契约
    req.set_raw(
        "payload",
        Arc::new(json!({ "command": command, "approved": true })),
    );
    Arc::new(req)
}

/// 断言一组帧全是「按编排层 id 写的流式帧」。
fn assert_all_snapshots_use_orchestrator_ids(
    ops: &[ChatMessage],
    msg_id: &str,
    tool_call_id: &str,
) {
    for message in ops {
        assert_eq!(message.id, msg_id, "帧 id 必须是编排层预留的结果节点 id");
        assert_eq!(message.parent_id.as_deref(), Some(tool_call_id));
        assert_eq!(message.role, Some(MessageRole::Tool));
        assert_eq!(message.status, Some(MessageStatus::Streaming));
        assert!(
            message.delta.is_some() || message.content.is_some(),
            "每一帧都得带上这一段正文（delta 增量 / content 整条替换）"
        );
    }
}

/// 按生产路径调用工具：信封拆出 `args` / `env`，与 `capability_invoke` 同形。
///
/// 测试只关心 `execute` 本身，故在这里复刻那一步拆解——工具侧不再自己读信封。
async fn exec(tool: &ShellTool, ctx: Arc<dyn PluginInvokeRequest>) -> Result<Value, PluginError> {
    let args = ctx.payload::<Value>().unwrap_or(Value::Null);
    let env = ExecEnv::from_request(&*ctx);
    tool.execute(args, &env, ctx).await
}

// ── pump：增量帧的唯一产生点 ────────────────────────────────────────

/// 首行必然成帧（节流窗口在循环外被预置为「已过期」），且帧携带编排层给的 id。
#[tokio::test]
async fn pump_emits_snapshot_with_orchestrator_ids() {
    let (sink, ops) = recording_sink();
    let own = Arc::new(Mutex::new(String::new()));
    let handle = pump_lines(
        &b"hello\n"[..],
        own.clone(),
        Arc::new(Mutex::new(String::new())),
        false,
        sink,
        ExecAbortSignal::new(),
        Some(SnapshotTarget {
            msg_id: "res-1".into(),
            tool_call_id: "tc-1".into(),
        }),
    );
    handle.await.unwrap();

    assert_eq!(own.lock().unwrap().as_str(), "hello\n", "累积缓冲必须完整");
    let ops = ops.lock().unwrap();
    assert_eq!(ops.len(), 1, "首行必须立即成帧");
    assert_all_snapshots_use_orchestrator_ids(&ops, "res-1", "tc-1");
    assert_eq!(
        ops[0].delta.as_deref(),
        Some("hello\n"),
        "首帧即首行：增量帧携带的就是这一段新正文"
    );
}

/// 无快照身份（直连调用）：照常累积，但一帧不发。
#[tokio::test]
async fn pump_without_target_accumulates_but_emits_nothing() {
    let (sink, ops) = recording_sink();
    let own = Arc::new(Mutex::new(String::new()));
    pump_lines(
        &b"line1\nline2\n"[..],
        own.clone(),
        Arc::new(Mutex::new(String::new())),
        false,
        sink,
        ExecAbortSignal::new(),
        None,
    )
    .await
    .unwrap();

    assert_eq!(own.lock().unwrap().as_str(), "line1\nline2\n");
    assert!(ops.lock().unwrap().is_empty(), "没有身份就不该造节点");
}

/// `Null` 出口 + 有身份：同一份代码不发增量（内部请求 / 静默场景）。
#[tokio::test]
async fn pump_with_silent_sink_emits_nothing() {
    let own = Arc::new(Mutex::new(String::new()));
    pump_lines(
        &b"quiet\n"[..],
        own.clone(),
        Arc::new(Mutex::new(String::new())),
        false,
        ExecEventSink::silent(),
        ExecAbortSignal::new(),
        Some(SnapshotTarget {
            msg_id: "res-1".into(),
            tool_call_id: "tc-1".into(),
        }),
    )
    .await
    .unwrap();

    assert_eq!(own.lock().unwrap().as_str(), "quiet\n");
}

// ── execute：一条路径，两种降级 ───────────────────────────────────────

/// 会话内执行：返回工具结果本身（不再是 `Session` 通道），完整输出在 `output` 字段里。
#[tokio::test]
async fn execute_returns_data_payload_with_full_output() {
    let tool = ShellTool::new(Arc::new(SecurityPolicy::default()));
    let (sink, ops) = recording_sink();
    let ctx = make_ctx("echo hello_stream", sink, Some(("res-1", "tc-1")));

    let v = exec(&tool, ctx).await.unwrap();
    let out = v["output"].as_str().unwrap();
    assert!(out.contains("hello_stream"), "完整输出缺失: {out}");
    assert!(out.contains("[exit code: 0]"), "退出码缺失: {out}");
    assert_eq!(v["exit_code"].as_i64(), Some(0));

    // 快照帧（若有）必须只认编排层的 id——工具不得自造节点身份。
    let ops = ops.lock().unwrap();
    assert_all_snapshots_use_orchestrator_ids(&ops, "res-1", "tc-1");
}

/// 直连调用（无快照身份）：同一份代码照跑，静默返回。
#[tokio::test]
async fn execute_without_target_returns_output_silently() {
    let tool = ShellTool::new(Arc::new(SecurityPolicy::default()));
    let (sink, ops) = recording_sink();
    let ctx = make_ctx("echo hello_direct", sink, None);

    let v = exec(&tool, ctx).await.unwrap();
    assert!(v["output"].as_str().unwrap().contains("hello_direct"));
    assert_eq!(v["exit_code"].as_i64(), Some(0));
    assert!(ops.lock().unwrap().is_empty());
}

/// 大量输出不再有「通道容量」这一类阻塞源（旧回归：>64 帧时死锁）。
#[tokio::test]
async fn execute_many_lines_completes_without_backpressure() {
    let tool = ShellTool::new(Arc::new(SecurityPolicy::default()));
    let (sink, ops) = recording_sink();
    let ctx = make_ctx(
        "echo line1 & echo line2 & echo line3 & echo line4",
        sink,
        Some(("res-1", "tc-1")),
    );

    let v = exec(&tool, ctx).await.unwrap();
    let out = v["output"].as_str().unwrap();
    assert!(out.contains("line1"), "输出缺失: {out}");
    assert!(out.contains("line4"), "输出缺失: {out}");
    let ops = ops.lock().unwrap();
    assert_all_snapshots_use_orchestrator_ids(&ops, "res-1", "tc-1");
}

/// 参数非法 ⇒ `Err`（不再是「错误哨兵帧」）：错误由 `tool_executor` 统一收敛。
#[tokio::test]
async fn execute_rejects_empty_command() {
    let tool = ShellTool::new(Arc::new(SecurityPolicy::default()));
    let (sink, ops) = recording_sink();
    let ctx = make_ctx("", sink, Some(("res-1", "tc-1")));

    let err = exec(&tool, ctx).await.unwrap_err();
    assert!(
        format!("{err}").contains("command"),
        "错误信息应指向缺失的参数，实际: {err}"
    );
    assert!(ops.lock().unwrap().is_empty(), "未执行就不该有快照");
}

// ── ctx 读取口径 ──────────────────────────────────────────────────────

/// 两个 id 必须同行：缺任一即视为「直连调用」。
#[tokio::test]
async fn snapshot_target_requires_both_ids() {
    let bare = PluginSimpleRequest::new(None, None);
    assert!(SnapshotTarget::from_ctx(&bare).is_none());

    let half = PluginSimpleRequest::new(None, None);
    half.set(crate::symbio_core::RESULT_MSG_ID, "res-1".to_string());
    assert!(
        SnapshotTarget::from_ctx(&half).is_none(),
        "只有 msg_id 不足以定位快照"
    );

    let full = PluginSimpleRequest::new(None, None);
    full.set(crate::symbio_core::RESULT_MSG_ID, "res-1".to_string());
    full.set(crate::symbio_core::TOOL_CALL_ID, "tc-1".to_string());
    let t = SnapshotTarget::from_ctx(&full).expect("两个 id 齐备");
    assert_eq!(t.msg_id, "res-1");
    assert_eq!(t.tool_call_id, "tc-1");
}

/// 出口缺席 ⇒ 静默（`route()` 直连调用的默认）。
#[test]
fn event_sink_absent_means_silent() {
    let bare = PluginSimpleRequest::new(None, None);
    assert_eq!(
        format!("{:?}", ExecEventSink::of(&bare)),
        "ExecEventSink::Null"
    );
}
