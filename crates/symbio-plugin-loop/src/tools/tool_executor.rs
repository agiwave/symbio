//! 工具执行层
//!
//! 负责：
//! - 单笔债的端口取用（`execute_one`：按端口名拿执行合同——
//!   线上名解析、空闲超时、结果守卫、产物格都住提供者一侧（session 的 `ToolPort`），
//!   本层不再认识它们）
//! - 批量工具调用处理与结果广播（`process_tool_calls_in_env`）
//!
//! 设计说明（会话激活/恢复状态机）：
//! - 本层**不阻塞**等待任何用户输入。需要用户确认（confirm）或主动询问（ask_user）
//!   的工具会自行产出 `user_prompt` 消息节点并标记 `WaitingUserAction`，由编排层
//!   （chat_loop）在本轮结束时将会话置于 `AwaitingInput(user)`；用户答案以一条普通
//!   `user` 消息回填后，新一轮会重跑该工具。详见 USER_INPUT_MECHANISM 设计文档。

use symbio_kernel::llm::TurnToolCallInfo;
use symbio_kernel::plugin::PLUGIN_PARENT;

use serde_json::{json, Value};
use std::sync::Arc;
use symbio_kernel::event::Event;
use symbio_kernel::exec::{ExecAbortSignal, ExecEventSink};
use symbio_kernel::plugin::{Plugin, PluginInvokeRequest, PluginInvokeRequestExt};
use symbio_kernel::store::Store;
use symbio_plugin_api::plugin_error;
use symbio_sdk::chat::chat_node::{ChatNode, MessageContent, NodeKind, NodeStatus};

mod decls;
mod effects;
mod gate_inputs;

pub use self::gate_inputs::GateInputs;
pub use self::decls::ToolDecls;
pub use self::effects::record_effect;
use symbio_plugin_api::effects::EffectInput;
// tool 端口的合同类型（下沉到 plugin-api）：`PendingPrompt` 经本模块 re-export，
// dispatch 一系（`dispatch.rs` / `dispatch/batch.rs`）与测试的导入面不变。
pub use symbio_plugin_api::port::PendingPrompt;
use symbio_plugin_api::port::{PortExecContext, PortInput};
use symbio_plugin_api::ports;

/// 同参重复调用的判据键：`(工具名, 参数文本)`。
///
/// 参数取 `Value::to_string()`（serde_json 对象按键序规范化），于是模型两次发出
/// 的同一调用——空白与键序不同也无妨——落到同一个键上。
pub(super) fn call_signature(name: &str, args: &str) -> String {
    format!("{name}\u{1}{args}")
}

/// 由载荷构造 user_prompt 节点（`WaitingUserAction`）。
///
/// **本文件是工具结果节点的唯一写入者**：普通结果（`llm_build_tool_message`）与
/// 等待用户（本函数）都出自这里，两者共享同一套 id 约定
/// （`id = result_node_id`、`parent_id = tool_call_id`）。
pub(super) fn build_user_prompt_message(
    result_node_id: &str,
    tool_call_id: &str,
    pending: &PendingPrompt,
    tool_name: &str,
) -> ChatNode {
    ChatNode {
        id: result_node_id.to_string(),
        parent_id: Some(tool_call_id.to_string()),
        // 等待用户的交互卡同样是**工具的结果**：族归工具自己。
        // 身份只写这一份——`role` 那格不再跟着写（D-147：主体在场时折叠读主体，
        // 第二份写了也不生效，只会在改写时留下「名实不符」的机会）。
        principal: Some(symbio_sdk::chat::principal::Principal::tool_principal(
            tool_name,
        )),
        msg_type: Some(NodeKind::Interaction),
        content: Some(MessageContent::Text(pending.text.clone())),
        status: Some(NodeStatus::WaitingUserAction),
        meta: Some(json!({
            "prompt": pending.prompt,
            "failure_kind": pending.failure_kind,
            "tool_name": tool_name,
        })),
        ..Default::default()
    }
}

