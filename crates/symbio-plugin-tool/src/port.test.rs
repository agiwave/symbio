//! `port.tool` 提供者的单元测试：线上名解析 / 空闲超时 / 结果契约的行为判据
//! 随执行体一起搬到本插件（代码搬家，钉住行为的用例跟着搬，不在原处留副本）。
//!
//! 注册面那一半与 `notify` 端口同款：`port_rows` 产行、`capability_port_of` 读到
//! `Ready`、执行面可取。

use super::*;
use async_trait::async_trait;
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;

use std::sync::atomic::{AtomicUsize, Ordering};

use symbio_plugin_api::{
    Capability, CapabilityFamily, CapabilityMeta, CapabilityRiskLevel, CapabilityRow,
    ExecAbortSignal, ExecEnv, ExecEventSink, Plugin, PluginError, PluginInvokeRequest,
    PluginInvokeResponse, PluginMeta, PluginPayload, PluginSimpleRequest, PortPayload, PortReading,
    Registry, CAPABILITY_VISITOR,
};

// ==================== extract_result：判定顺序即约定 ====================
//
// 这组用例的作用是**钉住判定顺序**——它现在是 `extract_result` 文档里的表，
// 不再是「读源码才知道」的实现细节。顺序变了这里会红。

#[test]
fn content_wins_over_output() {
    assert_eq!(
        extract_result(&serde_json::json!({"content": "正文", "output": "命令输出"})),
        "正文"
    );
}

#[test]
fn output_is_read_when_content_absent() {
    assert_eq!(
        extract_result(&serde_json::json!({"output": "ls 结果"})),
        "ls 结果"
    );
}

#[test]
fn success_true_falls_back_to_the_whole_payload() {
    assert_eq!(
        extract_result(&serde_json::json!({"success": true, "n": 3})),
        r#"{"n":3,"success":true}"#
    );
}

#[test]
fn success_false_renders_the_error_field() {
    assert_eq!(
        extract_result(&serde_json::json!({"success": false, "error": "权限不足"})),
        "Error: 权限不足"
    );
}

#[test]
fn success_false_without_error_says_unknown() {
    assert_eq!(
        extract_result(&serde_json::json!({"success": false})),
        "Error: unknown error"
    );
}

#[test]
fn bare_string_is_taken_as_is() {
    assert_eq!(
        extract_result(&serde_json::json!("直接给一段文本")),
        "直接给一段文本"
    );
}

#[test]
fn opaque_payload_is_stringified() {
    assert_eq!(
        extract_result(&serde_json::json!({"path": "a.txt", "lines": 12})),
        r#"{"lines":12,"path":"a.txt"}"#
    );
    assert_eq!(extract_result(&serde_json::json!(null)), "null");
    assert_eq!(extract_result(&serde_json::json!(42)), "42");
}

#[test]
fn non_string_content_does_not_hijack_the_read() {
    assert_eq!(
        extract_result(&serde_json::json!({"content": [1, 2], "output": "真正的输出"})),
        "真正的输出"
    );
}

#[test]
fn empty_content_string_is_a_hit() {
    assert_eq!(
        extract_result(&serde_json::json!({"content": "", "output": "x"})),
        ""
    );
}

#[test]
fn result_field_constants_match_the_documented_order() {
    assert_eq!((RESULT_CONTENT, RESULT_OUTPUT), ("content", "output"));
}

/// 回归（真实事故）：中文参数摘要不得在字节边界上 panic。
#[test]
fn args_summary_handles_multibyte_args() {
    use serde_json::json;

    let payload = "参数摘要：截断到 max_chars，用于日志打印（避免超长参数刷屏）。".repeat(8);
    let args = json!({ "path": "a.rs", "content": payload });

    for max in [1usize, 2, 3, 199, 200, 201, 512] {
        let s = args_summary(&args, max);
        assert!(
            s.chars().count() <= max + 32,
            "max={max} 摘要过长：{} 字节",
            s.len()
        );
    }

    let long = args_summary(&args, 200);
    assert!(long.ends_with(&format!("(len={})", args.to_string().len())));
    assert!(long.contains('…'));

    assert_eq!(args_summary(&json!({"a": 1}), 200), "{\"a\":1}");
}

// ==================== 工具执行期间的中止感知 ====================
//
// 非流式工具在执行期间是**单次 await**，没有任何帧可观测。若不与中止信号 select，
// 用户按下停止后工具照跑（最长硬超时 600s）。契约一句：**只有 `abort()` 能让等待返回**。

