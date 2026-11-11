//! `port.tool` 提供者的单元测试：线上名解析 / 空闲超时 / 结果契约的行为判据
//! 随执行体一起搬到本插件（代码搬家，钉住行为的用例跟着搬，不在原处留副本）。
//!
//! 注册面那一半与 `notify` 端口同款：`register_ports` 写进本次装配的绑定容器、
//! 按端口名取回绑定、执行面可取。

use super::*;
use async_trait::async_trait;
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;

use std::sync::atomic::{AtomicUsize, Ordering};

use symbio_plugin_api::{
    port_result, CapabilityFamily, CapabilityMeta, CapabilityRiskLevel, CapabilityRow,
    ExecAbortSignal, ExecEnv, ExecEventSink, Plugin, PluginError, PluginInvokeRequest,
    PluginInvokeResponse, PluginMeta, PluginPayload, PluginSimpleRequest, PortBindings,
    PortErrorClass, PortExecContext, PortExecutor, PortInput, PortOutcome, Registry,
    CAPABILITY_VISITOR, PORT_BINDINGS,
};

// ==================== 记账 fail-closed：造格型的账没落就不许报成功 ====================
//
// 判据只管**造格型**行为（`behaviors::Kind::LandsFacts`）：
// - 造格型 + 成功 + 0 草稿 ⇒ 违约（fail-closed）——挡「将来有人写出静默空转的行为」；
// - 富化型（只补 `file_op` 这类字段）与纯前置判断型本身不造格，0 草稿是正常形态，
//   **不能**判成「有账不落」（`file_change` 一度就是这样被误判的）。
// 种类的唯一来源是**实现自报**的 `kind()`（D-363），不在运行时猜 `after` 有没有被覆写。

/// 判据夹具：只在本测试二进制里登记，不依赖任何真插件被链接。
const FIXTURE_FILE_CHANGE: &str = "fixture_file_change";
const FIXTURE_TASK_TABLE: &str = "fixture_task_table";

struct FixtureEnricher;

impl symbio_plugin_api::ToolBehavior for FixtureEnricher {
    fn owner(&self) -> &'static str {
        "tests/enricher"
    }

    fn behavior(&self) -> &'static str {
        FIXTURE_FILE_CHANGE
    }

    fn kind(&self) -> symbio_plugin_api::behaviors::Kind {
        symbio_plugin_api::behaviors::Kind::NoFacts
    }
}

struct FixtureLander;

impl symbio_plugin_api::ToolBehavior for FixtureLander {
    fn owner(&self) -> &'static str {
        "tests/lander"
    }

    fn behavior(&self) -> &'static str {
        FIXTURE_TASK_TABLE
    }

    fn kind(&self) -> symbio_plugin_api::behaviors::Kind {
        symbio_plugin_api::behaviors::Kind::LandsFacts
    }
}

symbio_plugin_api::submit_behavior!(FixtureEnricher);
symbio_plugin_api::submit_behavior!(FixtureLander);

#[test]
fn declared_behavior_without_drafts_is_bookkeeping_missing() {
    // 造格型：0 草稿 ⇒ 违约。
    assert!(bookkeeping_missing(true, true, 0));
    // 执行本身失败：走失败路径，不叠加「记账违约」。
    assert!(!bookkeeping_missing(true, false, 0));
    // 落了账：合法。
    assert!(!bookkeeping_missing(true, true, 1));
    // 富化型 / 纯前置判断型：不造格，0 草稿不判违约。
    assert!(!bookkeeping_missing(false, true, 0));
}

#[test]
fn behavior_kinds_are_declared_per_name() {
    assert!(!symbio_plugin_api::effects::expects_facts(&[
        FIXTURE_FILE_CHANGE
    ]));
    assert!(symbio_plugin_api::effects::expects_facts(&[
        FIXTURE_TASK_TABLE
    ]));
}

#[test]
fn bookkeeping_missing_flips_artifact_to_failed() {
    let mut artifact = serde_json::json!({"success": true, "text": "ok"});
    mark_bookkeeping_missing(&mut artifact);
    assert_eq!(artifact["success"], serde_json::Value::Bool(false));
    assert_eq!(
        artifact["failure_kind"],
        serde_json::json!("bookkeeping_missing")
    );
    assert_eq!(
        artifact["why"],
        serde_json::json!("声明了行为却没有产出任何事实")
    );
}

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
///
/// 工具桩本身就是端口执行体（`D-440` ③：`Capability` 退场，桥随之消失）：**工具行**
/// 入本表、**端口绑定**入装配期容器（`register_tool` 一次写两处，D-468）——派发侧
/// 按端口名从容器取执行体，本表只管目录面。
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
async fn tool_port_registers_one_binding_and_exposes_an_executor() {
    let provider: Arc<dyn PortProvider> = Arc::new(ToolPort);
    assert_eq!(provider.ports(), &[TOOL], "提供者只申报 `tool` 一个端口");
    assert_eq!(provider.provider_id(), PLUGIN_ID_TOOL);

    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(PluginSimpleRequest::new(None, None));
    let bindings = Arc::new(PortBindings::default());
    ctx.set(PORT_BINDINGS, bindings.clone());
    symbio_plugin_api::port::register_ports(&ctx, provider.as_ref()).await;

    assert_eq!(bindings.bound(), vec![TOOL.to_string()], "一个端口一条绑定");
    let (owner, _executor) = bindings
        .get(TOOL)
        .expect("无冲突")
        .expect("端口 `tool` 应就绪");
    assert_eq!(owner, PLUGIN_ID_TOOL, "绑定记下提供者身份");
}

