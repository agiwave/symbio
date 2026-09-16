//! SESSION 聊天主循环
//!
//! 本文件只保留**主循环骨架**，其余按职责拆到子模块
//! （切分依据见 `docs/module-layout.md` §3.1）：
//!
//! ```text
//! 前步骤 ① 请求快照 ② 开会话 ③ TurnState+容器 ④ resume
//! loop
//!   步骤1 加载上下文
//!   步骤2 gate_turn            ← 启动条件 + 退出条件（唯一判定点）
//!   步骤3 prepare_turn_inputs  ← 提示词 + 工具 + 压缩 + 请求视图（唯一收集点）
//!   步骤4 execute_turn         ← LLM 调用
//!   步骤5 settle_reasoning     ← 推理产物并入上下文
//!   步骤6 close_turn           ← 工具分发 + 落库 + 流向判定
//! end loop
//! 后步骤 无（循环只能由 return 离开，全部经 finish_turn 收尾）
//! ```
//!
//! 子模块分工：
//! - [`state`]    会话上下文 / 请求快照 / 单轮状态 / 闸门结果 / 退出原因 / 编排器
//! - [`inputs`]   收口 ②③：提示词与工具的唯一收集点、压缩的唯一响应点
//! - [`turn`]     单轮收尾：推理并入 → 工具分发 → 落库 → 流向
//! - [`compress`] 压缩流水线（自动语义压缩与主动 context_compact 共用内核）
//! - [`io`]       副作用出口：落库 / 广播 / 流式占位 / 开会话 / 生命周期钩子
//!
//! 设计说明：
//! - 统一从会话服务获取消息历史，不区分有状态/无状态协议
//! - 具体协议实现层决定如何使用这些历史（有状态协议可能只使用部分或不使用）
//! - 请求中只包含当前要发送的单条消息（single_message）

mod compress;
mod inputs;
mod io;
mod state;
mod turn;

// 跨模块契约：`orchestrator.rs` / `resume.rs` 经 `chat_loop::X` 引用。
pub use self::state::{ChatOrchestrator, StopSignal};

// 模块内共享面：子模块经 `use super::*;` 取用，测试亦同（`gate_tests` 等）。
pub(crate) use self::compress::{auto_compress_process, run_context_compact};
pub(crate) use self::inputs::prepare_turn_inputs;
pub(crate) use self::io::{
    broadcast_message_update, emit_streaming_start, fire_stop_hook, fire_user_prompt_submit_hook,
    open_chat_session, persist_messages,
};
pub(crate) use self::state::{Gate, SessionContext, TurnExit, TurnRequest, TurnResult, TurnState};
pub(crate) use self::turn::{close_turn, settle_reasoning};

use super::chat_session::{ChatSession, PersistentChatSession, SESSION_HANDLE};
use super::model_chat;
use crate::plugin_info;
use crate::plugin_warn;
use crate::symbio_core::schemas::{
    session::chat_message::{ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType},
    HookEvent,
};
use crate::symbio_core::turn::{
    build_tool_message, emit_status, emit_update, short_id, ToolCallInfo, TurnOutput,
};
use crate::symbio_core::FinishReason;
use crate::symbio_core::{
    CapabilityMeta, InvokeRequest, InvokeRequestExt, ModelProvider, Plugin, PluginChannel,
    PluginError, PluginFrame, Usage,
};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::compression;
use super::tool_executor::{fire_hook, process_tool_calls_async};
use crate::symbio_core::schemas::session::session_chat_response;
use crate::symbio_core::schemas::session::session_config::SessionConfig;