#[tokio::test]
async fn wait_tool_abort_returns_when_signalled() {
    let abort = ExecAbortSignal::new();
    let waiter = abort.clone();

    let handle = tokio::spawn(async move { wait_tool_abort(&waiter).await });

    abort.abort();
    tokio::time::timeout(std::time::Duration::from_secs(2), handle)
        .await
        .expect("中止置位后应立即返回，而不是继续等工具")
        .expect("等待任务不应 panic");
}

#[tokio::test]
async fn wait_tool_abort_returns_immediately_when_already_aborted() {
    let abort = ExecAbortSignal::new();
    abort.abort();
    tokio::time::timeout(
        std::time::Duration::from_millis(200),
        wait_tool_abort(&abort),
    )
    .await
    .expect("信号已置位时应立即返回");
}

#[tokio::test]
async fn wait_tool_abort_stays_pending_without_signal() {
    let abort = ExecAbortSignal::new();
    let r = tokio::time::timeout(
        std::time::Duration::from_millis(300),
        wait_tool_abort(&abort),
    )
    .await;
    assert!(
        r.is_err(),
        "无中止来源时提前返回：正在跑的工具会被无故丢弃并报成「用户中止」"
    );
    assert!(!abort.is_aborted(), "未中止时信号不得被置位");
}

// ── 工具结果的「等待用户」意图 ────────────────────────────────────────────────

#[test]
fn pending_prompt_from_only_accepts_the_pending_kinds() {
    for kind in ["needs_approval", "needs_interaction"] {
        let data = serde_json::json!({
            "content": "需要确认：执行 cmd",
            "prompt": { "kind": "confirm" },
            "failure_kind": kind,
        });
        let p = pending_prompt_from(&data).unwrap_or_else(|| panic!("{kind} 应判为等待用户"));
        assert_eq!(p["failure_kind"], serde_json::json!(kind));
        assert_eq!(p["text"], serde_json::json!("需要确认：执行 cmd"));
        assert_eq!(p["prompt"]["kind"], serde_json::json!("confirm"));
    }

    for kind in ["error", "permission_denied", "tool_unavailable"] {
        let data = serde_json::json!({ "content": "x", "failure_kind": kind });
        assert!(
            pending_prompt_from(&data).is_none(),
            "{kind} 是信息性标记，不得收口为等待用户（会让会话永久挂起）"
        );
    }

    assert!(pending_prompt_from(&serde_json::json!({ "content": "ok" })).is_none());
}

#[test]
fn subagent_pending_payload_is_recognized_as_pending_intent() {
    let payload = serde_json::json!({
        "content": "需要确认：删除文件",
        "success": false,
        "failure_kind": symbio_plugin_api::failure_kind::NEEDS_APPROVAL,
        "prompt": { "tool_name": "agent_run", "args": { "session_id": "child-1" } },
    });
    let p = pending_prompt_from(&payload).expect("子会话待审批载荷必须被识别");

    assert_eq!(p["text"], serde_json::json!("需要确认：删除文件"));
    assert_eq!(p["prompt"]["tool_name"], serde_json::json!("agent_run"));
    assert_eq!(
        p["failure_kind"],
        serde_json::json!(symbio_plugin_api::failure_kind::NEEDS_APPROVAL)
    );
}

// ==================== 注册面 ====================

/// 最小登记表（与 `plugin-api` 的 port.test 同款夹具）。
struct FakeRegistry {
    rows: RwLock<Vec<Arc<CapabilityRow>>>,
}

impl FakeRegistry {
    fn new() -> Self {
        Self {
            rows: RwLock::new(Vec::new()),
        }
    }
}

#[async_trait]
impl Registry for FakeRegistry {
    async fn put(&self, row: CapabilityRow) {
        self.rows.write().await.push(Arc::new(row));
    }

    async fn get(&self, name: &str) -> Option<Arc<CapabilityRow>> {
        self.rows
            .read()
            .await
            .iter()
            .rev()
            .find(|r| r.name() == name)
            .cloned()
    }

    async fn list(&self, fam: CapabilityFamily) -> Vec<Arc<CapabilityRow>> {
        self.rows
            .read()
            .await
            .iter()
            .filter(|r| r.family() == fam)
            .cloned()
            .collect()
    }
}

