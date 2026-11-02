//! `port.tool` 提供者的单元测试：B1–B3（线上名解析 / 空闲超时 / 结果契约）的
//! 行为判据从 loop 的 `tool_executor.test.rs` 逐字迁来——代码搬家，
//! 钉住行为的用例跟着搬，不在原处留副本。

use super::*;
// 测试自己要的（`use super::*` 带不过来的）：
use symbio_kernel::capability::{Capability, CapabilityMeta, CapabilityRow, Registry};
use symbio_kernel::exec::ExecEventSink;
use symbio_kernel::plugin::PluginSimpleRequest;
use symbio_providers::DefaultToolVisitor;

// ==================== extract_result：判定顺序即约定 ====================
//
// 这组用例的作用是**钉住判定顺序**——它现在是 `extract_result` 文档里的表，
// 不再是「读源码才知道」的实现细节。顺序变了这里会红。

#[test]
fn content_wins_over_output() {
    // 1 优先于 2：两个都在时取 content
    assert_eq!(
        extract_result(&serde_json::json!({"content": "正文", "output": "命令输出"})),
        "正文"
    );
}

#[test]
fn output_is_read_when_content_absent() {
    // `shell` 的返回形状（字段名约定见 `local/shell.rs::execute_streaming`）
    assert_eq!(
        extract_result(&serde_json::json!({"output": "ls 结果"})),
        "ls 结果"
    );
}

#[test]
fn success_true_falls_back_to_the_whole_payload() {
    // 3：既无 content 也无 output、但 success=true → 整包 JSON
    // （形状不透明时宁可把结构给模型看，也不要凭空造一段文本）
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
    // 5：兜底——形状完全不认识时整包 JSON（含 `VdfsContent` 这类结构体序列化）
    assert_eq!(
        extract_result(&serde_json::json!({"path": "a.txt", "lines": 12})),
        r#"{"lines":12,"path":"a.txt"}"#
    );
    assert_eq!(extract_result(&serde_json::json!(null)), "null");
    assert_eq!(extract_result(&serde_json::json!(42)), "42");
}

#[test]
fn non_string_content_does_not_hijack_the_read() {
    // `content` 不是字符串 ⇒ 第 1 条不成立，继续往下走（不是「命中即返回」）。
    // 这是顺序判定与「取第一个存在的键」的分水岭：后者会把数组
    // 原样丢给模型。
    assert_eq!(
        extract_result(&serde_json::json!({"content": [1, 2], "output": "真正的输出"})),
        "真正的输出"
    );
}

#[test]
fn empty_content_string_is_a_hit() {
    // 空串是**合法**的正文（工具确实没有内容可说），不跳到 output
    assert_eq!(
        extract_result(&serde_json::json!({"content": "", "output": "x"})),
        ""
    );
}