pub async fn run_chat_loop(
    orchestrator: &ChatOrchestrator,
    ctx: Arc<dyn InvokeRequest>,
    mut channel: PluginChannel,
) -> Result<(), PluginError> {
    // ── 前步骤 ①：请求解析与请求级配置快照 ─────────────────────────────────
    let mut req: model_chat::Request = ctx.payload()?;
    // 请求体未给出的字段一律回落到 `SessionConfig::default()`，本函数不再自带魔法数
    // ——默认值的唯一真源在配置层（审计 C2）。历史上这里是 `unwrap_or(true)` /
    // `unwrap_or(false)` / `unwrap_or(15)` 三份影子默认值，与配置默认值恰好相等纯属
    // 巧合，改配置会静默失效。
    let turn_req = TurnRequest::new(&req);

    plugin_info!(
        "session",
        ">>> NEW SESSION START (Protocol: {:?})",
        orchestrator.provider.api_protocol()
    );

    // ── 前步骤 ②：会话引擎 ────────────────────────────────────────────────
    let session = open_chat_session(&ctx).await;

    // ── 前步骤 ③：轮次状态 + 上下文容器 ───────────────────────────────────
    let mut turn = TurnState {
        abort_flag: Arc::new(AtomicBool::new(false)),
        ..Default::default()
    };
    let mut single_message = req.single_message.take();
    let mut context = SessionContext {
        messages: Vec::new(),
        session,
    };

    // ── 会话恢复（resume）：在 turn 循环前处理 ──────────────────────────────
    //
    // 当 `req.resume` 存在时，本调用是用户的恢复操作：
    // - `RetryTurn`：LLM 失败重试，删除 Failed Turn 及其所有子节点，重新走 LLM 请求
    // - `Retry`/`Approve`/`Reject`/`Supply`/`Answer`：工具调用恢复，删除旧子节点、
    //   重新执行工具、创建新结果子节点并持久化。CAPABILITY_VISITOR 已由 agent chat
    //   handler 设置，`execute_tool_async` 直接复用。
    //
    // 成功 → `Continue`：turn 循环从 session 加载含新工具结果的历史，续写 LLM
    //   （RetryTurn 也走此路径，但因为是删除整个 Failed Turn 后重新请求，等价于普通 send）。
    // 失败/reject/answer → `Done`：退出循环，留 Failed/Completed 等下次 resume。
    if let Some(tr) = req.resume.take() {
        match crate::plugins::session::resume::process_resume(
            orchestrator,
            &ctx,
            &mut channel,
            &turn.abort_flag,
            &context.session,
            tr,
        )
        .await
        {
            Ok(crate::plugins::session::resume::ResumeOutcome::Continue) => {
                // 成功：turn 循环会从 session 加载含新工具结果的历史
            }
            Ok(crate::plugins::session::resume::ResumeOutcome::Done) => {
                return finish_turn(
                    orchestrator,
                    &context,
                    &channel,
                    &turn,
                    TurnExit::ResumeDone,
                )
                .await;
            }
            Err(e) => {
                plugin_warn!("session", "[Resume] process_resume failed: {}", e);
                return finish_turn(orchestrator, &context, &channel, &turn, TurnExit::Failed(e))
                    .await;
            }
        }
    }

    loop {
        // ── 步骤 1：加载本轮上下文 ───────────────────────────────────────────
        // 每轮开始时从 ChatSession 获取最新上下文（轮次窗口生效；骨架化/淡化为请求
        // 视图层职责）。心跳任务等场景可设置 `load_history = false`：仅用本次
        // single_message，完全不加载历史，也不保留上一轮内存累积（随本轮重置）。
        context.messages = if turn_req.load_history {
            context
                .session
                .get_context_messages(None)
                .await
                .unwrap_or_default()
        } else {
            // 心跳等无上下文场景：从空上下文开始，仅携带本轮新消息。
            Vec::new()
        };

        // 首轮追加当前用户消息（去重：避免与存储中已持久化的消息重复）
        // resume 时 single_message=None，不应触发 user_prompt_submit_hook
        //（否则会把工具结果当 user_text 传给 hook，产生错误副作用）。
        if turn.tool_rounds == 0 {
            let mut had_user_msg = false;
            if let Some(msg) = single_message.take() {
                if !context.messages.iter().any(|m| m.id == msg.id) {
                    context.messages.push(msg);
                }
                had_user_msg = true;
            }
            if had_user_msg {
                fire_user_prompt_submit_hook(orchestrator, &context, &ctx).await;
            }
        }

        turn.last_saved = context.messages.len();

        // ── 步骤 2：闸门（启动条件 + 退出条件，唯一判定点）───────────────────
        // 收口前，这里散落 4 处 `if abort_flag { ... return }`（其中两处相邻重复）
        // 与一处软上限判定，各自重写收尾三件套。
        match gate_turn(&turn_req, &turn) {
            Gate::Proceed => {}
            Gate::WaitForTools => {
                // 不完整不唤醒：本轮不发起 LLM 请求。级别 1（同步工具执行）下恒不
                // 命中——工具在 close_turn 内全部跑完才回到这里。级别 2 会在此
                // await 全部完成的通知后 continue；当前实现退出本轮，由下一次唤醒
                // （用户消息 / 心跳）重新入闸。
                plugin_warn!(
                    "session",
                    "[Gate] 在途工具未全部产出结果（{} 个），本轮不唤醒",
                    turn.in_flight_tools.len()
                );
                return finish_turn(orchestrator, &context, &channel, &turn, TurnExit::Completed)
                    .await;
            }
            Gate::Exit(exit) => {
                return finish_turn(orchestrator, &context, &channel, &turn, exit).await
            }
        }

        plugin_info!(
            "session",
            "--- TURN {} START --- (msgs={}, tools={})",
            turn.tool_rounds,
            context.messages.len(),
            0
        );

        // ── 步骤 3：本轮输入准备（收口 ②，唯一准备点）────────────────────────
        // 系统提示词与工具在同一函数内、同一时刻收集（同一个 CapabilityVisitor），
        // 并由此派生请求级开销与请求视图；自动压缩、水位提醒一并收口在该函数内。
        let inputs = match prepare_turn_inputs(
            orchestrator,
            &ctx,
            &mut context,
            &mut channel,
            &mut turn,
            &turn_req,
        )
        .await
        {
            Ok(inputs) => inputs,
            Err(exit) => {
                return finish_turn(orchestrator, &context, &channel, &turn, exit).await;
            }
        };

        // Turn 创建后的首个 abort 检查点：覆盖"压缩阶段中止"等 send_request
        // 之前置位的场景。压缩失败已就地降级（不冒泡），但 abort_flag 仍为
        // true 且 abort 帧已被压缩请求消费——若不在此拦截，execute_turn 会
        // 发起一次多余的 LLM 请求。此处 Turn 已创建（上方 emit_streaming_start），
        // 冒泡 Err(Aborted) → 消费循环 ABORTED 分支 → persist_failure 把本轮
        // Turn 收尾为 Failed + "用户手动中止了本次回复"（错误条 + 重试入口），
        // 不会波及上一轮已成功的 Turn（persist_failure 按 failing_turn 子树收窄）。
        if turn.abort_flag.load(Ordering::SeqCst) {
            return finish_turn(orchestrator, &context, &channel, &turn, TurnExit::Aborted).await;
        }

        let result = orchestrator
            .provider
            .execute_turn(
                &inputs.system_prompt,
                &inputs.request_view,
                &inputs.tools,
                &inputs.root_id,
                &mut channel,
                &turn.abort_flag,
            )
            .await;

        let out = match result {
            Err(PluginError::RetryWithoutContextId) => {
                for m in &mut context.messages {
                    m.response_id = None;
                }
                // 清除本轮未完成的 Streaming 半截内容（来自上一轮被中断的 LLM 流），
                // 避免下轮 get_context_messages 加载到半截消息污染 LLM 上下文。
                // RetryWithoutContextId 表示 LLM 提供商返回的 context_id 无效（会话不存在），
                // 本轮流式产出的 Streaming 节点都是无效半截响应，应直接删除而非保留为 Failed 终态。
                context
                    .messages
                    .retain(|m| m.status != Some(MessageStatus::Streaming));
                // 持久化清除后的 response_id 与 Streaming 删除结果，确保下轮加载到干净版本
                let _ = context
                    .session
                    .replace_messages(context.messages.clone())
                    .await;
                continue;
            }
            Err(PluginError::Aborted) => {
                // 用户手动中止：向上冒泡 Err(Aborted)，由消费循环识别 code=ABORTED
                // 后走 persist_failure —— 在途 Turn 持久化为 Failed + error，前端
                // 可渲染错误条与重试入口；若在此直接返回 Ok，在途 Turn 不落库，
                // 刷新即消失且无重试入口。
                return finish_turn(orchestrator, &context, &channel, &turn, TurnExit::Aborted)
                    .await;
            }
            Err(e) => {
                plugin_warn!("session", "send_request failed: {e}");
                return finish_turn(orchestrator, &context, &channel, &turn, TurnExit::Failed(e))
                    .await;
            }
            Ok(out) => out,
        };

        if turn.abort_flag.load(Ordering::SeqCst) {
            // 与 Err(PluginError::Aborted) 分支同理：请求结束后才置位的 abort 标志
            // 同样向上冒泡，由消费循环统一收尾（在途 Turn → Failed + error + 可重试）。
            // 仅 send_request 之后的 abort 冒泡；turn 循环顶部的边界检查点不冒泡——
            // 上一轮已定稿落库，冒泡会把成功的 Turn 误回滚为 Failed。
            return finish_turn(orchestrator, &context, &channel, &turn, TurnExit::Aborted).await;
        }

        // ── 步骤 3：推理收尾（定格子节点 → 校准估算 → 并入上下文）────────────
        let result =
            settle_reasoning(orchestrator, &mut context, &channel, &inputs.root_id, out).await;

        // ── 步骤 4：本轮结算 + 下一步判定 ───────────────────────────────────
        match close_turn(
            orchestrator,
            ctx.clone(),
            &mut channel,
            &mut context,
            &mut turn,
            result,
            &turn_req,
        )
        .await
        {
            TurnFlow::NextTurn => {}
            TurnFlow::Finish(exit) => {
                return finish_turn(orchestrator, &context, &channel, &turn, exit).await
            }
        }
    }
}