#[tokio::test]
async fn tool_port_registers_one_row_and_exposes_an_executor() {
    let provider: Arc<dyn PortProvider> = Arc::new(ToolPort);
    assert_eq!(provider.ports(), &[TOOL], "提供者只申报 `tool` 一个端口");
    assert_eq!(provider.provider_id(), PLUGIN_ID_TOOL);

    let rows = symbio_plugin_api::port_rows(&provider);
    assert_eq!(rows.len(), 1, "单端口提供者只产一行");
    assert_eq!(rows[0].name(), "<port:tool>:tool");

    let registry = FakeRegistry::new();
    for row in rows {
        registry.put(row).await;
    }
    let PortReading::Ready(row) = symbio_plugin_api::capability_port_of(&registry, TOOL).await
    else {
        panic!("端口 `{TOOL}` 应就绪");
    };
    let provider = row
        .payload::<PortPayload>()
        .expect("端口行应带提供者")
        .provider()
        .clone();
    assert!(
        provider.executor().is_some(),
        "提供者必须交出执行面（DE-047 I-6.4）"
    );
}

// ==================== 执行面（经 `PortExecutor`）====================

/// 夹具能力：按 canonical 名注册，返回固定 JSON。
struct EchoTool {
    meta: CapabilityMeta,
    data: serde_json::Value,
}

#[async_trait]
impl Capability for EchoTool {
    fn meta(&self) -> CapabilityMeta {
        self.meta.clone()
    }

    async fn execute(
        &self,
        _args: serde_json::Value,
        _env: &ExecEnv,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<serde_json::Value, PluginError> {
        Ok(self.data.clone())
    }
}

/// 无路由的宿主插件：命中的调用不需要 route；未名回落由它给出诚实 NotFound。
struct NopPlugin;

#[async_trait]
impl Plugin for NopPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta::new("nop", "Nop")
    }

    async fn route(
        self: Arc<Self>,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Err(PluginError::NotFound("nop".into()))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Err(PluginError::NotFound("nop".into()))
    }
}

/// 带一条工具行的请求上下文（`CAPABILITY_VISITOR` 是执行期的数据源）。
async fn ctx_with_tool(
    parent: &Arc<dyn Plugin>,
    name: &str,
    data: serde_json::Value,
) -> Arc<dyn PluginInvokeRequest> {
    let req = PluginSimpleRequest::new(Some(Weak::clone(&Arc::downgrade(parent))), None);
    let registry: Arc<dyn Registry> = Arc::new(FakeRegistry::new());
    registry
        .put(CapabilityRow::tool(Arc::new(EchoTool {
            meta: CapabilityMeta {
                name: name.to_string(),
                ..Default::default()
            },
            data,
        }) as Arc<dyn Capability>))
        .await;
    req.set(CAPABILITY_VISITOR, registry);
    Arc::new(req)
}

/// 一次执行的最小现场（parent 在场——提供者最先查宿主）。
fn call(tool_name: &str, ctx: Arc<dyn PluginInvokeRequest>) -> (PortExecContext, PortInput) {
    (
        PortExecContext {
            deadline_ms: None,
            abort: ExecAbortSignal::new(),
            idempotency_key: "tc-1".to_string(),
            sink: ExecEventSink::silent(),
            emit: symbio_plugin_api::port::PortFactEmit::silent(),
            ctx: Some(ctx),
        },
        PortInput {
            port: TOOL,
            payload: serde_json::json!({
                "name": tool_name,
                "args": {},
                "call_id": "tc-1",
            }),
        },
    )
}

/// 产出三格。
fn decode(out: &PortOutcome) -> (String, bool, Option<serde_json::Value>) {
    assert!(out.error.is_none(), "工具失败是信息性事实：{out:?}");
    let fact = out.facts.first().expect("产出必须带一格事实");
    (
        fact["result_text"].as_str().unwrap_or_default().to_string(),
        fact["success"].as_bool().unwrap_or(false),
        fact["pending"].as_object().map(|_| fact["pending"].clone()),
    )
}

#[tokio::test]
async fn executes_through_the_capability_table() {
    let parent: Arc<dyn Plugin> = Arc::new(NopPlugin);
    let ctx = ctx_with_tool(&parent, "echo", serde_json::json!({ "content": "done" })).await;
    let (exec, input) = call("echo", ctx);
    let out = ToolPort.execute(exec, input).await;
    let (text, success, pending) = decode(&out);
    assert_eq!(text, "done");
    assert!(success);
    assert!(pending.is_none());
}