#[test]
fn result_field_constants_match_the_documented_order() {
    // 常量与文档表里的字段名一致（改常量就得改文档，反之亦然）
    assert_eq!((RESULT_CONTENT, RESULT_OUTPUT), ("content", "output"));
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

// ==================== 工具执行期间的中止感知 ====================
//
// 非流式工具（`read_file` / `web_search` / MCP / `codebase_search` …，即绝大多数
// 工具）在执行期间是**单次 await**，没有任何帧可观测。若不与中止信号 select，
// 用户按下停止后工具照跑（最长硬超时 600s），整个 Turn 继续推进——用户以为停了，
// 其实没停。契约一句：**只有 `abort()` 能让等待返回**。

/// 中止置位 → 等待立即结束。
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

/// 已置位的信号（上游已判定）→ 立即返回，不挂起。
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

/// **未被中止就不得返回**——回归测试。
///
/// 这是「工具照跑但用户以为停了」的反面：若等待函数在没有任何中止来源时提前返回，
/// `select!` 会立刻 drop 掉仍在运行的工具 future，把它误报成「被用户中止」。
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
//
// 契约**工具只声明意图与载荷，节点归编排层构造**。工具不自建 Session 通道、
// 发一个随机 id 的节点，由消费方解回来改 id 再重播一次——同一个逻辑节点因此有两个
// id，前端出现重复审批卡，且 resume 只删得掉一个。

/// `failure_kind` 是**控制流**判据，因此只认闭集里的两个 pending 取值。
///
/// 反面同样重要：`error` / `permission_denied` / `tool_unavailable` 都是**信息性**
/// 标记。把它们当成「等待用户」会让自动模式下**无人可授权**的工具把会话永久挂起
/// ——本该让 LLM 换条路继续，却变成等一个永远不会来的回答。
#[test]
fn pending_prompt_from_only_accepts_the_pending_kinds() {
    for kind in ["needs_approval", "needs_interaction"] {
        let data = serde_json::json!({
            "content": "需要确认：执行 cmd",
            "prompt": { "kind": "confirm" },
            "failure_kind": kind,
        });
        let p = pending_prompt_from(&data).unwrap_or_else(|| panic!("{kind} 应判为等待用户"));
        assert_eq!(p.failure_kind, kind);
        assert_eq!(p.text, "需要确认：执行 cmd");
        assert_eq!(p.prompt["kind"], serde_json::json!("confirm"));
    }

    for kind in ["error", "permission_denied", "tool_unavailable"] {
        let data = serde_json::json!({ "content": "x", "failure_kind": kind });
        assert!(
            pending_prompt_from(&data).is_none(),
            "{kind} 是信息性标记，不得收口为等待用户（会让会话永久挂起）"
        );
    }

    // 没有 failure_kind ⇒ 普通结果
    assert!(pending_prompt_from(&serde_json::json!({ "content": "ok" })).is_none());
}

/// **跨工具契约**：`agent_run` 把子会话审批转成载荷回传时，本层必须认出它。
///
/// 两个文件各自演化时这条契约最容易静默断掉：工具改了字段名，编排层就再也识别
/// 不出「待用户动作」，审批卡直接消失（且不报错）。形状与
/// `agent/host/subagent.rs::RelayOutcome::Pending` 的序列化逐字对应。
#[test]
fn subagent_pending_payload_is_recognized_as_pending_intent() {
    let payload = serde_json::json!({
        "content": "需要确认：删除文件",
        "success": false,
        "failure_kind": symbio_kernel::capability::failure_kind::NEEDS_APPROVAL,
        "prompt": { "tool_name": "agent_run", "args": { "session_id": "child-1" } },
    });
    let p = pending_prompt_from(&payload).expect("子会话待审批载荷必须被识别");

    assert_eq!(p.text, "需要确认：删除文件");
    assert_eq!(p.prompt["tool_name"], serde_json::json!("agent_run"));
    assert_eq!(
        p.failure_kind,
        symbio_kernel::capability::failure_kind::NEEDS_APPROVAL
    );
    // 载荷里没有身份字段 ⇒ [`PendingPrompt`] 无法承载悬空引用（类型层面成立）
}

// ==================== ToolPort 的执行面（B1 + B3 经真路径）====================

/// 夹具能力：按 canonical 名注册，返回固定 JSON（content / pending 形态可配）。
struct EchoTool {
    meta: CapabilityMeta,
    data: serde_json::Value,
}

#[async_trait::async_trait]
impl Capability for EchoTool {
    fn meta(&self) -> CapabilityMeta {
        self.meta.clone()
    }

    async fn execute(
        &self,
        _args: serde_json::Value,
        _env: &symbio_kernel::exec::ExecEnv,
        _ctx: std::sync::Arc<dyn symbio_kernel::plugin::PluginInvokeRequest>,
    ) -> Result<serde_json::Value, symbio_kernel::plugin::PluginError> {
        Ok(self.data.clone())
    }
}

/// 带一条工具行的请求上下文（`CAPABILITY_VISITOR` 是 ToolPort 的执行期数据源）。
async fn ctx_with_tool(
    name: &str,
    data: serde_json::Value,
) -> std::sync::Arc<dyn symbio_kernel::plugin::PluginInvokeRequest> {
    let req = PluginSimpleRequest::new(None, None);
    let visitor = std::sync::Arc::new(DefaultToolVisitor::new());
    visitor
        .put(CapabilityRow::tool(std::sync::Arc::new(EchoTool {
            meta: CapabilityMeta {
                name: name.to_string(),
                ..Default::default()
            },
            data,
        })
            as std::sync::Arc<dyn Capability>))
        .await;
    req.set(
        symbio_sdk::capability::CAPABILITY_VISITOR,
        visitor as std::sync::Arc<dyn Registry>,
    );
    std::sync::Arc::new(req)
}

/// 无路由的宿主插件：命中的调用不需要 route；未命名的回落由它给出诚实 NotFound。
struct NopPlugin;

#[async_trait::async_trait]
impl symbio_kernel::plugin::Plugin for NopPlugin {
    fn meta(&self) -> symbio_kernel::plugin::PluginMeta {
        symbio_kernel::plugin::PluginMeta::new("nop", "Nop")
    }

    async fn route(
        self: std::sync::Arc<Self>,
        _ctx: std::sync::Arc<dyn symbio_kernel::plugin::PluginInvokeRequest>,
    ) -> Result<symbio_kernel::plugin::PluginPayload, symbio_kernel::plugin::PluginError> {
        Err(symbio_kernel::plugin::PluginError::NotFound("nop".into()))
    }

    async fn traverse(
        self: std::sync::Arc<Self>,
        _path: String,
        _ctx: std::sync::Arc<dyn symbio_kernel::plugin::PluginInvokeRequest>,
    ) -> Result<symbio_kernel::plugin::PluginPayload, symbio_kernel::plugin::PluginError> {
        Err(symbio_kernel::plugin::PluginError::NotFound("nop".into()))
    }
}

/// 一次执行的最小现场（parent 在场——生产 `ToolPort` 最先查宿主）。
fn call(
    tool_name: &str,
    ctx: std::sync::Arc<dyn symbio_kernel::plugin::PluginInvokeRequest>,
) -> ToolCallRequest {
    ToolCallRequest {
        parent: Some(std::sync::Arc::new(NopPlugin)),
        tool_name: tool_name.to_string(),
        args: serde_json::json!({}),
        tool_call_id: "tc-1".to_string(),
        sink: ExecEventSink::silent(),
        abort: ExecAbortSignal::new(),
        result_msg_id: "res-1".to_string(),
        ctx,
    }
}

/// 字面名经能力表命中：结果契约（content 判据）在执行出口上成立。
#[tokio::test]
async fn executes_through_the_capability_table() {
    let ctx = ctx_with_tool("echo", serde_json::json!({ "content": "done" })).await;
    let out = ToolPort.execute(call("echo", ctx)).await;
    assert_eq!(out.result_text, "done");
    assert!(out.success);
    assert!(out.pending.is_none());
}

/// 线上名解析（B1）：`mcp__fs__read` 认回 `mcp.fs.read`——判据是「注册表里谁映射到
/// 这个名字」，不是把名字反演回去。
#[tokio::test]
async fn wire_names_resolve_to_canonical_capabilities() {
    let ctx = ctx_with_tool("mcp.fs.read", serde_json::json!({ "content": "ok" })).await;
    let out = ToolPort.execute(call("mcp__fs__read", ctx)).await;
    assert_eq!(
        out.result_text, "ok",
        "线上名必须解析到 canonical 名并命中表（而不是回落 route 的 NotFound）"
    );
    assert!(out.success);
}

/// 表里没有的名字**不猜**：按原样交给 route 回落，由它给出诚实的 NotFound——
/// 反演猜错会调起另一个工具，那比报错坏得多。无 parent 的 route 回落即报
/// 「No parent plugin」，同样是诚实的失败。
#[tokio::test]
async fn unknown_names_do_not_get_guessed() {
    let ctx = ctx_with_tool("echo", serde_json::json!({ "content": "done" })).await;
    let out = ToolPort.execute(call("totally_unknown", ctx)).await;
    assert!(!out.success, "未知名不得被解析成别的工具");
    assert!(
        out.result_text.starts_with("Error:"),
        "诚实失败（实际: {}）",
        out.result_text
    );
}

/// 「等待用户」意图经执行出口原样透传（B3 的 pending 半边）。
#[tokio::test]
async fn pending_intent_travels_through_the_outcome() {
    let ctx = ctx_with_tool(
        "confirm_tool",
        serde_json::json!({
            "content": "需要确认：删除文件",
            "failure_kind": "needs_approval",
            "prompt": { "kind": "confirm" },
        }),
    )
    .await;
    let out = ToolPort.execute(call("confirm_tool", ctx)).await;
    let pending = out
        .pending
        .expect("pending 闭集内的 failure_kind 必须被认出");
    assert_eq!(pending.failure_kind, "needs_approval");
    assert_eq!(pending.text, "需要确认：删除文件");
    assert_eq!(pending.prompt["kind"], serde_json::json!("confirm"));
}