/// `run_chat_loop` 单轮的流向。
///
/// - `Finish`：本轮即请求终态，携带退出原因（收尾由 [`finish_turn`] 统一执行）
/// - `NextTurn`：工具轮结束，进入下一轮 LLM 请求
pub(crate) enum TurnFlow {
    Finish(TurnExit),
    NextTurn,
}

/// 主循环顶部的**唯一闸门**：启动条件 + 退出条件。
///
/// 收口前，循环顶部与推理前后散落 4 处 `if abort_flag { ... return }`（其中两处
/// 相邻重复，中间只隔一行日志），软上限判定另占一处，每处各自重写收尾三件套。
/// 现在：**退出条件只在本函数判定一次**，收尾动作只在 [`finish_turn`] 出现一次。
///
/// ## 启动条件：本轮在途工具是否已全部产出结果
///
/// 语义是「多个工具调用全部结束后才唤醒主循环，不完整不唤醒」。判据取自
/// [`TurnState::in_flight_tools`]（运行时在途集合），**不是**"历史中所有
/// ToolCall 都有结果子节点"——
///
/// ⚠️ 后者在本代码库并不成立：「ToolCall 无结果」是**合法状态**。交互模式下工具批
/// 被用户审批中断时，本批剩余 ToolCall 会被有意留空，由请求视图层
/// `flatten_chat_messages`（`plugins/model/message_builder.rs`）合成占位 tool 结果
/// 喂回模型。按历史判定会让 approve/reject 恢复后的续写被永久挡住。
///
/// 级别 1（当前，同步工具执行）：`close_turn` 内的 `process_tool_calls_async` 会把
/// 本批工具全部跑完才返回，回到闸门时在途集合恒为空 ⇒ 恒放行。
/// 级别 2（异步工具调用）：`settle_turn` 把每个工具 spawn 出去并登记 id，完成回调
/// 逐个移除；集合非空时本函数返回 [`Gate::WaitForTools`]。
fn gate_turn(req: &TurnRequest, turn: &TurnState) -> Gate {
    // ── 启动条件 ─────────────────────────────────────────────────────────
    if !turn.in_flight_tools.is_empty() {
        return Gate::WaitForTools;
    }

    // ── 退出条件 ①：显式软上限 ────────────────────────────────────────────
    // 仅当调用方**主动**给出 max_tool_rounds 才生效（默认 None = 无限轮次）。
    // 达到上限时必须给出明确提示再退出，绝不静默返回。
    if let Some(max) = req.max_tool_rounds {
        if turn.tool_rounds >= max {
            return Gate::Exit(TurnExit::MaxToolRounds { max });
        }
    }

    // ── 退出条件 ②：用户中止（边界检查点）─────────────────────────────────
    // 循环顶部的中止检查点**不冒泡** `Err(Aborted)`：上一轮 Turn 已定稿落库，
    // 冒泡会让消费循环的 `persist_failure` 把成功的 Turn 误回滚为 Failed。
    // 推理前后（在途 Turn 尚未定稿）的中止才走 [`TurnExit::Aborted`]。
    if turn.abort_flag.load(Ordering::SeqCst) {
        return Gate::Exit(TurnExit::AbortedAtBoundary);
    }

    Gate::Proceed
}