#[tokio::test]
async fn wire_names_resolve_to_canonical_capabilities() {
    let parent: Arc<dyn Plugin> = Arc::new(NopPlugin);
    let ctx = ctx_with_tool(
        &parent,
        "mcp.fs.read",
        serde_json::json!({ "content": "ok" }),
    )
    .await;
    let (exec, input) = call("mcp__fs__read", ctx);
    let out = ToolPort.execute(exec, input).await;
    let (text, success, _) = decode(&out);
    assert_eq!(
        text, "ok",
        "线上名必须解析到 canonical 名并命中表（而不是回落 route 的 NotFound）"
    );
    assert!(success);
}

#[tokio::test]
async fn unknown_names_do_not_get_guessed() {
    let parent: Arc<dyn Plugin> = Arc::new(NopPlugin);
    let ctx = ctx_with_tool(&parent, "echo", serde_json::json!({ "content": "done" })).await;
    let (exec, input) = call("totally_unknown", ctx);
    let out = ToolPort.execute(exec, input).await;
    let (text, success, _) = decode(&out);
    assert!(!success, "未知名不得被解析成别的工具");
    assert!(text.starts_with("Error:"), "诚实失败（实际: {text}）");
}

#[tokio::test]
async fn pending_intent_travels_through_the_outcome() {
    let parent: Arc<dyn Plugin> = Arc::new(NopPlugin);
    let ctx = ctx_with_tool(
        &parent,
        "confirm_tool",
        serde_json::json!({
            "content": "需要确认：删除文件",
            "failure_kind": "needs_approval",
            "prompt": { "kind": "confirm" },
        }),
    )
    .await;
    let (exec, input) = call("confirm_tool", ctx);
    let out = ToolPort.execute(exec, input).await;
    let (_, _, pending) = decode(&out);
    let pending = pending.expect("pending 闭集内的 failure_kind 必须被认出");
    assert_eq!(pending["failure_kind"], serde_json::json!("needs_approval"));
    assert_eq!(pending["text"], serde_json::json!("需要确认：删除文件"));
    assert_eq!(pending["prompt"]["kind"], serde_json::json!("confirm"));
}

// ==================== 授权判定：执行体先判，再执行（D-306 裁决一） ====================
//
// 载荷带 `threshold`（开账行写的判定输入）时，执行体先落一条授权事实，再决定是否
// 进入能力调用。拒绝必须是**判死**，不能静默放行；旧路径载荷不带 `threshold`，
// 保持旁路（双路径到期点：P4）。

/// 捕获 `emit` 的事实草稿（只验证「落了哪条」，去处由宿主决定）。
struct CapturedFacts(Arc<std::sync::Mutex<Vec<FactDraft>>>);

#[async_trait]
impl symbio_plugin_api::port::PortFactWriter for CapturedFacts {
    async fn emit(&self, facts: Vec<FactDraft>) {
        self.0.lock().unwrap().extend(facts);
    }
}

/// 一笔带判定输入的债（无宿主 ctx：授权之后的执行会诚实失败，不影响授权事实）。
fn authz_call(
    payload: serde_json::Value,
) -> (
    PortExecContext,
    PortInput,
    Arc<std::sync::Mutex<Vec<FactDraft>>>,
) {
    let captured: Arc<std::sync::Mutex<Vec<FactDraft>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let emit =
        symbio_plugin_api::port::PortFactEmit::new(Arc::new(CapturedFacts(captured.clone())));
    (
        PortExecContext {
            deadline_ms: None,
            abort: ExecAbortSignal::new(),
            idempotency_key: "req-1".to_string(),
            sink: ExecEventSink::silent(),
            emit,
            ctx: None,
        },
        PortInput {
            port: TOOL,
            payload,
        },
        captured,
    )
}

fn debt_input(risk: &str, threshold: &str, approved: bool) -> serde_json::Value {
    serde_json::json!({
        "tool": "shell.exec",
        "id": "tool-1",
        "attempt": 1,
        "risk": risk,
        "threshold": threshold,
        "approved": approved,
    })
}

