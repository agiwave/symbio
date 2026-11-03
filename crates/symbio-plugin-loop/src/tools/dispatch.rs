//! 工具批执行的**阶段分层**。
//!
//! 两个函数按**处理阶段**（不按职责）分开：
//!
//! - [`gates`] —— 拒止骨架与外部执行的**中性输入**（`GateInputs`）：没有批首读数，
//!   读数按每一笔债在准备点现取；判定（快慢 / 预算 / 开窗）在执行体。
//! - [`batch`] —— 逐条执行：`for tc in tool_calls` 的顺序流程，含协议失败、声明闸
//!   拒止、钩子拦下与正常执行四条出口。
//!
//! 「没跑到」的占位结果子节点不在这里收口：节点是事实的投影，环只写
//! `failed{why}`，展示由投影器渲染（DE-047 D-312）。
//!
//! 编排入口是 [`crate::tools::tool_executor::process_tool_calls_in_env`]：
//! 空批早退 → 逐条准备 → 逐条执行。段间只传出参 `effects` / `breaks`。
//!
//! 每段一条判据（纪律 3，「这段错了谁会红」）：
//! - `gates` 错 ⇒ 拒止出口语义 / 外部执行闸用例红（`gates.test.rs`）；
//! - `batch` 错 ⇒ 批内事件与父节点终态用例红（`tool_executor.test.rs` 的
//!   `process_tool_calls_in_env` 组：中止、交互 break、闸门拒止、协议失败）。

pub(crate) mod batch;
pub(crate) mod gates;

/// 中止的成因词：与 [`NOT_EXECUTED`] 同属「没跑到」的词表。
pub const ABORTED: &str = "aborted";
/// 未执行的成因词：租约超时 / 等待用户前没派到的调用由环写这条 `failed` 事实。
/// 占位结果子项由投影器按该事实渲染，环不再写节点（DE-047 D-312）。
pub const NOT_EXECUTED: &str = "not_executed";

/// 这笔结果是否属于「没跑到」（中止 / 未执行）——决定关账写 `failed` 还是 `response`。
pub fn is_unanswered(failure_kind: &str) -> bool {
    failure_kind == ABORTED || failure_kind == NOT_EXECUTED
}

use super::tool_executor::{
    build_user_prompt_message, call_signature, execute_one, record_effect, ExecResult, GateInputs,
    RoundEnv, ToolDecls,
};
use serde_json::{json, Value};
use std::sync::Arc;
use symbio_kernel::event::{Event, Seq};
use symbio_kernel::exec::{ExecAbortSignal, ExecEventSink};
use symbio_kernel::llm::TurnToolCallInfo;
use symbio_kernel::plugin::{Plugin, PluginInvokeRequest, PluginInvokeRequestExt};
use symbio_kernel::store::Store;
use symbio_plugin_api::effects::EffectInput;
use symbio_plugin_api::hooks::{fire_hook, HookEvent};
use symbio_sdk::chat::chat_node::{
    ChatNode, MessageContent, NodeKind, NodeStatus, CHAT_STYLE_TAG_TOOL,
};
use symbio_sdk::llm::llm_emit_state;
use symbio_sdk::llm::turn::{llm_emit_message, llm_short_id};
use symbio_sdk::turn::node_build::{llm_build_tool_message, tool_call_args_text};
use {
    symbio_plugin_api::plugin_error, symbio_plugin_api::plugin_info, symbio_plugin_api::plugin_warn,
};

// 拒止骨架：声明闸与 PreToolUse 钩子经 `refuse` 收口。
pub(crate) use self::gates::{batch_principal, refuse, Refusal};

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
