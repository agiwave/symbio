//! `process_tool_calls_async` 的**阶段拆分层**（/ [DE-021 §5 动作 B](../../../../docs/003-design/021-横向扩充与插件切分.md "DE-021-横向扩充与插件切分") B1）。
//!
//! 三个巨型函数按**处理阶段**（不按职责）拆分，本层是第一批 `process_tool_calls_async`
//! （原 714 行单体）的三段落点：
//!
//! - [`gates`] —— ① 批首闸门读数：`mode` / 熔断 / 自声明位 / 主体与控制面结论 /
//!   本批留底 / 同参台账，一次读齐给整批复用（产出 [`gates::GateReadings`]）。
//! - [`batch`] —— ② 逐条执行：`for tc in tool_calls` 的顺序流程，含协议失败、宿主闸 /
//!   声明闸拒止、钩子拦下与正常执行四条出口。
//! - [`unfinished`] —— ③ 未执行收口：本批没跑到的调用统一补父节点终态与结果子节点。
//!
//! 编排入口仍是 `tool_executor::process_tool_calls_async`
//! （签名与 `tools` 门面不变），它只做：空批早退 → ① → ② → ③。
//!
//! **纯结构拆分纪律**（DE-021 §5 纪律 2）：三段代码逐字来自拆分前的单体函数体，
//! 拆分批次内 `if` 条件零变化；段间只传值（[`gates::GateReadings`] 与出参
//! `effects` / `breaks`）。
//!
//! 每段一条判据（纪律 3，「这段错了谁会红」）：
//! - ① 错 ⇒ 闸门判据读数用例红（`breaker` 的批首读场景、`batch_principal` 的派生用例）；
//! - ② 错 ⇒ 批内事件与父节点终态用例红（`tool_executor.test.rs` 的
//!   `process_tool_calls_async` 组：中止、交互 break、闸门拒止、协议失败）；
//! - ③ 错 ⇒ 未执行收口用例红（`not_executed` / `aborted` 结果子节点与父终态断言）。

pub(crate) mod batch;
pub(crate) mod gates;
pub(crate) mod parallel;
pub(crate) mod unfinished;

use super::tool_executor::{
    build_user_prompt_message, call_signature, execute_tool_async, record_effect, BreakerInputs,
    PendingPrompt, ToolDecls,
};
use super::tool_result_guard::{guard_tool_result, DEFAULT_TOOL_RESULT_TOKEN_CAP};
use serde_json::{json, Value};
use std::sync::Arc;
use symbio_core::node_build::{llm_build_tool_message, tool_call_args_text};
use symbio_core::{
    chat_node::{ChatNode, MessageContent, NodeKind, NodeStatus, CHAT_STYLE_TAG_TOOL},
    failure_kind, llm_emit_message, llm_emit_state, llm_short_id, Event, ExecAbortSignal,
    ExecEventSink, Plugin, PluginInvokeRequest, PluginInvokeRequestExt, Seq, Store,
    TurnToolCallInfo,
};
use symbio_plugin_api::effects::EffectInput;
use symbio_plugin_api::hooks::{fire_hook, HookEvent};
use {symbio_core::plugin_error, symbio_core::plugin_info, symbio_core::plugin_warn};

pub(crate) use self::gates::GateReadings;
// 闸门表与拒止骨架：阶段②按 `first_refusal` 判、经 `refuse` 收口。
pub(crate) use self::gates::{first_refusal, refuse, GateContext, Refusal};
// 批内执行策略：风险档 → 并行/串行，映射单点在 `parallel::policy_for_risk`。
pub(crate) use self::parallel::{policy_for_port, ExecPolicy, ToolRisks, MAX_IN_FLIGHT};

// ── 共用收口原语（逐字来自拆分前的 tool_executor.rs）────────────────────────
/// 把父 ToolCall 的**完整副本**定稿为终态并广播。
///
/// 完整快照纪律：从权威转写取副本（id / 父子关系 / name / 参数都在）→
/// 应用状态、合并 meta、设置 error → 整条广播。找不到 = 协议违例
/// （ToolCallDelta 必然先广播过完整节点），报错并返回 `None`——调用方跳过
/// 父状态帧，结果子节点照常给出。返回值是已广播的完整消息，调用方把它
/// 收进 `parent_updates` 供内存镜像同步与落库（整条替换，非补丁合并）。
async fn emit_parent_finalized(
    sink: &ExecEventSink,
    context_messages: &[ChatNode],
    tool_call_id: &str,
    status: NodeStatus,
    meta_extra: Value,
    error: Option<String>,
) -> Option<ChatNode> {
    let Some(parent) = context_messages.iter().find(|m| m.id == tool_call_id) else {
        plugin_error!(
            "session",
            "[Tool] 终态广播: 转写中不存在工具调用 {}（协议违例），跳过父状态帧",
            tool_call_id
        );
        return None;
    };
    let mut full = parent.clone();
    full.status = Some(status);
    // meta 在**发射端**并入权威副本（读-改-写发生在状态所有者处），
    // 广出去的 meta 因此是完整对象，接收端只做整条替换。
    let mut meta = full.meta.clone().unwrap_or_else(|| json!({}));
    if let (Some(obj), Some(new)) = (meta.as_object_mut(), meta_extra.as_object()) {
        for (k, v) in new {
            obj.insert(k.clone(), v.clone());
        }
    } else {
        meta = meta_extra;
    }
    full.meta = Some(meta);
    full.error = error;
    // 终态帧只带状态 / 元数据 / 错误，**不带正文**：ToolCall 的参数正文已在
    // 流式阶段以 delta 逐帧上线。
    llm_emit_state(sink, full.clone()).await;
    Some(full)
}