#[tokio::test]
async fn low_risk_within_threshold_is_granted_by_the_threshold() {
    let (exec, input, captured) = authz_call(debt_input("low", "medium", false));
    let _ = ToolPort.execute(exec, input).await;
    let facts = captured.lock().unwrap();
    assert_eq!(facts.len(), 1, "一次判定只落一条授权事实");
    assert_eq!(facts[0].kind, "authz.granted");
    assert_eq!(facts[0].payload["port"], serde_json::json!("tool"));
    assert_eq!(facts[0].payload["id"], serde_json::json!("tool-1"));
    assert_eq!(facts[0].payload["attempt"], serde_json::json!(1));
    assert_eq!(facts[0].payload["risk"], serde_json::json!("low"));
    assert_eq!(facts[0].payload["basis"], serde_json::json!("threshold"));
    assert_eq!(facts[0].payload["deferred"], serde_json::json!(false));
}

#[tokio::test]
async fn high_risk_without_basis_is_denied_and_the_debt_fails() {
    let (exec, input, captured) = authz_call(debt_input("high", "low", false));
    let out = ToolPort.execute(exec, input).await;
    assert!(
        matches!(out.error, Some((PortErrorClass::NonRetryable, _))),
        "拒绝必须判死，不能静默放行：{out:?}"
    );
    let facts = captured.lock().unwrap();
    assert_eq!(facts[0].kind, "authz.denied");
    assert_eq!(
        facts[0].payload["why"],
        serde_json::json!("approval_required")
    );
}

#[tokio::test]
async fn approval_or_permanent_grant_grants_high_risk() {
    let mut payload = debt_input("high", "low", true);
    let (exec, input, captured) = authz_call(payload.clone());
    let _ = ToolPort.execute(exec, input).await;
    assert_eq!(
        captured.lock().unwrap()[0].payload["basis"],
        serde_json::json!("approved")
    );

    // 永久上限（grant）写在载荷里——开账行按授权清单算定，执行体不读清单。
    payload["approved"] = serde_json::json!(false);
    payload["grant"] = serde_json::json!("high");
    let (exec, input, captured) = authz_call(payload);
    let _ = ToolPort.execute(exec, input).await;
    assert_eq!(
        captured.lock().unwrap()[0].payload["basis"],
        serde_json::json!("grant")
    );
}

#[tokio::test]
async fn route_only_tools_are_deferred_not_granted() {
    let (exec, input, captured) = authz_call(serde_json::json!({
        "tool": "mcp.unknown",
        "id": "tool-2",
        "attempt": 1,
        "risk": "unknown",
        "threshold": "medium",
    }));
    let _ = ToolPort.execute(exec, input).await;
    let facts = captured.lock().unwrap();
    assert_eq!(facts[0].kind, "authz.granted");
    assert_eq!(facts[0].payload["deferred"], serde_json::json!(true));
    assert_eq!(facts[0].payload["risk"], serde_json::json!("unknown"));
}

#[tokio::test]
async fn invalid_calls_are_denied_before_any_approval_reading() {
    let mut payload = debt_input("low", "medium", true);
    payload["invalid"] = serde_json::json!(true);
    let (exec, input, captured) = authz_call(payload);
    let out = ToolPort.execute(exec, input).await;
    assert!(out.error.is_some());
    assert_eq!(
        captured.lock().unwrap()[0].payload["why"],
        serde_json::json!("invalid_call")
    );
}

#[tokio::test]
async fn legacy_payload_without_threshold_stays_on_the_old_path() {
    let parent: Arc<dyn Plugin> = Arc::new(NopPlugin);
    let ctx = ctx_with_tool(&parent, "echo", serde_json::json!({ "content": "done" })).await;
    let (exec, input) = call("echo", ctx);
    let captured: Arc<std::sync::Mutex<Vec<FactDraft>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut exec = exec;
    exec.emit =
        symbio_plugin_api::port::PortFactEmit::new(Arc::new(CapturedFacts(captured.clone())));
    let out = ToolPort.execute(exec, input).await;
    let (text, success, _) = decode(&out);
    assert_eq!(text, "done");
    assert!(success);
    assert!(
        captured.lock().unwrap().is_empty(),
        "旧载荷不得被新授权路径判一次（否则双路径会落两份授权）"
    );
}

// ==================== 并发策略：写侧独占、只读并行（D-311）====================

/// 并发探针：`execute` 期间计数同时在跑的条数，返回前睡一会儿拉长执行窗口。
struct ProbeTool {
    meta: CapabilityMeta,
    active: Arc<AtomicUsize>,
    peak: Arc<AtomicUsize>,
    sleep_ms: u64,
}