// ==================== 执行面（经 `PortExecutor`）====================

/// 夹具工具：按 canonical 名注册，返回固定 JSON。
struct EchoTool {
    meta: CapabilityMeta,
    data: serde_json::Value,
}

impl EchoTool {
    fn meta(&self) -> CapabilityMeta {
        self.meta.clone()
    }
}

#[async_trait]
impl PortExecutor for EchoTool {
    async fn execute(&self, exec: PortExecContext, input: PortInput) -> PortOutcome {
        let Some(ctx) = exec.ctx else {
            return PortOutcome::failed(
                PortErrorClass::NonRetryable,
                "工具端口没有调用方上下文，无法拆信封".to_string(),
            );
        };
        let env = ExecEnv::from_request(&*ctx);
        port_result(self.run(input.payload, &env, ctx).await)
    }
}

impl EchoTool {
    async fn run(
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

/// 带一条工具行与一条端口绑定的请求上下文（注册与派发同一条路）。
async fn ctx_with_tool(
    parent: &Arc<dyn Plugin>,
    name: &str,
    data: serde_json::Value,
) -> Arc<dyn PluginInvokeRequest> {
    let req = PluginSimpleRequest::new(Some(Weak::clone(&Arc::downgrade(parent))), None);
    let registry: Arc<dyn Registry> = Arc::new(FakeRegistry::new());
    // 注册走 `register_tool` 收口（与生产同一条路）：工具行进表、端口绑定进容器，
    // 派发侧按名从容器取执行体才有着落。
    req.set(CAPABILITY_VISITOR, registry);
    req.set(PORT_BINDINGS, Arc::new(PortBindings::default()));
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(req);
    let tool = EchoTool {
        meta: CapabilityMeta {
            name: name.to_string(),
            ..Default::default()
        },
        data,
    };
    symbio_providers::register_tool(&ctx, tool.meta(), None, Arc::new(tool)).await;
    ctx
}

/// 一次执行的最小现场（parent 在场——提供者最先查宿主）。
fn call(tool_name: &str, ctx: Arc<dyn PluginInvokeRequest>) -> (PortExecContext, PortInput) {
    (
        PortExecContext {
            abort: ExecAbortSignal::new(),
            idempotency_key: "tc-1".to_string(),
            sink: ExecEventSink::silent(),
            emit: symbio_plugin_api::port::PortFactEmit::silent(),
            ctx: Some(ctx),
            subject: String::new(),
            group: String::new(),
            session_id: String::new(),
            origin: None,
            scratch_dir: None,
            source: String::new(),
            reader: None,
        },
        PortInput {
            port: TOOL.to_string(),
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

/// 返回调用只认**本轮暴露快照**的别名表（D-445 ②）：不反演字符串、不回退
/// 「精确匹配优先」、内部端口名本身也不当别名用。
#[test]
fn resolve_only_accepts_this_rounds_exposure_aliases() {
    let mut aliases = std::collections::BTreeMap::new();
    aliases.insert("mcp.fs.read".to_string(), "mcp__fs__read".to_string());
    aliases.insert("shell".to_string(), "shell".to_string());
    assert_eq!(
        resolve("mcp__fs__read", &aliases).as_deref(),
        Some("mcp.fs.read")
    );
    assert_eq!(resolve("shell", &aliases).as_deref(), Some("shell"));
    assert!(
        resolve("mcp.fs.read", &aliases).is_none(),
        "内部端口身份不是 wire 别名，模型不能直接按它发起调用"
    );
    assert!(resolve("unknown", &aliases).is_none());
    assert!(resolve("", &aliases).is_none());
    // 装配期已拒绝 wire 碰撞；即便客户端硬塞一张有歧义的表，解析侧也**拒绝**
    // 而不是在 `a.b` / `a__b` 里「精确匹配优先」挑一个。
    let mut ambiguous = std::collections::BTreeMap::new();
    ambiguous.insert("a.b".to_string(), "a__b".to_string());
    ambiguous.insert("a__b".to_string(), "a__b".to_string());
    assert!(resolve("a__b", &ambiguous).is_none());
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

// ==================== 工具轮：清单来自事实读面，载荷留空 ====================

/// 记录执行体经写口交出的草稿。
#[derive(Default)]
struct RecordingWriter {
    drafts: std::sync::Mutex<Vec<symbio_plugin_api::FactDraft>>,
}

#[async_trait]
impl symbio_plugin_api::port::PortFactWriter for RecordingWriter {
    async fn emit(&self, facts: Vec<symbio_plugin_api::FactDraft>) {
        self.drafts.lock().unwrap().extend(facts);
    }
}

/// 事实读面的静态夹具。
struct StaticReader(Vec<symbio_plugin_api::port::PortFact>);

impl symbio_plugin_api::port::PortFactReader for StaticReader {
    fn facts(&self) -> Vec<symbio_plugin_api::port::PortFact> {
        self.0.clone()
    }
}

/// 工具轮的现场：债行载荷只带 `(request_id, call_id)`，清单在读面的 `tool.requested`。
fn requested_call(
    ctx: Arc<dyn PluginInvokeRequest>,
    recorder: Arc<RecordingWriter>,
) -> (PortExecContext, PortInput) {
    requested_call_with(
        ctx,
        recorder,
        "c1",
        serde_json::json!([
            { "id": "c1", "name": "echo", "arguments": { "content": "甲" } },
            { "id": "c2", "name": "echo", "arguments": { "content": "乙" } },
        ]),
    )
}

/// 同一条 `tool.requested`，这次执行只认 `call_id` 指名的那一笔。
fn requested_call_with(
    ctx: Arc<dyn PluginInvokeRequest>,
    recorder: Arc<RecordingWriter>,
    call_id: &str,
    calls: serde_json::Value,
) -> (PortExecContext, PortInput) {
    (
        PortExecContext {
            abort: ExecAbortSignal::new(),
            idempotency_key: "tool-1".to_string(),
            sink: ExecEventSink::silent(),
            emit: symbio_plugin_api::port::PortFactEmit::new(recorder),
            ctx: Some(ctx),
            subject: String::new(),
            group: String::new(),
            session_id: String::new(),
            origin: None,
            scratch_dir: None,
            source: String::new(),
            reader: Some(Arc::new(StaticReader(vec![
                symbio_plugin_api::port::PortFact {
                    id: "tool.requested:model-1".into(),
                    port: "model".into(),
                    kind: "tool.requested".into(),
                    payload: serde_json::json!({ "root_id": "u-1", "calls": calls }),
                },
            ]))),
        },
        PortInput {
            port: TOOL.to_string(),
            payload: serde_json::json!({
                "request_id": "tool.requested:model-1",
                "call_id": call_id,
            }),
        },
    )
}

/// 一笔债只执行它指名的那次调用：清单里的其余调用由各自的债负责。
///
/// `tool.calls.done` 不在执行体里写——收尾由簿记写方从各笔债的终局重算（D-378）。
#[tokio::test]
async fn one_debt_runs_only_the_call_it_names() {
    let parent: Arc<dyn Plugin> = Arc::new(NopPlugin);
    let ctx = ctx_with_tool(&parent, "echo", serde_json::json!({ "content": "done" })).await;
    let recorder = Arc::new(RecordingWriter::default());
    let (exec, input) = requested_call(ctx, recorder.clone());

    let out = ToolPort.execute(exec, input).await;
    assert!(out.error.is_none(), "工具轮不是协议失败：{out:?}");
    assert_eq!(out.facts.len(), 1, "这次债只交回 c1 的结果");

    let drafts = recorder.drafts.lock().unwrap().clone();
    let artifacts: Vec<_> = drafts
        .iter()
        .filter(|draft| draft.kind == EVENT_ARTIFACT_ADDED)
        .collect();
    assert_eq!(
        artifacts.len(),
        1,
        "只落被指名调用的产物（实际: {drafts:?}）"
    );
    assert_eq!(artifacts[0].key, "artifact:c1");
    assert_eq!(artifacts[0].payload["tool_call_id"], "c1");
    assert_eq!(artifacts[0].payload["text"], "done");
    assert_eq!(artifacts[0].payload["success"], true);
    assert!(
        !drafts.iter().any(|draft| draft.kind == "tool.calls.done"),
        "执行体不再写批次完成事实（由 round_done 投影补写）"
    );
}

/// 指名一笔不存在的调用：协议失败，不误执行别的调用。
#[tokio::test]
async fn a_debt_naming_an_unknown_call_is_a_protocol_error() {
    let parent: Arc<dyn Plugin> = Arc::new(NopPlugin);
    let ctx = ctx_with_tool(&parent, "echo", serde_json::json!({ "content": "done" })).await;
    let recorder = Arc::new(RecordingWriter::default());
    let (exec, input) = requested_call_with(
        ctx,
        recorder.clone(),
        "c9",
        serde_json::json!([{ "id": "c1", "name": "echo", "arguments": {} }]),
    );

    let out = ToolPort.execute(exec, input).await;
    assert!(
        matches!(out.error, Some((PortErrorClass::NonRetryable, _))),
        "清单里没有这一笔就是协议错：{out:?}"
    );
    assert!(out.facts.is_empty());
    assert!(
        recorder.drafts.lock().unwrap().is_empty(),
        "不拿另一笔调用顶替"
    );
}

/// 超限丢弃：派发处**不执行**，但必须留痕——收据 `success=false` +
/// `failure_kind=overflow_dropped`，正文就是模型写下的那一句。
///
/// 这是「收据不许与结果矛盾」的判据：同一笔调用只有一条结果事实，产物格从它派生。
#[tokio::test]
async fn overflow_dropped_call_is_not_executed_and_the_receipt_says_so() {
    const NOTICE: &str = "本次调用未执行：本条响应内的工具调用数超过上限（32）……";
    let parent: Arc<dyn Plugin> = Arc::new(NopPlugin);
    let ctx = ctx_with_tool(&parent, "echo", serde_json::json!({ "content": "done" })).await;
    let recorder = Arc::new(RecordingWriter::default());
    let (exec, input) = requested_call_with(
        ctx,
        recorder.clone(),
        "c2",
        serde_json::json!([
            { "id": "c1", "name": "echo", "arguments": { "content": "甲" } },
            { "id": "c2", "name": "echo", "arguments": { "content": "乙" },
              "overflow_notice": NOTICE },
        ]),
    );

    let out = ToolPort.execute(exec, input).await;
    assert!(out.error.is_none(), "丢弃不是协议失败：{out:?}");
    assert_eq!(out.facts.len(), 1, "被丢弃的调用同样交回一条结果事实");
    assert_eq!(
        out.facts[0]["success"],
        serde_json::json!(false),
        "没跑到的结果不许自称成功"
    );
    assert_eq!(
        out.facts[0]["failure_kind"],
        serde_json::json!("overflow_dropped")
    );

    let drafts = recorder.drafts.lock().unwrap().clone();
    let artifacts: Vec<_> = drafts
        .iter()
        .filter(|draft| draft.kind == EVENT_ARTIFACT_ADDED)
        .collect();
    assert_eq!(artifacts.len(), 1);
    assert_eq!(
        artifacts[0].payload["success"],
        serde_json::json!(false),
        "收据与结果同源：不得一处说未执行、一处记成功"
    );
    assert_eq!(
        artifacts[0].payload["failure_kind"],
        serde_json::json!("overflow_dropped")
    );
    assert_eq!(
        artifacts[0].payload["text"],
        serde_json::json!(NOTICE),
        "结果正文就是模型侧的丢弃提示"
    );
}

/// 指名一条读面里没有的 `tool.requested`：可重试，**不**拿「最近一条」顶替。
#[tokio::test]
async fn a_debt_naming_an_unseen_request_is_retryable_not_redirected() {
    let parent: Arc<dyn Plugin> = Arc::new(NopPlugin);
    let ctx = ctx_with_tool(&parent, "echo", serde_json::json!({ "content": "done" })).await;
    let recorder = Arc::new(RecordingWriter::default());
    let (exec, _) = requested_call_with(
        ctx,
        recorder.clone(),
        "c1",
        serde_json::json!([{ "id": "c1", "name": "echo", "arguments": {} }]),
    );
    let input = PortInput {
        port: TOOL.to_string(),
        payload: serde_json::json!({
            "request_id": "tool.requested:model-2",
            "call_id": "c1",
        }),
    };

    let out = ToolPort.execute(exec, input).await;
    assert!(
        matches!(out.error, Some((PortErrorClass::Retryable, _))),
        "目标请求尚未可见是半步时差：{out:?}"
    );
    assert!(
        recorder.drafts.lock().unwrap().is_empty(),
        "不拿另一条请求的清单顶替"
    );
}

/// 停等调用的产物载荷必须有 `needs_user_action=true` + `prompt` +
/// 顶层 `failure_kind`。
///
/// 产物载荷只有一个构造点（`artifact_payload`），逐笔派发也必须走它。从前批派自带一份
/// 只拷 `tool/text/args/success` 的装配，停等字段在那一遍里被丢掉，投影只好把审批卡
/// 渲染成普通 `body`（t27 / t78 抓到的就是这一处）。
#[tokio::test]
async fn pending_call_lands_the_waiting_card_fields() {
    let parent: Arc<dyn Plugin> = Arc::new(NopPlugin);
    let prompt = serde_json::json!({ "kind": "confirm", "question": "确认删除？" });
    let ctx = ctx_with_tool(
        &parent,
        "echo",
        serde_json::json!({
            "content": "需要确认后执行",
            "failure_kind": "needs_approval",
            "prompt": prompt,
        }),
    )
    .await;
    let recorder = Arc::new(RecordingWriter::default());
    let (exec, input) = requested_call_with(
        ctx,
        recorder.clone(),
        "c1",
        serde_json::json!([
            { "id": "c1", "name": "echo", "arguments": { "content": "甲" } },
        ]),
    );

    let out = ToolPort.execute(exec, input).await;
    assert!(
        matches!(out.error, Some((PortErrorClass::NeedsHuman, _))),
        "停等交回第三态「挂起」（不落收束、不开判死），不是判死：{out:?}"
    );

    let drafts = recorder.drafts.lock().unwrap().clone();
    let artifact = drafts
        .iter()
        .find(|draft| draft.kind == EVENT_ARTIFACT_ADDED)
        .expect("停等也要落产物（结果节点由它派生）");
    assert_eq!(
        artifact.payload["needs_user_action"],
        serde_json::json!(true),
        "投影凭这个字段渲染 user_prompt / waiting_user_action"
    );
    assert_eq!(artifact.payload["prompt"], prompt, "确认卡正文随产物落格");
    assert_eq!(
        artifact.payload["failure_kind"],
        serde_json::json!("needs_approval"),
        "停等成因在载荷顶层可见，不是只埋在 pending 里"
    );
    // 闸门拦下的一笔必须留痕（t78-A）：同批要有与原 `request` 同 id 的授权拒绝，
    // 且**不得**有同 id 的 `sent`——没放行就没派发。
    let denied = drafts
        .iter()
        .find(|draft| draft.kind == EVENT_AUTHZ_DENIED)
        .expect("审批拒绝必须落 authz.denied（否则事实面上一笔拒绝是空白）");
    assert_eq!(denied.payload["port"], serde_json::json!("tool"));
    assert_eq!(denied.payload["id"], serde_json::json!("c1"));
    assert_eq!(
        denied.payload["detail"],
        serde_json::json!("needs_approval"),
        "线上措辞由契约固定"
    );
    assert!(
        !drafts
            .iter()
            .any(|draft| draft.kind == "sent" && draft.payload["id"] == "c1"),
        "被拒的调用不得派发（不得落同 id 的 sent）"
    );
}

// ==================== 授权判定：执行体先判，再执行（D-306 裁决一） ====================
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
            abort: ExecAbortSignal::new(),
            idempotency_key: "req-1".to_string(),
            sink: ExecEventSink::silent(),
            emit,
            ctx: None,
            subject: String::new(),
            group: String::new(),
            session_id: String::new(),
            origin: None,
            scratch_dir: None,
            source: String::new(),
            reader: None,
        },
        PortInput {
            port: TOOL.to_string(),
            payload,
        },
        captured,
    )
}

/// 一笔 gate 调用：只判一次，不执行（`authorize_only`），模式默认交互。
fn debt_input(risk: &str, threshold: &str, approved: bool) -> serde_json::Value {
    serde_json::json!({
        "tool": "shell.exec",
        "id": "tool-1",
        "attempt": 1,
        "risk": risk,
        "threshold": threshold,
        "approved": approved,
        "authorize_only": true,
        "mode": "interactive",
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
        out.error.is_none() && out.facts.is_empty(),
        "gate 调用只判不执行：{out:?}"
    );
    let facts = captured.lock().unwrap();
    assert_eq!(facts[0].kind, "authz.denied");
    // 线上契约：`detail` 是措辞（e2e 读它），`why` 是内部原因码。
    assert_eq!(
        facts[0].payload["detail"],
        serde_json::json!("needs_approval")
    );
    assert_eq!(
        facts[0].payload["why"],
        serde_json::json!("approval_required")
    );
}

#[tokio::test]
async fn auto_mode_denies_with_permission_denied() {
    let mut payload = debt_input("high", "low", false);
    payload["mode"] = serde_json::json!("auto");
    let (exec, input, captured) = authz_call(payload);
    let _ = ToolPort.execute(exec, input).await;
    let facts = captured.lock().unwrap();
    assert_eq!(facts[0].kind, "authz.denied");
    assert_eq!(
        facts[0].payload["detail"],
        serde_json::json!("permission_denied")
    );
}

#[tokio::test]
async fn wake_approval_grants_and_rejection_denies_without_risk_comparison() {
    // 批准：用户动作直接放行（route-only 记为 deferred）。
    let (exec, input, captured) = authz_call(serde_json::json!({
        "tool": "shell.exec",
        "id": "tool-w1",
        "attempt": 1,
        "authorize_only": true,
        "resume": "approve",
    }));
    let _ = ToolPort.execute(exec, input).await;
    {
        let facts = captured.lock().unwrap();
        assert_eq!(facts[0].kind, "authz.granted");
        assert_eq!(facts[0].payload["basis"], serde_json::json!("approved"));
        assert_eq!(facts[0].payload["deferred"], serde_json::json!(true));
    }

    // 拒绝：用户给的判死，措辞 `user_rejected`。
    let (exec, input, captured) = authz_call(serde_json::json!({
        "tool": "shell.exec",
        "id": "tool-w2",
        "attempt": 1,
        "authorize_only": true,
        "resume": "reject",
    }));
    let _ = ToolPort.execute(exec, input).await;
    let facts = captured.lock().unwrap();
    assert_eq!(facts[0].kind, "authz.denied");
    assert_eq!(
        facts[0].payload["detail"],
        serde_json::json!("user_rejected")
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
        "authorize_only": true,
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
    payload["failure_kind"] = serde_json::json!("invalid_tool_call");
    let (exec, input, captured) = authz_call(payload);
    let out = ToolPort.execute(exec, input).await;
    assert!(
        out.error.is_none(),
        "gate 只落事实，不借 error 判死：{out:?}"
    );
    assert_eq!(
        captured.lock().unwrap()[0].payload["detail"],
        serde_json::json!("invalid_tool_call")
    );
    assert_eq!(
        captured.lock().unwrap()[0].payload["why"],
        serde_json::json!("invalid_call")
    );
}

/// 一次建账调用（`open_only`）：执行体只落 `request` 草稿，不执行、不写 `sent`。
fn open_call(
    ctx: Option<Arc<dyn PluginInvokeRequest>>,
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
            abort: ExecAbortSignal::new(),
            idempotency_key: "open-7-d2".to_string(),
            sink: ExecEventSink::silent(),
            emit,
            ctx,
            subject: "req-1".to_string(),
            group: String::new(),
            session_id: "s-1".to_string(),
            origin: Some(3),
            scratch_dir: None,
            source: String::new(),
            reader: None,
        },
        PortInput {
            port: TOOL.to_string(),
            payload,
        },
        captured,
    )
}

/// 建账判定的固定输入：一次模型应答 + 一条调用 + 策略读数字串。
fn open_payload(name: &str, grants: &str) -> serde_json::Value {
    serde_json::json!({
        "open_only": true,
        "epoch": 2,
        "policy": {
            "threshold": "medium",
            "grants": grants,
            "mode": "interactive",
        },
        "response": {
            "id": "model-7",
            "calls": [{ "id": "tool-7-d2-model-7-c000-tc-a" }],
        },
        "tool_calls": [{
            "id": "tc-a",
            "name": name,
            "arguments": { "path": "a.md" },
            "parse_error": false,
            "overflow_notice": false,
        }],
    })
}

#[tokio::test]
async fn open_writes_one_request_per_call_with_the_resolved_risk() {
    let parent: Arc<dyn Plugin> = Arc::new(NopPlugin);
    let ctx = ctx_with_tool(&parent, "echo", serde_json::json!({})).await;
    let (exec, input, captured) = open_call(Some(ctx), open_payload("echo", "{}"));
    let out = ToolPort.execute(exec, input).await;
    assert!(
        out.error.is_none() && out.facts.is_empty(),
        "建账即终局：{out:?}"
    );
    let facts = captured.lock().unwrap();
    assert_eq!(facts.len(), 1, "一条调用开一笔账");
    assert_eq!(facts[0].kind, "request");
    assert_eq!(facts[0].key, "request-tool-tool-7-d2-model-7-c000-tc-a-1");
    let payload = &facts[0].payload;
    assert_eq!(payload["port"], serde_json::json!("tool"));
    assert_eq!(payload["attempt"], serde_json::json!(1));
    assert_eq!(payload["name"], serde_json::json!("echo"));
    assert_eq!(payload["args"], serde_json::json!({ "path": "a.md" }));
    assert_eq!(payload["mode"], serde_json::json!("interactive"));
    assert_ne!(
        payload["risk"],
        serde_json::json!("unknown"),
        "注册表里有的能力必须判出风险档"
    );
    assert_eq!(payload["deferred"], serde_json::json!(false));
}

#[tokio::test]
async fn open_keeps_unresolvable_names_route_only() {
    let (exec, input, captured) = open_call(None, open_payload("mcp.unknown", "{}"));
    let out = ToolPort.execute(exec, input).await;
    assert!(out.facts.is_empty());
    let facts = captured.lock().unwrap();
    assert_eq!(facts[0].payload["risk"], serde_json::json!("unknown"));
    assert_eq!(facts[0].payload["deferred"], serde_json::json!(true));
    assert_eq!(facts[0].payload["threshold"], serde_json::json!("medium"));
}

#[tokio::test]
async fn open_marks_protocol_failures_invalid_and_falls_back_to_the_epoch_id() {
    let payload = serde_json::json!({
        "open_only": true,
        "epoch": 2,
        "policy": { "threshold": "medium", "grants": "{}", "mode": "" },
        "response": { "id": "model-7", "calls": [] },
        "tool_calls": [
            { "id": "", "name": "echo", "arguments": {}, "parse_error": false, "overflow_notice": false },
            { "id": "tc-b", "name": "echo", "arguments": {}, "parse_error": true, "overflow_notice": false },
        ],
    });
    let (exec, input, captured) = open_call(None, payload);
    let _ = ToolPort.execute(exec, input).await;
    let facts = captured.lock().unwrap();
    assert_eq!(facts.len(), 2);
    assert_eq!(
        facts[0].key, "request-tool-tool-req-1-d2-model-7-c000-1",
        "缺模型 id 的兜底段只由应答顺序决定"
    );
    assert_eq!(
        facts[0].payload["failure_kind"],
        serde_json::json!("invalid_tool_call")
    );
    assert_eq!(
        facts[1].payload["failure_kind"],
        serde_json::json!("invalid_tool_call")
    );
}

#[tokio::test]
async fn open_reads_the_grant_ceiling_by_canonical_name() {
    let parent: Arc<dyn Plugin> = Arc::new(NopPlugin);
    let ctx = ctx_with_tool(&parent, "echo", serde_json::json!({})).await;
    let (exec, input, captured) = open_call(Some(ctx), open_payload("echo", r#"{"echo":"high"}"#));
    let _ = ToolPort.execute(exec, input).await;
    assert_eq!(
        captured.lock().unwrap()[0].payload["grant"],
        serde_json::json!("high")
    );
}

/// 协议失败的三条读数各说各的：缺 id / 缺 name / 参数不可解析（D-312 的措辞在
/// 执行体，环不解释）。三者一律**不执行**、落一条失败结果，且成因可指认。
#[tokio::test]
async fn protocol_failures_each_name_their_own_cause() {
    let dir = std::path::Path::new(".");

    let missing_id = refuse_protocol(
        ProtocolInput {
            id_missing: true,
            name_missing: false,
            parse_error: None,
        },
        dir,
    )
    .await
    .expect("缺 id 必有结果");
    let (text, success, pending) = decode(&missing_id);
    assert!(!success);
    assert!(pending.is_none());
    assert!(text.contains("工具调用 ID"), "实得：{text}");
    assert_eq!(
        missing_id.facts[0]["failure_kind"],
        serde_json::json!("error")
    );

    let missing_name = refuse_protocol(
        ProtocolInput {
            id_missing: false,
            name_missing: true,
            parse_error: None,
        },
        dir,
    )
    .await
    .expect("缺 name 必有结果");
    let (text, success, _) = decode(&missing_name);
    assert!(!success);
    assert!(text.contains("工具名称"), "实得：{text}");

    let unparseable = refuse_protocol(
        ProtocolInput {
            id_missing: false,
            name_missing: false,
            parse_error: Some(r#"{"command": "cargo test"#),
        },
        dir,
    )
    .await
    .expect("参数不可解析必有结果");
    let (text, success, _) = decode(&unparseable);
    assert!(!success);
    assert!(text.contains("参数 JSON 解析失败"), "实得：{text}");
    assert!(text.contains("本次调用未执行"), "实得：{text}");
    assert!(text.contains("参数原文"), "实得：{text}");
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

impl ProbeTool {
    fn meta(&self) -> CapabilityMeta {
        self.meta.clone()
    }
}

#[async_trait]
impl PortExecutor for ProbeTool {
    async fn execute(&self, exec: PortExecContext, input: PortInput) -> PortOutcome {
        let Some(ctx) = exec.ctx else {
            return PortOutcome::failed(
                PortErrorClass::NonRetryable,
                "工具端口没有调用方上下文，无法拆信封".to_string(),
            );
        };
        let env = ExecEnv::from_request(&*ctx);
        port_result(self.run(input.payload, &env, ctx).await)
    }
}

impl ProbeTool {
    async fn run(
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
) -> Arc<ProbeTool> {
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
    tools: Vec<Arc<ProbeTool>>,
) -> Arc<dyn PluginInvokeRequest> {
    let req = PluginSimpleRequest::new(Some(Weak::clone(&Arc::downgrade(parent))), None);
    // 执行窗口按会话分格：每个用例用自己的会话 id，避免并行跑的单测共用一格。
    req.set(SESSION_ID, session.to_string());
    let registry: Arc<dyn Registry> = Arc::new(FakeRegistry::new());
    req.set(CAPABILITY_VISITOR, registry);
    req.set(PORT_BINDINGS, Arc::new(PortBindings::default()));
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(req);
    for tool in tools {
        symbio_providers::register_tool(&ctx, tool.meta(), None, tool).await;
    }
    ctx
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
    assert_eq!(facts[0].kind, crate::declarations::EVENT_TASK_CONTROLLED);
    assert_eq!(facts[0].entity, Entity::Control);
    assert_eq!(facts[0].verb, Verb::Opened);
    assert_eq!(
        facts[0].payload["reason"],
        serde_json::json!("budget-exhausted")
    );
}

// ==================== 批准留痕（D-368）：风险档原样搬，不在消费端重算 ====================

#[test]
fn approval_granted_carries_the_gate_risk_without_reparsing() {
    let artifact = serde_json::json!({
        "prompt": { "risk_level": "high", "content": "允许执行吗" }
    });
    let draft = approval_granted_draft("k1", &artifact, "call-1", "shell")
        .expect("审批卡之后应能落批准结论");
    assert_eq!(draft.kind, EVENT_AUTHZ_GRANTED);
    assert_eq!(draft.key, "k1-call-1.authz-approved");
    assert_eq!(draft.payload["id"], serde_json::json!("call-1"));
    assert_eq!(draft.payload["risk"], serde_json::json!("high"));
    assert_eq!(draft.payload["basis"], serde_json::json!("approved"));
    assert_eq!(draft.payload["deferred"], serde_json::json!(false));
}

#[test]
fn approval_without_a_risk_level_writes_nothing() {
    // 非审批路径的卡没有风险档：宁缺不假，不补一个 `unknown` 的授权结论。
    let artifact = serde_json::json!({ "prompt": { "content": "随便问一句" } });
    assert!(approval_granted_draft("k1", &artifact, "call-1", "ask_user").is_none());
}

// ==================== 非法参数：端口不解释参数形状（D-450）====================
//
// 参数合法性由**执行体自己**判（MCP server 的 `inputSchema` / 本地工具的前置检查）：
// 端口是零世界词的通用契约，它不知道也不该知道某个工具的 `arguments` 该长什么样。
// 在端口层塞一份 JSON Schema 校验等于给同一件事维护两处判定（schema 与执行体），
// 漂移形态是「校验放行的参数执行体拒绝」/「校验拒绝的参数执行体本可以跑」。
//
// 端口在这件事上要守的只有两条，都钉在下面：
// 1. **不经手**：参数原样交给执行体，不解析、不补默认、不把「坏形状」换成 `{}`——
//    换成 `{}` 就把「参数不合法」变成「参数合法但意图错了」，那才是误执行；
// 2. **不冒充**：执行体拒收即失败事实，`success` 不写 true，正文是拒收理由。

/// 夹具工具：只认 `{ "content": <string> }`，其余一律拒收——站的是真执行体那一侧。
struct StrictTool {
    meta: CapabilityMeta,
    seen: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
}

impl StrictTool {
    fn meta(&self) -> CapabilityMeta {
        self.meta.clone()
    }
}

#[async_trait]
impl PortExecutor for StrictTool {
    async fn execute(&self, _exec: PortExecContext, input: PortInput) -> PortOutcome {
        // 先记「我收到了什么」——判据 1 的证据只在这里。
        self.seen.lock().unwrap().push(input.payload.clone());
        match input.payload.get("content").and_then(|v| v.as_str()) {
            Some(text) => port_result(Ok(serde_json::json!({ "content": text }))),
            None => port_result(Err(PluginError::ValidationError(
                "arguments.content 必须是字符串".to_string(),
            ))),
        }
    }
}

/// 带一条**严格**工具行的请求上下文（同上，只是执行体自己判参数）。
async fn ctx_with_strict_tool(
    parent: &Arc<dyn Plugin>,
    name: &str,
    seen: Arc<std::sync::Mutex<Vec<serde_json::Value>>>,
) -> Arc<dyn PluginInvokeRequest> {
    let req = PluginSimpleRequest::new(Some(Weak::clone(&Arc::downgrade(parent))), None);
    let registry: Arc<dyn Registry> = Arc::new(FakeRegistry::new());
    req.set(CAPABILITY_VISITOR, registry);
    req.set(PORT_BINDINGS, Arc::new(PortBindings::default()));
    let ctx: Arc<dyn PluginInvokeRequest> = Arc::new(req);
    let tool = StrictTool {
        meta: CapabilityMeta {
            name: name.to_string(),
            ..Default::default()
        },
        seen,
    };
    symbio_providers::register_tool(&ctx, tool.meta(), None, Arc::new(tool)).await;
    ctx
}

#[tokio::test]
async fn malformed_arguments_reach_the_executor_verbatim_and_a_rejection_never_claims_success() {
    let parent: Arc<dyn Plugin> = Arc::new(NopPlugin);
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ctx = ctx_with_strict_tool(&parent, "strict", seen.clone()).await;

    // 正向对照：同一套夹具、同一条路径，**合法参数**必须报成功。没有它，下面那两条
    // `success: false` 就说不清是「拒收」还是「这条路径根本不产出成功」。
    let recorder = Arc::new(RecordingWriter::default());
    let (exec, input) = requested_call_with(
        ctx.clone(),
        recorder,
        "c0",
        serde_json::json!([{ "id": "c0", "name": "strict", "arguments": { "content": "甲" } }]),
    );
    let out = ToolPort.execute(exec, input).await;
    assert!(out.error.is_none(), "{out:?}");
    assert_eq!(
        out.facts[0]["success"],
        serde_json::json!(true),
        "合法参数必须跑通：{out:?}"
    );

    // 模型把参数写成**JSON 字符串**（有的提供方就这么发）。形状不对，但那是模型的原话：
    // 端口既不能替它解析成对象（越权解释形状），也不能换成 `{}`（把「不合法」洗成「合法」）。
    let recorder = Arc::new(RecordingWriter::default());
    let (exec, input) = requested_call_with(
        ctx.clone(),
        recorder,
        "c1",
        serde_json::json!([
            { "id": "c1", "name": "strict", "arguments": "{\"content\":\"甲\"}" }
        ]),
    );
    let out = ToolPort.execute(exec, input).await;
    assert!(
        out.error.is_none(),
        "工具拒收是信息性事实，不是协议失败：{out:?}"
    );
    assert_eq!(out.facts.len(), 1, "拒收的调用同样交回一条结果事实");
    assert_eq!(
        out.facts[0]["success"],
        serde_json::json!(false),
        "执行体没跑成的调用不许自称成功"
    );
    let text = out.facts[0]["result_text"].as_str().unwrap_or_default();
    assert!(
        text.contains("arguments.content"),
        "失败正文必须是执行体给的拒收理由，不能是端口的通用套话：{text}"
    );
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        &[
            serde_json::json!({ "content": "甲" }),
            serde_json::json!("{\"content\":\"甲\"}"),
        ],
        "参数原样交给执行体：不解析、不补默认、不把坏形状换成 {{}}"
    );

    // 缺 `arguments` 也一样：补 `{}` 等于替模型伪造了一份合法参数。
    let recorder = Arc::new(RecordingWriter::default());
    let (exec, input) = requested_call_with(
        ctx,
        recorder,
        "c2",
        serde_json::json!([{ "id": "c2", "name": "strict" }]),
    );
    let out = ToolPort.execute(exec, input).await;
    assert!(out.error.is_none(), "{out:?}");
    assert_eq!(out.facts[0]["success"], serde_json::json!(false));
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        &[
            serde_json::json!({ "content": "甲" }),
            serde_json::json!("{\"content\":\"甲\"}"),
            serde_json::Value::Null
        ],
        "缺参数就是缺参数（`Null`），不补空对象"
    );
}