/// 主循环的**唯一收尾点**：软上限提示 → 增量落库 → Stop 钩子 → 返回语义。
///
/// 收口前，12 个出口各自重写「Stop 钩子 + 落库 + 返回」三件套，新增出口必须记得
/// 补齐（漏了只有 `StopSignal` 的 RAII 兜底能发现）。现在每个出口只负责判定
/// [`TurnExit`]，收尾动作只在本函数出现一次。
///
/// `persist_messages` 在锚点已对齐时是 no-op（切片为空即返回），因此所有出口都可以
/// 无条件调用——不会重复落库：`close_turn` 与压缩流程每次落库后都会同步推进
/// [`TurnState::last_saved`]。
async fn finish_turn(
    orchestrator: &ChatOrchestrator,
    context: &SessionContext,
    channel: &PluginChannel,
    turn: &TurnState,
    exit: TurnExit,
) -> Result<(), PluginError> {
    // 软上限：先广播明确提示再退出，绝不静默（文案唯一出处）。
    if let TurnExit::MaxToolRounds { max } = &exit {
        let _ = channel
            .tx
            .send(PluginFrame::Data(
                serde_json::to_value(session_chat_response::StreamEvent::Error {
                    error: format!("已达到本轮工具调用上限（{max}）。如需继续，请再次发送消息。"),
                })
                .unwrap_or_default(),
            ))
            .await;
    }

    // 增量落库（锚点已对齐时为空切片，天然 no-op）。
    persist_messages(context, turn.last_saved, channel).await;

    // Stop 钩子（幂等）：resume 出口发生在主循环之前，此时尚无消息列表
    // （收口前该分支即传空切片），其余出口一律携带当前消息列表。
    let messages: &[ChatMessage] = match exit {
        TurnExit::ResumeDone => &[],
        _ => &context.messages,
    };
    fire_stop_hook(orchestrator, messages).await;

    // 返回语义：中止与失败向上冒泡，由消费循环统一收尾（在途 Turn → Failed + 可重试）。
    match exit {
        TurnExit::Aborted => Err(PluginError::Aborted),
        TurnExit::Failed(e) => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests;