#[async_trait]
impl Capability for ProbeTool {
    fn meta(&self) -> CapabilityMeta {
        self.meta.clone()
    }

    async fn execute(
        &self,
        _args: serde_json::Value,
        _env: &ExecEnv,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<serde_json::Value, PluginError> {
        let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        tokio::time::sleep(std::time::Duration::from_millis(self.sleep_ms)).await;
        self.active.fetch_sub(1, Ordering::SeqCst);
        Ok(serde_json::json!({ "content": format!("done:{}", self.meta.name) }))
    }
}

fn probe(
    name: &str,
    risk: CapabilityRiskLevel,
    active: &Arc<AtomicUsize>,
    peak: &Arc<AtomicUsize>,
    sleep_ms: u64,
) -> Arc<dyn Capability> {
    Arc::new(ProbeTool {
        meta: CapabilityMeta {
            name: name.to_string(),
            risk: Some(risk),
            ..Default::default()
        },
        active: active.clone(),
        peak: peak.clone(),
        sleep_ms,
    })
}

async fn ctx_with_probes(
    parent: &Arc<dyn Plugin>,
    session: &str,
    tools: Vec<Arc<dyn Capability>>,
) -> Arc<dyn PluginInvokeRequest> {
    let req = PluginSimpleRequest::new(Some(Weak::clone(&Arc::downgrade(parent))), None);
    // 执行窗口按会话分格：每个用例用自己的会话 id，避免并行跑的单测共用一格。
    req.set(SESSION_ID, session.to_string());
    let registry: Arc<dyn Registry> = Arc::new(FakeRegistry::new());
    for tool in tools {
        registry.put(CapabilityRow::tool(tool)).await;
    }
    req.set(CAPABILITY_VISITOR, registry);
    Arc::new(req)
}

/// 写侧档（`Medium`）与只读档同时到达：写侧独占执行窗口，峰值并发必须 = 1。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn side_effect_calls_never_overlap_with_another_call() {
    for write_first in [true, false] {
        let active = Arc::new(AtomicUsize::new(0));
        let peak = Arc::new(AtomicUsize::new(0));
        let write = probe(
            "probe_write",
            CapabilityRiskLevel::Medium,
            &active,
            &peak,
            40,
        );
        let read = probe("probe_read", CapabilityRiskLevel::Low, &active, &peak, 20);
        let parent: Arc<dyn Plugin> = Arc::new(NopPlugin);
        let ctx = ctx_with_probes(&parent, "test-exec-window-write", vec![write, read]).await;
        let (first, second) = if write_first {
            ("probe_write", "probe_read")
        } else {
            ("probe_read", "probe_write")
        };
        let (a, b) = tokio::join!(
            ToolPort.execute(call(first, ctx.clone()).0, call(first, ctx.clone()).1),
            ToolPort.execute(call(second, ctx.clone()).0, call(second, ctx).1),
        );
        assert!(a.error.is_none() && b.error.is_none());
        assert_eq!(
            peak.load(Ordering::SeqCst),
            1,
            "写侧档不得与任何调用重叠（write_first={write_first}）"
        );
    }
}

/// 两条只读档同时到达：允许并行（受端口声明上界约束）。
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn read_only_calls_run_concurrently() {
    let active = Arc::new(AtomicUsize::new(0));
    let peak = Arc::new(AtomicUsize::new(0));
    let a = probe("probe_a", CapabilityRiskLevel::Low, &active, &peak, 40);
    let b = probe("probe_b", CapabilityRiskLevel::Low, &active, &peak, 40);
    let parent: Arc<dyn Plugin> = Arc::new(NopPlugin);
    let ctx = ctx_with_probes(&parent, "test-exec-window-read", vec![a, b]).await;
    let (x, y) = tokio::join!(
        ToolPort.execute(
            call("probe_a", ctx.clone()).0,
            call("probe_a", ctx.clone()).1
        ),
        ToolPort.execute(call("probe_b", ctx.clone()).0, call("probe_b", ctx).1),
    );
    assert!(x.error.is_none() && y.error.is_none());
    assert_eq!(
        peak.load(Ordering::SeqCst),
        2,
        "只读档应可同时在途（端口上界允许 2 条）"
    );
}