/// 一批工具调用的执行信封（环侧每次派发建一份，逐调用共享）。
///
/// 字段都是环的**通用形式**：轮号 / 根节点 / 归属 / 产物闸结论 / 宿主写口。执行体
/// 不认识世界，只按这些值记账与落格（P1b，DE-047 D-308）。
#[derive(Clone, Default)]
pub struct RoundEnv {
    /// L0 存档目录（超预算结果写全文）。
    pub archive_dir: std::path::PathBuf,
    /// 宿主写口（产物格 / 授权事实都经它落，`D-305` 裁决一）。
    pub emit: symbio_plugin_api::port::PortFactEmit,
    /// 轮号（产物格幂等键的一部分）。
    pub round: u64,
    /// 轮根节点 id（随产物格载荷下发，见 [`Self::allowed`] 的上下文）。
    pub round_id: String,
    /// 归属序号（事实的 `produced_by`）。
    pub produced_by: u64,
    /// 环侧产物闸（`closure_gate`）的结论；执行体据此决定要不要落产物格。
    pub allowed: bool,
    /// 产物闸拒绝的理由（放行时为空）；执行体把它写进幂等的 `authz.denied`。
    pub denied_reason: String,
}

/// 执行体交回的结果：节点正文用已守卫的 [`Self::text`]，记账与钩子用 [`Self::raw`]。
#[derive(Debug, Clone, Default)]
pub struct ExecResult {
    pub text: String,
    pub raw: String,
    pub success: bool,
    pub pending: Option<PendingPrompt>,
    pub truncated: bool,
    pub archive_path: Option<String>,
    pub origin_tokens: usize,
    /// 执行体在结果事实里交回的 `failure_kind`（中性词；环不解释原因）。
    pub failure_kind: Option<String>,
}

/// 执行**一笔债**（不阻塞等待用户）：按端口名取提供者，交给端口执行合同跑。
///
/// 这是环与执行体之间的**通用单笔入口**（DE-047 D-311）：一笔调用一笔债，入参只有
/// 端口名与这笔债的载荷；并发、顺序、熔断全在 provider 内部，本函数不认识它们。
///
/// 本函数只是**端口取用面**：按端口名从能力表读提供者，把调用现场整份交给
/// 端口执行合同，收回文本 + 守卫元数据——线上名怎么解析、挂死怎么判（空闲而不是
/// 总时长）、任意 JSON 怎么压成一段文本、超预算结果怎么裁剪与存档、产物格怎么落，
/// 都是提供者一侧的私事，行为等价由 e2e 全量守着。
///
/// 端口缺席（没装提供者插件——零工具的装配没有工具世界的提供者）或合同缺席
/// （提供者不实现该端口的合同）都**响亮失败**：错误文本回传给模型（失败属信息性，
/// 模型据此改道）、`success = false`、日志出声——与既有 route 错误同形。不许
/// 静默回退、不许按「判不出来」继续：装配缺了提供者，继续转只会把缺失往后推。
#[allow(clippy::too_many_arguments)]
pub async fn execute_one(
    port: &'static str,
    env: &RoundEnv,
    gate: &GateInputs,
    parent: &Option<Arc<dyn Plugin>>,
    name: &str,
    args: Value,
    call_id: &str,
    sink: &ExecEventSink,
    abort: &ExecAbortSignal,
    ctx: Arc<dyn PluginInvokeRequest>,
) -> ExecResult {
    let absent = |why: &str| {
        let text = format!("Error: `port.{port}` 缺席：{why}（{name} 未执行）");
        plugin_error!("session", "[Tool] {text}");
        ExecResult {
            text: text.clone(),
            raw: text,
            success: false,
            ..Default::default()
        }
    };
    let Some(provider) = super::super::port::provider_of(&ctx, port).await else {
        return absent("没有可用的端口提供者");
    };
    let Some(executor) = provider.executor() else {
        return absent("提供者不实现执行合同");
    };

    // 宿主插件必须让提供者拿得到：`ctx.parent()` 是提供者解析失败时的回落面。
    // 在 fork 上写 `PLUGIN_PARENT`，不改调用方共享的那份 ctx。
    let exec_ctx = ctx.fork();
    if let Some(p) = parent {
        exec_ctx.set(PLUGIN_PARENT, Some(Arc::downgrade(p)));
    }
    let out = executor
        .execute(
            PortExecContext {
                deadline_ms: None,
                abort: abort.clone(),
                idempotency_key: call_id.to_string(),
                sink: sink.clone(),
                emit: env.emit.clone(),
                ctx: Some(exec_ctx),
            },
            PortInput {
                port,
                payload: json!({
                    "name": name,
                    "args": args,
                    "call_id": call_id,
                    "archive_dir": env.archive_dir.to_string_lossy(),
                    // 中性输入：开窗判据与熔断事实归执行体（DE-039 §6，D-311）。
                    "gate": {
                        "authorized": gate.authorized,
                        "spent_ms": gate.spent_ms,
                        "requested_ms": gate.requested_ms,
                        "budget_ms": gate.budget_ms,
                    },
                }),
            },
        )
        .await;

    // 端口协议失败与工具自身失败是两件事：前者由 `error` 交回，后者住在
    // `facts` 的 `success=false` 里（工具失败是信息性事实，模型据此自我纠正）。
    if let Some((class, why)) = out.error {
        plugin_error!(
            "session",
            format!("[Tool] 端口执行失败（{class:?}）：{why}")
        );
        let text = format!("Error: {why}");
        return ExecResult {
            text: text.clone(),
            raw: text,
            success: false,
            ..Default::default()
        };
    }
    let fact = out.facts.first().cloned().unwrap_or(Value::Null);
    let result_text = fact
        .get("result_text")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let raw = fact
        .get("raw_text")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| result_text.clone());
    let success = fact
        .get("success")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let pending = fact
        .get("pending")
        .filter(|v| !v.is_null())
        .map(|v| PendingPrompt {
            text: v
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("需要用户操作")
                .to_string(),
            prompt: v.get("prompt").cloned().unwrap_or(Value::Null),
            failure_kind: v
                .get("failure_kind")
                .and_then(|k| k.as_str())
                .unwrap_or_default()
                .to_string(),
        });
    let guard = fact.get("guard").cloned().unwrap_or(Value::Null);
    ExecResult {
        text: result_text,
        raw,
        success,
        pending,
        truncated: guard
            .get("truncated")
            .and_then(|v| v.as_bool())
            .unwrap_or(false),
        archive_path: guard
            .get("archive_path")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        origin_tokens: guard
            .get("origin_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as usize,
        failure_kind: fact
            .get("failure_kind")
            .and_then(|v| v.as_str())
            .filter(|k| !k.is_empty())
            .map(str::to_string),
    }
}

/// 产物落格的执行体出口：只交回**结果纯值**，不再执行工具。
///
/// 唤醒结果与没跑到的占位结果没有一次「执行」可挂，但它们同样是产物；环把结果字段
/// 按通用载荷给出，执行体负责装配产物载荷、富化字段与幂等的拒绝事实（P1b /
/// DE-047 D-308）。环不按世界分支，也不在环里留第二条落格路径。
pub(crate) async fn land_outcome_async(
    env: &RoundEnv,
    parent: &Option<Arc<dyn Plugin>>,
    sink: &ExecEventSink,
    abort: &ExecAbortSignal,
    ctx: Arc<dyn PluginInvokeRequest>,
    outcome: &super::super::tool_dispatch::CallOutcome,
) {
    let Some(provider) = super::super::port::provider_of(&ctx, ports::TOOL).await else {
        return;
    };
    let Some(executor) = provider.executor() else {
        return;
    };
    let exec_ctx = ctx.fork();
    if let Some(p) = parent {
        exec_ctx.set(PLUGIN_PARENT, Some(Arc::downgrade(p)));
    }
    let args = serde_json::from_str(&outcome.args).unwrap_or(Value::Null);
    let _ = executor
        .execute(
            PortExecContext {
                deadline_ms: None,
                abort: abort.clone(),
                idempotency_key: outcome.call_id.clone(),
                sink: sink.clone(),
                emit: env.emit.clone(),
                ctx: Some(exec_ctx),
            },
            PortInput {
                port: ports::TOOL,
                payload: json!({
                    "name": outcome.name,
                    "args": args,
                    "call_id": outcome.call_id,
                    "archive_dir": env.archive_dir.to_string_lossy(),
                    "artifact": {
                        // 键按**这一笔债务**而不是节点身份生成：同一张调用卡的等待态与
                        // 答复态是两笔债（`…-resume-…-1`），各有一次产物；按节点身份
                        // 生成会让后一条因幂等被吞掉（答复卡凭空消失）。
                        "key": format!("a-{}-{}", env.round, outcome.call.id),
                        "round": env.round,
                        "round_id": env.round_id,
                        "produced_by": env.produced_by,
                        "allowed": env.allowed,
                        "denied_reason": env.denied_reason,
                    },
                    "land_only": {
                        "text": outcome.text,
                        "success": outcome.ok,
                        "needs_user_action": outcome.needs_user_action,
                        "prompt": outcome.prompt,
                        "failure_kind": outcome.failure_kind,
                    },
                }),
            },
        )
        .await;
}