// ==================== 外部执行闸门：判据与开窗事实都在执行体 ====================
//
// 环只交中性输入（`gate` 四个值）；这里的用例钉 [DE-039 §6] 的两条验收：
// 拒绝**零事件**，熔断**必须**落 `control × opened`。

/// 带闸门输入的一笔债：捕获写口，其余与 [`call`] 同形。
fn gated_call(
    tool: &str,
    gate: serde_json::Value,
    ctx: Arc<dyn PluginInvokeRequest>,
) -> (
    PortExecContext,
    PortInput,
    Arc<std::sync::Mutex<Vec<FactDraft>>>,
) {
    let captured: Arc<std::sync::Mutex<Vec<FactDraft>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));
    let (mut exec, mut input) = call(tool, ctx);
    exec.emit =
        symbio_plugin_api::port::PortFactEmit::new(Arc::new(CapturedFacts(captured.clone())));
    input.payload["gate"] = gate;
    (exec, input, captured)
}

/// 预算之内：工具照常执行，写口不落任何事实。
#[tokio::test]
async fn affordable_calls_run_without_any_event() {
    let parent: Arc<dyn Plugin> = Arc::new(NopPlugin);
    let ctx = ctx_with_tool(&parent, "echo", serde_json::json!({ "content": "done" })).await;
    let (exec, input, captured) = gated_call(
        "echo",
        serde_json::json!({
            "authorized": true,
            "spent_ms": 0,
            "requested_ms": 500,
            "budget_ms": 1_000,
        }),
        ctx,
    );
    let out = ToolPort.execute(exec, input).await;
    let (text, success, _) = decode(&out);
    assert_eq!(text, "done");
    assert!(success);
    assert_eq!(
        captured.lock().unwrap().len(),
        0,
        "放行不是事实——没有开窗就没有事件"
    );
}

/// 验收 1：未授权 ⇒ 拒绝且**零事件**，工具不执行。
#[tokio::test]
async fn unauthorized_calls_are_refused_with_zero_events() {
    let parent: Arc<dyn Plugin> = Arc::new(NopPlugin);
    let ctx = ctx_with_tool(&parent, "echo", serde_json::json!({ "content": "done" })).await;
    let (exec, input, captured) = gated_call(
        "echo",
        serde_json::json!({
            "authorized": false,
            "spent_ms": 0,
            "requested_ms": 500,
            "budget_ms": 1_000,
        }),
        ctx,
    );
    let out = ToolPort.execute(exec, input).await;
    let (text, success, _) = decode(&out);
    assert!(text.starts_with("Refused:"), "实际: {text}");
    assert!(!success, "拒绝不是「照常执行后的输出」");
    assert_eq!(
        captured.lock().unwrap().len(),
        0,
        "拒绝**零事件**（DE-039 §6 验收 1）"
    );
}

/// 验收 2：预算耗尽 ⇒ 不开窗，且熔断事实落进事实源；结果交回 `tool_unavailable`。
#[tokio::test]
async fn exhausted_budget_breaks_and_lands_a_control_event() {
    let parent: Arc<dyn Plugin> = Arc::new(NopPlugin);
    let ctx = ctx_with_tool(&parent, "echo", serde_json::json!({ "content": "done" })).await;
    let (exec, input, captured) = gated_call(
        "echo",
        serde_json::json!({
            "authorized": true,
            "spent_ms": 900,
            "requested_ms": 500,
            "budget_ms": 1_000,
        }),
        ctx,
    );
    let out = ToolPort.execute(exec, input).await;
    let (text, success, _) = decode(&out);
    assert!(text.starts_with("Refused:"), "实际: {text}");
    assert!(!success);
    let fact = out.facts.first().expect("拒绝也要有结果事实");
    assert_eq!(
        fact["failure_kind"],
        serde_json::json!("tool_unavailable"),
        "熔断不是「等待用户」，是让模型改走别的路"
    );

    let facts = captured.lock().unwrap();
    assert_eq!(facts.len(), 1, "一笔熔断只落一条事实");
    assert_eq!(facts[0].key, "cb-tc-1");
    assert_eq!(
        facts[0].kind,
        symbio_plugin_api::events::EVENT_TASK_CONTROLLED
    );
    assert_eq!(facts[0].entity, Entity::Control);
    assert_eq!(facts[0].verb, Verb::Opened);
    assert_eq!(
        facts[0].payload["reason"],
        serde_json::json!("budget-exhausted")
    );
}