// 批量工具调用处理

/// 带执行信封的批量入口：顺序处理一批工具调用（产物格 / 宿主写口 / 轮归属随
/// [`RoundEnv`] 走），向 channel 广播每个工具的结果，
/// 并返回 `(tool_messages, parent_updates)`：
/// - `tool_messages`：工具结果子节点（用于追加到对话历史）
/// - `parent_updates`：ToolCall 父节点的**完整终态快照**（发射端从
///   `context_messages` 取权威副本应用终态，整条广播），供调用方做内存镜像
///   整条替换并随 `persist_messages` 落库。
///
/// 交互模式（interactive）下，若前一个工具产出 user_prompt（待审批/询问）或失败，
/// 则中止本批剩余工具（用户需逐个处理）；auto 模式不中止，失败结果传 LLM 继续。
///
/// ## 四条不变量（本函数负责保证）
///
/// 1. **本批每一个工具调用都必须以终态收场**（`Completed` / `WaitingUserAction`）：
///    调用前广播 `Streaming`（`dispatch::batch::emit_tool_running`），调用后广播终态。
///    「没跑到」的调用由环写 `failed{why}` 事实，投影器按事实渲染调用卡终态与
///    「未执行」结果子项，环不再补节点（DE-047 D-312）。
///
/// 2. **外部执行的授权 / 成本读数在每个调用点各取一次**（[`GateInputs`]），随债交给
///    执行体：判据与开窗不在环——`Refuse` 零事件、`Break` 由执行体自己 `emit`
///    `control × opened`、`Allow` 照旧开窗（DE-039 §6，D-311）。
/// 3. **声明闸由能力包自己判**（DE-025 D-219）：工具在 `CapabilityMeta.behaviors`
///    上声明「我受哪项管辖」，对应实现从
///    `ToolBehavior::before` 回答能不能跑（淘汰 / 任务表写侧即此类）。宿主只交这一笔
///    的结论（主体 / 控制面撤销值 / 会话事实切片），本层不含任何「哪个工具名
///    受什么管辖」的分支。
///
/// 阶段分层见 `dispatch/{gates,batch}`；**没有批首读数**——编排顺序（空批早退 →
/// 逐条准备 → 逐条执行）不变。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn process_tool_calls_in_env(
    tool_calls: Vec<TurnToolCallInfo>,
    parent: &Option<Arc<dyn Plugin>>,
    sink: &ExecEventSink,
    abort: &ExecAbortSignal,
    ctx: Arc<dyn PluginInvokeRequest>,
    context_messages: &[ChatNode],
    env: &RoundEnv,
    facts: &dyn Store<Event = Event>,
    elimination_revoked: bool,
    effects: &mut Vec<EffectInput>,
    breaks: &mut Vec<&'static str>,
) -> (Vec<ChatNode>, Vec<ChatNode>) {
    // 空批不读闸门、不发日志——与拆分前的早退位置逐字一致。
    if tool_calls.is_empty() {
        return (Vec::new(), Vec::new());
    }

    // 逐条执行：批内顺序流程（协议失败 / 闸门拒止 / 钩子拦下 / 正常执行四条出口）。
    // - `tool_messages`：工具结果子节点（用于追加到对话历史）
    // - `parent_updates`：ToolCall 父节点的**完整终态快照**（发射端从
    //   `context_messages` 取权威副本应用终态），供调用方做内存镜像整条替换并随
    //   `persist_messages` 落库。
    let (tool_messages, parent_updates) = super::dispatch::batch::run_batch(
        tool_calls,
        parent,
        sink,
        abort,
        ctx,
        context_messages,
        env,
        facts,
        elimination_revoked,
        effects,
        breaks,
    )
    .await;

    (tool_messages, parent_updates)
}

#[cfg(test)]
#[path = "tool_executor.test.rs"]
mod tests;
