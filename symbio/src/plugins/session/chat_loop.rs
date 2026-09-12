//! SESSION 聊天主循环（自 model 插件迁入，Phase E-②）
//!
//! 职责：
//! - 主循环入口 run_chat_loop
//! - 单轮处理编排
//! - 压缩逻辑处理
//!
//! 设计说明：
//! - 统一从会话服务获取消息历史，不区分有状态/无状态协议
//! - 具体协议实现层决定如何使用这些历史（有状态协议可能只使用部分或不使用）
//! - 请求中只包含当前要发送的单条消息（single_message）

use super::model_chat;
use crate::plugin_info;
use crate::plugin_warn;
use crate::symbio_core::schemas::{
    session::chat_message::{
        assign_seq, max_seq, ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
    },
    HookEvent,
};
use crate::symbio_core::turn::{
    build_tool_message, emit_status, emit_update, short_id, ToolCallInfo, TurnOutput,
};
use crate::symbio_core::{
    ChatSession, InvokeRequest, InvokeRequestExt, ModelProvider, Plugin, PluginChannel,
    PluginError, PluginFrame, SESSION_HANDLE,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::compression;
use super::tool_executor::{fire_hook, process_tool_calls_async};
use crate::symbio_core::schemas::session::session_chat_response;

/// MODEL 会话上下文
///
/// 设计说明：
/// - 仅包含消息列表，不包含 MODEL 请求配置
/// - system_prompt、tools、thinking 等配置应从 model_chat::Request 获取
/// - session 用于管理会话历史（滑动窗口/自动截断/持久化）
struct SessionContext {
    pub messages: Vec<ChatMessage>,
    pub session: Arc<dyn ChatSession>,
}

/// Stop 钩子的幂等触发器（session-mechanism-unification.md §4.4）。
///
/// 契约：**一个请求生命周期内，Stop 恰好触发一次**——无论该生命周期以何种方式
/// 结束（正常完成 / 各类错误 / abort / 软上限 / 消费循环超时 / 任务 panic）。
///
/// 实现方式：`run_chat_loop_task` 在任务最开头创建 `Arc<StopSignal>`，交给
/// [`ChatOrchestrator`]（供 `run_chat_loop` 各出口显式触发）。显式触发点携带
/// 准确的"本轮最后一条消息"；[`StopSignal::drop`] 兜底仅在显式触发全部未发生时
/// 生效（例如 chat_loop 任务 panic 被 JoinError 吞掉、消费循环 1800s 超时提前
/// return、provider 解析失败根本没能进入 loop），从而把 Stop 从"依赖每个出口都
/// 记得调用"升级为"由生命周期保证"。
///
/// 为什么显式 + RAII 双轨而非纯 RAII：`last_message` 取自 chat_loop 的
/// `context.messages`，其所有权随函数返回销毁，只有显式调用点能拿到准确值；
/// RAII 只能提供"一定会触发、但 last_message 退化为空串"的下界。兜底触发时打
/// warn 日志，使"漏调显式 fire"这类缺口在运行时可见（session-mechanism-unification.md §4.4）。
pub struct StopSignal {
    parent: Option<Arc<dyn Plugin>>,
    /// Stop 钩子要投递的请求上下文（`fire_hook` 内部会再 fork 一份并设置
    /// PATH=payload，故此处持有的是原始 chat 上下文）。
    ctx: Arc<dyn InvokeRequest>,
    fired: AtomicBool,
}

impl StopSignal {
    pub fn new(parent: Option<Arc<dyn Plugin>>, ctx: Arc<dyn InvokeRequest>) -> Self {
        Self {
            parent,
            ctx,
            fired: AtomicBool::new(false),
        }
    }

    /// 本请求生命周期内 Stop 是否已触发。
    pub fn fired(&self) -> bool {
        self.fired.load(Ordering::SeqCst)
    }
    /// 触发 Stop；返回 `true` 表示本次调用是真正生效的那一次。
    /// `messages` 为当前请求视图消息（取末条作为 `last_message`）。
    pub async fn fire(&self, messages: &[ChatMessage]) -> bool {
        if self.fired.swap(true, Ordering::SeqCst) {
            return false;
        }
        let last_message = messages
            .last()
            .map(|m| m.content.as_ref().map(|c| c.to_text()).unwrap_or_default())
            .unwrap_or_default();
        let _ = fire_hook(
            &self.parent,
            HookEvent::Stop {
                last_message: last_message.to_string(),
            },
            self.ctx.clone(),
        )
        .await;
        true
    }
    /// 兜底触发（同步、幂等）：显式触发点一次都没执行过时，补发一次 Stop。
    ///
    /// 由 `WorkingGuard::drop`（panic / 消费循环超时 / provider 解析失败）与
    /// [`StopSignal::drop`]（最后防线）共用。Drop 语境不能 await，故投递到
    /// detached 任务；无 tokio 运行时（进程退出路径）时跳过外发并告警，
    /// `fired` 保持置位、不再重试。
    pub fn fire_fallback(&self) {
        if self.fired.swap(true, Ordering::SeqCst) {
            return;
        }
        if self.parent.is_none() {
            // 无父插件时 fire_hook 本身就是 no-op，不打噪声日志
            return;
        }
        crate::plugin_warn!(
            "session",
            "[Stop] 显式 Stop 触发点未执行，由 StopSignal 生命周期兜底补发一次（last_message 为空）"
        );
        let parent = self.parent.clone();
        let ctx = self.ctx.clone();
        match tokio::runtime::Handle::try_current() {
            Ok(handle) => {
                handle.spawn(async move {
                    let _ = fire_hook(
                        &parent,
                        HookEvent::Stop {
                            last_message: String::new(),
                        },
                        ctx,
                    )
                    .await;
                });
            }
            Err(_) => {
                // 没有运行时可承载：保持 fired 置位、不再重试。回滚标记没有意义——
                // 本方法的全部调用点（WorkingGuard::drop / StopSignal::drop）都处在
                // 同一条同步析构链上，下一层 Drop 同样不会有运行时，重试只会失败并
                // 重复告警（进程退出路径本就放弃了外发）。
                crate::plugin_warn!(
                    "session",
                    "[Stop] 当前无 tokio 运行时，Stop 兜底触发被跳过（进程退出路径）"
                );
            }
        }
    }
}

impl Drop for StopSignal {
    fn drop(&mut self) {
        // 最后防线：显式触发点一个都没走到（panic / 消费循环超时 / 提前 return）。
        // 已触发过则直接返回——与 `fire_fallback` 的幂等判定等价，但避免在
        // 正常路径（绝大多数请求都显式 fire 过）上多做一次原子写。
        if self.fired() {
            return;
        }
        self.fire_fallback();
    }
}

/// 会话编排器（自 model/context.rs 迁入，Phase E-②）：
/// 持有唯一生效的模型服务、父插件钩子通道与预计算上下文上限（session 确定性持有）。
/// 轮次收尾状态机 `finalize_assistant_turn` 随之一并迁入；
/// turn_processor 薄委托层消亡（chat_loop 直调
/// `provider.execute_turn` 与 `finalize_assistant_turn`）。
///
/// 生命周期与 [`StopSignal`] 绑定：Stop 的显式触发点在本 loop 的各出口，
/// RAII 兜底点在 `run_chat_loop_task` 的 `WorkingGuard`。
pub struct ChatOrchestrator {
    /// 唯一生效的模型服务（model 插件按上下文解析后经 CAPABILITY_VISITOR 注册；
    /// core 纯 trait 的 trait object——session 对协议实现零依赖）
    pub provider: Arc<dyn ModelProvider>,
    pub parent: Option<Arc<dyn Plugin>>,
    /// 预计算的生效上下文上限（`provider.effective_context_tokens()` 结果，
    /// 构造时由调用方传入，避免异步钩子在热路径反复触发）
    pub context_limit: u32,
    /// 本次请求生命周期的 Stop 触发器（由 `run_chat_loop_task` 创建并共享给
    /// `WorkingGuard` 兜底，见 [`StopSignal`]）
    pub stop: Arc<StopSignal>,
}

impl ChatOrchestrator {
    pub fn new(
        provider: Arc<dyn ModelProvider>,
        parent: Option<Arc<dyn Plugin>>,
        context_limit: u32,
        stop: Arc<StopSignal>,
    ) -> Self {
        Self {
            provider,
            parent,
            context_limit,
            stop,
        }
    }

    pub async fn finalize_assistant_turn(
        &self,
        root_id: &str,
        out: &TurnOutput,
        tools: &[ToolCallInfo],
        channel: &PluginChannel,
    ) {
        if out.is_reasoning_only(tools.len()) {
            // reasoning-only：模型只产生了 reasoning，没有独立的文本回复。
            //
            // 同一段 reasoning 在落库时由 build_assistant_messages 以「Text 响应子节点」承载
            // （effective_text 对「无文本回复」的回退语义）。因此这里**绝不能**再额外广播一个
            // content=reasoning 的 Text 节点——否则前端会同时持有「Reasoning 子节点」与
            // 「Text 响应子节点」两份相同内容，表现为：
            //   · 流式期间：思考块 + 一段相同文本先后出现，看起来像"同一段文本被重复写入"；
            //   · 历史刷新后：存储层本就重复（factor≈2），渲染出两份。
            //
            // 流式期间 ReasoningDelta 已经把 reasoning 累积进 reasoning_child_id 节点，
            // 此处仅将其与根 Turn 标记 Completed 即可。仅当流式期间因故未建立 Reasoning 节点时，
            // 才补发一个 Text 节点兜底（此时不存在 Reasoning 节点，不会造成重复）。
            if !out.reasoning_child_id.is_empty() {
                emit_status(
                    channel,
                    out.reasoning_child_id.clone(),
                    MessageStatus::Completed,
                )
                .await;
            } else {
                let resp_id = if out.response_text_child_id.is_empty() {
                    short_id()
                } else {
                    out.response_text_child_id.clone()
                };
                emit_update(
                    channel,
                    ChatMessage {
                        id: resp_id,
                        parent_id: Some(root_id.into()),
                        role: Some(MessageRole::Assistant),
                        msg_type: Some(MessageType::Text),
                        content: Some(MessageContent::Text(out.reasoning.clone())),
                        status: Some(MessageStatus::Completed),
                        ..Default::default()
                    },
                )
                .await;
            }
            emit_status(channel, root_id.into(), MessageStatus::Completed).await;
            return;
        }

        // Mark reasoning child as completed
        if !out.reasoning.is_empty() && !out.reasoning_child_id.is_empty() {
            emit_status(
                channel,
                out.reasoning_child_id.clone(),
                MessageStatus::Completed,
            )
            .await;
        }

        // Mark response text child as completed (exists if there was text content)
        if !out.text.is_empty() && !out.response_text_child_id.is_empty() {
            emit_status(
                channel,
                out.response_text_child_id.clone(),
                MessageStatus::Completed,
            )
            .await;
        }

        // Mark tool calls (composite) as completed
        for tc in tools {
            if let Some(tc_id) = &tc.id {
                emit_status(channel, tc_id.clone(), MessageStatus::Completed).await;
            }
        }

        // Mark the root Turn node as completed
        emit_status(channel, root_id.into(), MessageStatus::Completed).await;
    }
}

/// 系统提示词的唯一真源（P0-1）。
///
/// 解析优先级：
/// 1. 请求显式指定（`req.system_prompt`）
/// 2. 统一收集机制注册的系统提示词：优先 `"default"` 键，其次请求指定的
///    `provider_id` 键，再退首个注册项
/// 3. 硬编码兜底（维持既有行为）
///
/// 此前该逻辑散落在 `run_chat_loop` 循环体内，且实际发给模型的提示词另取自
/// `req.system_prompt`，导致 visitor 注册链被完全绕过——插件经 `traverse`
/// 注册的系统提示词从未真正送达模型。收敛到此单点后，压缩开销估算与本轮
/// 实际请求共用同一份解析结果。
async fn resolve_system_prompt(
    req_system_prompt: Option<&str>,
    req_provider_id: Option<&str>,
    ctx: &Arc<dyn InvokeRequest>,
) -> String {
    let fallback = || "You are a helpful MODEL assistant.".to_string();
    if let Some(s) = req_system_prompt {
        if !s.is_empty() {
            return s.to_string();
        }
    }
    let resolved = match ctx.get(crate::symbio_core::CAPABILITY_VISITOR) {
        Some(visitor) => {
            let prompts = visitor.list_system_prompts().await;
            prompts
                .iter()
                .find(|(k, _)| k == "default")
                .or_else(|| req_provider_id.and_then(|pid| prompts.iter().find(|(k, _)| k == pid)))
                .or_else(|| prompts.first())
                .map(|(_, v)| v.clone())
        }
        None => None,
    };
    resolved.filter(|s| !s.is_empty()).unwrap_or_else(fallback)
}

pub async fn run_chat_loop(
    orchestrator: &ChatOrchestrator,
    ctx: Arc<dyn InvokeRequest>,
    mut channel: PluginChannel,
) -> Result<(), PluginError> {
    let mut req: model_chat::Request = ctx.payload()?;

    plugin_info!(
        "session",
        ">>> NEW SESSION START (Protocol: {:?})",
        orchestrator.provider.api_protocol()
    );

    // 用户明确要求**不要**设置 max_tool_rounds 硬性上限（智能体会话轮次越来越多）。
    // 因此默认（request 未显式给出）=「无上限」；仅在调用方**显式**设置时才作为软上限并给出提示。
    let configured_max_tool_rounds = req.max_tool_rounds;
    let auto_compress = req.auto_compress.unwrap_or(true);
    // context_compact 主动压缩机制默认关闭（与 session_config serde 默认一致），
    // 必须在插件配置中显式开启才生效
    let enable_compact_tool = req.enable_compact_tool.unwrap_or(false);

    let mut tool_rounds: usize = 0;
    let mut continuation_count: u32 = 0;
    // 水位提醒（nudge）一次性标记：每次用户请求生命周期内最多注入一次；
    // 主动压缩成功后重置（上下文回落后允许再次提醒）。
    let mut nudged_this_request = false;
    const MAX_CONTINUE_ROUNDS: u32 = 3;
    // 轮次老化淡化的激活阈值与保留窗口：超过该轮次后，请求视图层（build_request_view）
    // 对较早的工具结果做 head/tail 摘要，始终保持最近 K 轮的原文与全部 assistant
    // 文本/推理，最小化对思维链的破坏。淡化只作用于请求视图，存储保持完整历史。
    const FADE_ACTIVATE_ROUNDS: usize = 40;
    const FADE_KEEP_RECENT_TURNS: usize = 12;

    let session = open_chat_session(&ctx).await;
    let mut single_message = req.single_message;
    let mut context = SessionContext {
        messages: Vec::new(),
        session,
    };

    let abort_flag = Arc::new(AtomicBool::new(false));

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
            &abort_flag,
            &context.session,
            tr,
        )
        .await
        {
            Ok(crate::plugins::session::resume::ResumeOutcome::Continue) => {
                // 成功：turn 循环会从 session 加载含新工具结果的历史
            }
            Ok(crate::plugins::session::resume::ResumeOutcome::Done) => {
                fire_stop_hook(orchestrator, &[]).await;
                return Ok(());
            }
            Err(e) => {
                plugin_warn!("session", "[Resume] process_resume failed: {}", e);
                fire_stop_hook(orchestrator, &[]).await;
                return Err(e);
            }
        }
    }

    loop {
        // 每轮开始时从 ChatSession 获取最新上下文（轮次窗口生效；骨架化/淡化为请求视图层职责）。
        // 心跳任务等场景可设置 `load_history = false`：仅用本次 single_message，
        // 完全不加载历史，也不保留上一轮内存累积（上一轮内容随本轮重置）。
        context.messages = if req.load_history.unwrap_or(true) {
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
        if tool_rounds == 0 {
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

        let mut last_saved = context.messages.len();

        // 显式软上限：仅当调用方**主动**给出 max_tool_rounds 才生效（默认 None = 无限轮次）。
        // 达到上限时给出明确提示再退出，而不像从前那样在 chat_loop.rs:419 静默 Ok(())。
        if let Some(max) = configured_max_tool_rounds {
            if tool_rounds >= max {
                let _ = channel
                    .tx
                    .send(PluginFrame::Data(
                        serde_json::to_value(session_chat_response::StreamEvent::Error {
                            error: format!(
                                "已达到本轮工具调用上限（{max}）。如需继续，请再次发送消息。"
                            ),
                        })
                        .unwrap_or_default(),
                    ))
                    .await;
                persist_messages(&context, last_saved, &channel).await;
                fire_stop_hook(orchestrator, &context.messages).await;
                return Ok(());
            }
        }

        if abort_flag.load(Ordering::SeqCst) {
            // SYS-002: 早期 return 路径上的副作用（last_saved 尚未用作流式增量锚点，
            // 此分支里不更新，但保留 last_saved 维持语义对称）。
            fire_stop_hook(orchestrator, &context.messages).await;
            return Ok(());
        }

        plugin_info!(
            "session",
            "--- TURN {} START --- (msgs={}, tools={})",
            tool_rounds,
            context.messages.len(),
            0
        );

        if check_abort(&abort_flag).await {
            fire_stop_hook(orchestrator, &context.messages).await;
            return Ok(());
        }

        // ── 被动语义压缩（L5：70% 触发）────────────────────────────────
        // 系统提示词：唯一真源 resolve_system_prompt（P0-1）。解析结果同时供
        // 压缩开销估算与 execute_turn 实际请求使用——此前 execute_turn 直接取
        // req.system_prompt，绕过了 visitor 注册链，插件经 traverse 注册的系统
        // 提示词从未真正送达模型。
        let system_prompt_owned = resolve_system_prompt(
            req.system_prompt.as_deref(),
            req.provider_id.as_deref(),
            &ctx,
        )
        .await;
        let system_prompt_for_request = system_prompt_owned.as_str();
        if auto_compress {
            match auto_compress_process(
                orchestrator,
                &mut context,
                &mut channel,
                &ctx,
                &abort_flag,
                system_prompt_for_request,
                false,
                None,
            )
            .await
            {
                Ok(Some(history_count)) => {
                    plugin_info!(
                        "session",
                        "Context compressed: {} messages -> 1 message",
                        history_count
                    );
                    last_saved = context.messages.len();
                }
                Ok(None) => {}
                Err(e) => {
                    plugin_warn!("session", "auto_compress_process failed: {e}");
                    fire_stop_hook(orchestrator, &context.messages).await;
                    return Err(e);
                }
            }
        }

        // ── 水位提醒（nudge，目标四）─────────────────────────────────────
        // 估算用量 ≥ 55% 有效上限时，在请求视图末尾注入一条一次性系统提示
        // （请求级、不落库，由 build_request_view 统一追加），引导模型在
        // "阶段间隙"主动调用 context_compact（比 70% 硬触发更早、时机更优）。
        // 门控：提醒只为引导工具调用，跟随工具开关（enable_compact_tool），
        // 与自动压缩开关解耦（关自动压缩、开工具压缩时仍需提醒）。
        // 去重：每次用户请求生命周期内最多注入一次（主动压缩成功后重置）；
        // 提醒不再持久化，无需扫描历史做去重，也不会占用轮次窗口的 User 计数。
        let mut inject_nudge = false;
        if enable_compact_tool && !nudged_this_request {
            let effective_limit = orchestrator.context_limit as usize;
            let overhead =
                compression::estimate_request_overhead(system_prompt_for_request, &ctx).await;
            if compression::should_emit_context_nudge(&context.messages, effective_limit, overhead)
            {
                nudged_this_request = true;
                inject_nudge = true;
                plugin_info!(
                    "session",
                    "[Compress] context nudge emitted (~55% of limit), suggesting context_compact"
                );
            }
        }

        let root_id: String = short_id();
        emit_streaming_start(&mut channel, &root_id, Some(tool_rounds)).await;

        let mut tools = if let Some(tool_visitor) = ctx.get(crate::symbio_core::CAPABILITY_VISITOR)
        {
            tool_visitor.list_capability().await
        } else {
            Vec::new()
        };
        // 主动压缩工具（目标四）：仅当工具压缩启用时暴露给模型（独立于自动压缩开关）。
        // 执行不走 CapabilityVisitor 分发，由下方拦截逻辑处理（需要编排器内部链路）。
        if enable_compact_tool {
            tools.push(compression::context_compact_tool_meta());
        }

        // 请求视图（唯一入口 build_request_view）：存储视图之上叠加四项**不落库**的
        // 裁剪，全部只作用于本次 send_request 的请求包，不回写 context.messages——
        // 存储保持完整历史，last_saved 锚点与 persist_messages 切片不会错位。
        // 1) 内容节点淡化：B1 保护窗口（末条 + 最近 N 个内容节点）外的超大正文/思考
        //    做 head/tail 摘要（阈值取会话配置 line_threshold / token 上限 2048）；
        // 2) fade：轮次过多时淡化较早的工具结果（存储保留全文，视图每轮重建，天然幂等）；
        // 3) 工具级骨架化：从 CapabilityVisitor 的能力声明（context_retention）动态解析
        //    保留策略，LastOnly/LastN → 更早调用的参数与结果替换为占位文案
        //    （ToolCall↔Tool 配对完整保留，不会造成大模型逻辑断联）；
        // 4) nudge：水位提醒请求级注入（不落库、不占轮次窗口的 User 计数）。
        let request_view: Vec<ChatMessage> = {
            let window = req.tool_context_window.unwrap_or(15);
            let retention: std::collections::HashMap<
                String,
                crate::symbio_core::ToolContextRetention,
            > = tools
                .iter()
                .filter_map(|t| {
                    t.context_retention
                        .filter(|r| !matches!(r, crate::symbio_core::ToolContextRetention::All))
                        .map(|r| {
                            let short = t.name.rsplit('/').next().unwrap_or(&t.name);
                            (short.to_string(), r)
                        })
                })
                .collect();
            compression::build_request_view(
                &context.messages,
                window,
                &retention,
                tool_rounds > FADE_ACTIVATE_ROUNDS,
                FADE_KEEP_RECENT_TURNS,
                context.session.compress_keep_recent(),
                context.session.line_threshold(),
                inject_nudge,
            )
        };
        let request_messages: &[ChatMessage] = request_view.as_slice();

        // Turn 创建后的首个 abort 检查点：覆盖"压缩阶段中止"等 send_request
        // 之前置位的场景。压缩失败已就地降级（不冒泡），但 abort_flag 仍为
        // true 且 abort 帧已被压缩请求消费——若不在此拦截，execute_turn 会
        // 发起一次多余的 LLM 请求。此处 Turn 已创建（上方 emit_streaming_start），
        // 冒泡 Err(Aborted) → 消费循环 ABORTED 分支 → persist_failure 把本轮
        // Turn 收尾为 Failed + "用户手动中止了本次回复"（错误条 + 重试入口），
        // 不会波及上一轮已成功的 Turn（persist_failure 按 failing_turn 子树收窄）。
        if abort_flag.load(Ordering::SeqCst) {
            fire_stop_hook(orchestrator, &context.messages).await;
            return Err(PluginError::Aborted);
        }

        let result = orchestrator
            .provider
            .execute_turn(
                system_prompt_for_request,
                request_messages,
                &tools,
                &root_id,
                &mut channel,
                &abort_flag,
            )
            .await;

        let mut out = match result {
            Err(PluginError::RetryWithoutContextId) => {
                for m in &mut context.messages {
                    m.response_id = None;
                }
                // 清除本轮未完成的 Streaming 半截内容（来自上一轮被中断的 LLM 流），
                // 避免下轮 get_context_messages 加载到半截消息污染 LLM 上下文。
                // RetryWithoutContextId 表示 LLM 提供商返回的 context_id 无效（会话不存在），
                // 此前流式产出的 Streaming 节点都是无效半截响应，应直接删除而非保留为 Failed 终态。
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
                // 可渲染错误条与重试入口（docs/turn-tool-mechanisms.md 2.4）。
                // 旧实现直接 return Ok(())：在途 Turn 不落库，刷新即消失且无重试入口。
                fire_stop_hook(orchestrator, &context.messages).await;
                return Err(PluginError::Aborted);
            }
            Err(e) => {
                plugin_warn!("session", "send_request failed: {e}");
                fire_stop_hook(orchestrator, &context.messages).await;
                return Err(e);
            }
            Ok(out) => out,
        };

        if abort_flag.load(Ordering::SeqCst) {
            // 与 Err(PluginError::Aborted) 分支同理：请求结束后才置位的 abort 标志
            // 同样向上冒泡，由消费循环统一收尾（在途 Turn → Failed + error + 可重试，
            // 见 docs/turn-tool-mechanisms.md 2.4）。仅 send_request 之后的 abort
            // 冒泡；turn 循环顶部的边界检查点不冒泡——上一轮已定稿落库，冒泡会把
            // 成功的 Turn 误回滚为 Failed。
            fire_stop_hook(orchestrator, &context.messages).await;
            return Err(PluginError::Aborted);
        }

        let tools_done = out.tool_accumulator.get_completed();
        // 提前取出本轮的结束原因 / 用量 / 是否出现过工具调用 / 文本子节点 id，
        // 因为 `out.into_messages` 会按值消费 out，之后无法再读这些字段。
        let had_tool = out.tool_accumulator.had_any_tool_call();
        let finish = out.finish.clone();
        let usage = out.usage;
        let rtid = out.response_text_child_id.clone();
        let rrid = out.reasoning_child_id.clone();

        orchestrator
            .finalize_assistant_turn(&root_id, &out, &tools_done, &channel)
            .await;

        // 用 provider 返回的真实用量滚动校准 token 估算（中文/代码场景收益最大；
        // 估算长期偏低会直接导致 400 而非过早压缩）。
        if let Some(u) = usage {
            let tok = super::tokenizer::default_tokenizer();
            // 反馈必须用原始启发式估算（count_raw）；用校准后的 count() 自反馈
            // 会让校准系数收敛到 √(真实比值)（见 CalibratedTokenizer::feedback 文档）。
            // 分母必须覆盖 provider 计入 output_tokens 的全部内容：文本 + 思考 +
            // 工具调用名 + 参数 JSON。漏掉任一部分都会系统性低估估算值 → 校准比
            // 偏高 → 水位提前越过阈值 → 压缩被频繁触发。
            let mut estimated = tok.count_raw(&out.text) + tok.count_raw(&out.reasoning);
            for tc in &tools_done {
                if let Some(name) = &tc.name {
                    if !name.is_empty() {
                        estimated += tok.count_raw(name);
                    }
                }
                estimated += tok.count_raw(&tc.arguments.to_string());
            }
            super::tokenizer::report_provider_usage(estimated, u.output);
        }

        let new_msgs = out.into_messages(&root_id, tools_done.len());
        // 工具上下文保留策略（机制化）：不再把策略 Stamp 到节点 meta 持久化，
        // 由 run_chat_loop 在构建 LLM 请求前从 CapabilityVisitor 动态解析，
        // 节点 name 即 LLM 可见工具名，与声明名直接匹配。
        context.messages.extend(new_msgs);

        // 被长度截断的 Turn 打标（供前端「继续」按钮与回溯），不再静默结束（修复"对话突然结束"）。
        if finish.is_length() {
            for m in context.messages.iter_mut() {
                if m.id == rtid || m.id == rrid {
                    let mut meta = m.meta.clone().unwrap_or_else(|| serde_json::json!({}));
                    meta["finish_reason"] = serde_json::json!("length");
                    m.meta = Some(meta);
                }
            }
        }

        if tools_done.is_empty() {
            // 本轮无工具调用 —— 正常收尾，除非是被长度截断。
            if finish.is_length() && !had_tool {
                // 纯文本被 max_tokens 截断且参数完整 → 自动续写：
                // 已产出的（截断）文本已作为 assistant 消息进入上下文，下一轮请求时模型会
                // 自然从断点继续。最多续写 MAX_CONTINUE_ROUNDS 次，避免失控死循环。
                if continuation_count < MAX_CONTINUE_ROUNDS {
                    continuation_count += 1;
                    plugin_info!(
                        "session",
                        "finish=Length，自动续写 ({}/{})",
                        continuation_count,
                        MAX_CONTINUE_ROUNDS
                    );
                    persist_messages(&context, last_saved, &channel).await;
                    continue;
                }
                // 续写次数耗尽：明确告知，不再静默结束。
                let _ = channel.tx.send(PluginFrame::Data(
                    serde_json::to_value(session_chat_response::StreamEvent::Error {
                        error: format!(
                            "输出因达到长度上限而中断（已自动续写 {} 次仍超出）。请提高单次输出预算或缩小任务范围。",
                            MAX_CONTINUE_ROUNDS
                        ),
                    })
                    .unwrap_or_default(),
                )).await;
            } else if finish.is_length() && had_tool {
                // 工具调用参数 JSON 被长度截断：参数残破无法通过续写修复，
                // 该次调用已丢弃 → 明确报错而非静默结束。
                let _ = channel.tx.send(PluginFrame::Data(
                    serde_json::to_value(session_chat_response::StreamEvent::Error {
                        error: "输出在工具调用参数中途达到长度上限而中断。请提高单次输出预算，或把大任务拆小后重试。"
                            .to_string(),
                    })
                    .unwrap_or_default(),
                )).await;
            }
            persist_messages(&context, last_saved, &channel).await;
            fire_stop_hook(orchestrator, &context.messages).await;
            plugin_info!(
                "session",
                "--- TURN END (正常收尾，无工具调用) --- finish={:?}",
                finish
            );
            return Ok(());
        }

        // ── 主动压缩工具拦截（目标四）─────────────────────────────────
        // context_compact 不走 CapabilityVisitor 分发：它需要编排器内部的
        // 压缩链路（LLM 摘要 + 上下文替换 + 会话持久化）。
        // 在此拆分：压缩调用就地执行并生成合成工具结果；其余工具正常分发。
        // 门控：仅当工具压缩开关开启时拦截；开关关闭时工具不暴露，模型幻觉
        // 调用则归入标准工具链，以"未知路径"错误返回（不执行内部压缩链路）。
        let (compact_calls, other_calls): (Vec<_>, Vec<_>) = if enable_compact_tool {
            tools_done.into_iter().partition(|tc| {
                tc.name
                    .as_deref()
                    .map(|n| n == compression::CONTEXT_COMPACT_TOOL_NAME)
                    .unwrap_or(false)
            })
        } else {
            (Vec::new(), tools_done)
        };

        let mut tool_results: Vec<ChatMessage> = Vec::new();
        let mut parent_updates: Vec<ChatMessage> = Vec::new();

        if !compact_calls.is_empty() {
            // 切分点前移到当前用户指令：保留区 = [用户指令, Turn 及其子节点...]，
            // Turn 子树 parent 链完整；旧版切在本 Turn 首个 ToolCall，用户指令与
            // Turn 根被压进快照，保留区只剩 parent 悬空的 ToolCall → provider 400。
            // 返回 0 时 run_context_compact 以 split==0 视为中止，安全。
            let split_user_idx = compression::find_turn_user_split_idx(&context.messages, &root_id);
            let first = compact_calls.first().cloned();
            if let Some(first) = first {
                let call_id = first.id.clone().unwrap_or_default();
                let hints = first
                    .arguments
                    .get("hints")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let (ok, before_t, after_t) = run_context_compact(
                    orchestrator,
                    &mut context,
                    &mut channel,
                    &ctx,
                    &abort_flag,
                    split_user_idx,
                    hints.as_deref(),
                )
                .await;
                if ok {
                    // 压缩成功：上下文回落后允许再次水位提醒
                    nudged_this_request = false;
                    // replace_messages 已整体重写会话存储，当前内存镜像即已落库状态；
                    // 重置持久化锚点，避免末尾 persist_messages 用旧下标切片越界/重复落库
                    last_saved = context.messages.len();
                    plugin_info!(
                        "session",
                        "[Compress] manual compaction done: ~{} -> ~{} tokens",
                        before_t,
                        after_t
                    );
                }
                let result_text = if ok {
                    format!(
                        "Context compacted: ~{before_t} -> ~{after_t} tokens. \
                         The session now starts from the state snapshot followed by the current \
                         task. Continue the task based on the snapshot; archived transcripts are \
                         referenced inside it if details are needed."
                    )
                } else if before_t > 0 && before_t == after_t {
                    format!(
                        "Compaction skipped: history to compress is only ~{before_t} tokens \
                         (below the useful threshold), context unchanged. Continue the task."
                    )
                } else {
                    "Compaction failed and was rolled back; context unchanged. \
                     Continue the task."
                        .to_string()
                };
                let mut meta = serde_json::json!({ "success": ok, "kind": "context_compact" });
                if ok {
                    meta["before_tokens"] = serde_json::json!(before_t);
                    meta["after_tokens"] = serde_json::json!(after_t);
                }
                // 标准工具广播模式（与 process_tool_calls_async 一致，修复诉求1：
                // 前端实时可见 context_compact 的结果子节点与父节点状态）：
                // 先广播 Tool 结果子节点，再广播父 ToolCall 状态补丁。
                let mut tool_msg = build_tool_message(&call_id, &result_text, Some(ok), None);
                if !ok {
                    // 失败属信息性：结果以 Completed 定格（父节点同为 Completed），
                    // 与普通工具结果的处理保持一致，避免孤儿 Failed 节点
                    tool_msg.status = Some(MessageStatus::Completed);
                }
                broadcast_message_update(&channel, tool_msg.clone()).await;
                let parent_update = ChatMessage {
                    id: call_id.clone(),
                    status: Some(MessageStatus::Completed),
                    meta: Some(meta),
                    ..Default::default()
                };
                broadcast_message_update(&channel, parent_update.clone()).await;
                parent_updates.push(parent_update);
                tool_results.push(tool_msg);
            }
            // 同批多余的 compact 调用：直接标记跳过
            for extra in compact_calls.iter().skip(1) {
                if let Some(cid) = &extra.id {
                    // 同批多余调用同样走标准广播模式（跳过说明属信息性结果，定格 Completed）
                    let mut tool_msg = build_tool_message(
                        cid,
                        "Skipped: another context_compact call in this batch was executed.",
                        Some(false),
                        None,
                    );
                    tool_msg.status = Some(MessageStatus::Completed);
                    broadcast_message_update(&channel, tool_msg.clone()).await;
                    let parent_update = ChatMessage {
                        id: cid.clone(),
                        status: Some(MessageStatus::Completed),
                        meta: Some(serde_json::json!({
                            "success": false,
                            "kind": "context_compact",
                            "skipped": true
                        })),
                        ..Default::default()
                    };
                    broadcast_message_update(&channel, parent_update.clone()).await;
                    parent_updates.push(parent_update);
                    tool_results.push(tool_msg);
                }
            }
        }

        let (other_results, other_parent_updates) = process_tool_calls_async(
            other_calls,
            &orchestrator.parent,
            &mut channel,
            &abort_flag,
            ctx.clone(),
        )
        .await;
        tool_results.extend(other_results);
        parent_updates.extend(other_parent_updates);
        context.messages.extend(tool_results.clone());

        // 持久化 ToolCall 父节点状态更新（解决父节点状态不持久化问题）。
        // append_messages 是 push-only 无法更新已存在消息，故显式调用 update_messages。
        if !parent_updates.is_empty() {
            // 同步到 context.messages 内存镜像
            for patch in &parent_updates {
                if let Some(msg) = context.messages.iter_mut().find(|m| m.id == patch.id) {
                    if let Some(s) = &patch.status {
                        msg.status = Some(s.clone());
                    }
                    if let Some(m) = &patch.meta {
                        msg.meta = Some(m.clone());
                    }
                    if let Some(e) = &patch.error {
                        msg.error = Some(e.clone());
                    }
                }
            }
            if let Err(e) = context
                .session
                .update_messages(parent_updates.clone())
                .await
            {
                plugin_warn!("session", "[Session] 父节点状态持久化失败: {}", e);
            }
        }

        persist_messages(&context, last_saved, &channel).await;

        // 检测工具待用户恢复 → 退出本轮：
        // 仅当存在 UserPrompt/WaitingUserAction（confirm/ask_user）时才算需要用户输入。
        // 普通的工具执行失败不再使会话停摆：父节点已标 Completed、错误结果作为合法
        // tool 结果留在上下文喂回 LLM 继续处理，用户也可随时直接发新消息继续。
        let mode = ctx.get(crate::symbio_core::MODE).unwrap_or_default();
        let needs_user_action = tool_results.iter().any(|m| {
            m.msg_type == Some(MessageType::UserPrompt)
                && m.status == Some(MessageStatus::WaitingUserAction)
        }) || parent_updates
            .iter()
            .any(|p| p.status == Some(MessageStatus::WaitingUserAction));

        if needs_user_action {
            // 注：信息性策略下工具失败的父 ToolCall 已标 Completed（错误结果作为
            // 合法 tool 结果喂回 LLM，loop 不中断），不存在「Failed 父节点等待
            // 恢复」的场景——旧版在此处给 Failed+failure_kind 父节点打 recoverable
            // 标记的代码属不可达遗留，已删除（docs/turn-tool-mechanisms.md 1.5）。
            // user_prompt(WaitingUserAction) 驱动的暂停走 approve/reject/answer 恢复。
            plugin_info!("session", "工具待用户恢复（mode={}），退出本轮", mode);
            fire_stop_hook(orchestrator, &context.messages).await;
            return Ok(());
        }

        // 轮次计数：用户明确要求不设硬性上限，超长对话的规模控制由请求视图层的
        // fade / 骨架化（build_request_view）承担——存储保持完整历史，视图逐轮裁剪。
        tool_rounds += 1;
        plugin_info!(
            "session",
            "--- TURN {} DONE (工具轮结束，进入下一轮) --- 工具调用 {} 个",
            tool_rounds - 1,
            tool_results.len()
        );
    }
}

async fn broadcast_message_update(channel: &PluginChannel, message: ChatMessage) {
    let _ = channel
        .tx
        .send(PluginFrame::Data(
            serde_json::to_value(session_chat_response::StreamEvent::Update { message })
                .unwrap_or_default(),
        ))
        .await;
}

async fn persist_messages(context: &SessionContext, last_saved: usize, channel: &PluginChannel) {
    let new_messages = &context.messages[last_saved..];
    if new_messages.is_empty() {
        return;
    }

    if let Err(e) = context.session.append_messages(new_messages.to_vec()).await {
        // 持久化失败：必须显式通知前端（不静默吃错误），让用户知道部分消息没落库。
        // 仍把消息留在内存中，chat_loop 不中断（让当前对话能继续）。
        let msg = format!("消息持久化失败（消息仍在内存中）: {}", e);
        plugin_warn!("session", "[Session] {}", msg);
        let _ = channel
            .tx
            .send(PluginFrame::Data(
                serde_json::to_value(session_chat_response::StreamEvent::Error { error: msg })
                    .unwrap_or_default(),
            ))
            .await;
    }
}

/// 从 ctx 读取 session 编排器交付的会话引擎句柄（SESSION_HANDLE）。
///
/// session 编排在路由 model/chat 前已将构造好的会话引擎实例放入 chat_ctx，
/// model 无状态化后不再反向路由 session/open。仅句柄缺失（异常编排路径）时
/// 回退内存 FallbackChatSession（无持久化）。
async fn open_chat_session(ctx: &Arc<dyn InvokeRequest>) -> Arc<dyn ChatSession> {
    if let Some(handle) = ctx.get(SESSION_HANDLE) {
        return handle.0.clone();
    }

    plugin_warn!(
        "session",
        "[Session] 上下文未交付 SESSION_HANDLE，回退内存会话（无持久化）"
    );
    Arc::new(FallbackChatSession::default())
}

struct FallbackChatSession {
    // 用 tokio::sync::Mutex 替代 std::sync::RwLock
    // 原因（S-002 修复）：std::sync::RwLock 的 read/write guard 持锁时若遇到 .await
    // 会导致 tokio worker 线程被同步阻塞；本结构虽是 fallback 路径，但 get_messages 是
    // 每次 session 切换的高频调用点。tokio::sync::Mutex 的 lock() 是异步的，不阻塞 worker。
    // 锁内操作仅是 Vec 克隆/追加/替换，无 await 边界，因此不会出现持锁跨 await 的反模式。
    messages: tokio::sync::Mutex<Vec<ChatMessage>>,
}

impl Default for FallbackChatSession {
    fn default() -> Self {
        Self {
            messages: tokio::sync::Mutex::new(Vec::new()),
        }
    }
}

#[async_trait::async_trait]
impl ChatSession for FallbackChatSession {
    async fn get_messages(&self) -> Result<Vec<ChatMessage>, PluginError> {
        let messages = self.messages.lock().await;
        let mut messages = messages.clone();
        // 与持久化实现保持一致：按单调序号排序
        messages.sort_by_key(|m| m.seq.unwrap_or(i64::MAX));
        Ok(messages.clone())
    }

    async fn get_context_messages(
        &self,
        _max_turns: Option<usize>,
    ) -> Result<Vec<ChatMessage>, PluginError> {
        self.get_messages().await
    }

    async fn append_messages(&self, messages: Vec<ChatMessage>) -> Result<usize, PluginError> {
        let mut store = self.messages.lock().await;
        let mut seq_cursor = max_seq(&store);
        for mut m in messages {
            if m.seq.is_none() {
                seq_cursor += 1;
                m.seq = Some(seq_cursor);
            } else if let Some(s) = m.seq {
                if s > seq_cursor {
                    seq_cursor = s;
                }
            }
            store.push(m);
        }
        Ok(store.len())
    }

    async fn replace_messages(&self, messages: Vec<ChatMessage>) -> Result<(), PluginError> {
        let mut store = self.messages.lock().await;
        let mut messages = messages;
        assign_seq(&mut messages, max_seq(&store));
        *store = messages;
        Ok(())
    }

    async fn update_messages(&self, messages: Vec<ChatMessage>) -> Result<(), PluginError> {
        let mut store = self.messages.lock().await;
        for patch in messages {
            if let Some(existing) = store.iter_mut().find(|m| m.id == patch.id) {
                // 增量合并：patch 里为 None 的字段表示"不修改"，保留原值。
                existing.apply_patch(&patch);
            }
        }
        Ok(())
    }

    async fn clear(&self) -> Result<(), PluginError> {
        let mut messages = self.messages.lock().await;
        messages.clear();
        Ok(())
    }

    fn session_id(&self) -> &str {
        "ephemeral"
    }
    fn max_messages(&self) -> usize {
        100
    }
    fn line_threshold(&self) -> usize {
        200
    }
}

async fn fire_user_prompt_submit_hook(
    orchestrator: &ChatOrchestrator,
    context: &SessionContext,
    ctx: &Arc<dyn InvokeRequest>,
) {
    let user_prompt = context
        .messages
        .last()
        .map(|m| m.content.as_ref().map(|c| c.to_text()).unwrap_or_default())
        .unwrap_or_default();
    let _ = fire_hook(
        &orchestrator.parent,
        HookEvent::UserPromptSubmit {
            prompt: user_prompt,
        },
        ctx.clone(),
    )
    .await;
}

async fn emit_streaming_start(channel: &mut PluginChannel, root_id: &str, turn: Option<usize>) {
    let meta = turn.map(|t| serde_json::json!({"turn": t}));
    let _ = channel
        .tx
        .send(PluginFrame::Data(
            serde_json::to_value(session_chat_response::StreamEvent::Update {
                message: ChatMessage {
                    id: root_id.to_string(),
                    // Turn 是根级节点，与 User 互为兄弟（请求/响应由 MessageRole 区分）
                    parent_id: None,
                    role: Some(
                        crate::symbio_core::schemas::session::chat_message::MessageRole::Assistant,
                    ),
                    msg_type: Some(MessageType::Turn),
                    status: Some(MessageStatus::Streaming),
                    meta,
                    ..Default::default()
                },
            })
            .unwrap_or_default(),
        ))
        .await;
}

async fn check_abort(abort_flag: &Arc<AtomicBool>) -> bool {
    abort_flag.load(Ordering::SeqCst)
}

// 8 个参数均为流程管道的直通依赖，提取 struct 反而增加一层无谓的间接性（代码库中
// chat_loop 主流程同样以长参数管道为惯例），故允许超参。
#[allow(clippy::too_many_arguments)]
async fn auto_compress_process(
    orchestrator: &ChatOrchestrator,
    context: &mut SessionContext,
    channel: &mut PluginChannel,
    ctx: &Arc<dyn InvokeRequest>,
    abort_flag: &Arc<AtomicBool>,
    system_prompt: &str,
    force: bool,
    extra_hints: Option<&str>,
) -> Result<Option<usize>, PluginError> {
    let effective_context_limit = orchestrator.context_limit as usize;

    // 请求级固定开销（system prompt + 工具定义）必须计入阈值判断，
    // 否则上下文实际占用被低估，压缩触发过晚 → 撞 provider 的 context-length 400。
    let overhead = compression::estimate_request_overhead(system_prompt, ctx).await;

    if !compression::should_start_compression(
        &context.messages,
        effective_context_limit,
        force,
        overhead,
    ) {
        return Ok(None);
    }

    let (compression_msg, history_to_compress, history_to_keep) =
        match compression::prepare_compression(&context.messages) {
            Some(v) => v,
            None => return Ok(None),
        };

    // 主动压缩收益护栏：待压缩历史太小就不值得一次 LLM 调用
    // （被动压缩天然满足，这里主要防 context_compact 的无效触发）。
    let compress_tokens: usize = history_to_compress
        .iter()
        .map(compression::estimate_message_tokens)
        .sum();
    if compress_tokens < compression::MIN_COMPACT_TOKENS {
        return Ok(None);
    }

    // 压缩核心与主动 context_compact 工具共用（compress_with_snapshot_core）：
    // 失败已在核心内就地回滚，这里只区分"成功/未压缩"两种结果。
    let original_count = context.messages.len();
    let post_tokens = compress_with_snapshot_core(
        orchestrator,
        context,
        channel,
        ctx,
        abort_flag,
        compression_msg,
        history_to_keep,
        extra_hints,
        "auto",
    )
    .await;
    match post_tokens {
        Some(_) => Ok(Some(original_count)),
        None => Ok(None),
    }
}

/// 快照压缩核心 —— 被动 L2 自动压缩（[`auto_compress_process`]）与主动
/// `context_compact` 工具（[`run_context_compact`]）共用的唯一实现。
///
/// 旧版两处各维护一份 ~60 行近乎相同的流水线（PreCompact 钩子 → transcript
/// 转存 → LLM 压缩请求 → 快照校验/纠正重试/降级兜底 → meta 与快照消息构造 →
/// 保留区拼接落库），行为漂移风险高，故收敛于此。
///
/// 职责：
/// 1. PreCompact 钩子 + 压缩前完整历史 transcript 转存（可回溯原则）；
/// 2. 上下文临时替换为 `[compression_msg]`，以专用压缩提示词发起 LLM 请求；
/// 3. 快照校验：提取 `<state_snapshot>` → 缺失则附纠正指令重试一次 →
///    仍失败降级 `fallback_snapshot`；两次均空 → 回滚并放弃；
/// 4. 构造快照消息（meta：compacted/post_tokens/transcript_path/compact_hints/
///    protocol_version/prompt_fingerprint）并与保留区拼接，replace_messages 落库。
///
/// 失败语义：**任何失败都不向上冒泡**（回滚到调用前历史后返回 None），由调用方
/// 决定对外呈现（auto → `Ok(None)` 继续本轮 Turn；manual → 工具结果"压缩失败已回滚"）。
///
/// `keep_messages`：压缩后原样保留在快照之后的消息（auto = 未压缩尾段；
/// manual = 当前用户指令 + 进行中 Turn 及之后，保证 Turn 子树 parent 链完整，
/// 详见 [`run_context_compact`] 文档中的切分点约束）。
/// 成功返回 `Some(post_tokens)`（压缩后内容水位：快照 + 保留区，不含请求级 overhead）。
#[allow(clippy::too_many_arguments)]
async fn compress_with_snapshot_core(
    orchestrator: &ChatOrchestrator,
    context: &mut SessionContext,
    channel: &mut PluginChannel,
    ctx: &Arc<dyn InvokeRequest>,
    abort_flag: &Arc<AtomicBool>,
    compression_msg: ChatMessage,
    keep_messages: Vec<ChatMessage>,
    extra_hints: Option<&str>,
    log_tag: &str,
) -> Option<usize> {
    // 保存原始历史：压缩失败时回滚，绝不能让 `[compression_msg]` 残留在上下文里。
    let original_messages = context.messages.clone();
    let _ = fire_hook(&orchestrator.parent, HookEvent::PreCompact, ctx.clone()).await;

    // 可回溯原则：压缩前把完整历史转存为 transcript，路径记入快照 meta。
    // 旧版直接 replace_messages，被压掉的历史在物理层"凭空消失"，
    // 旧存档文件成为孤儿，事后无法审计。
    let transcript_path = save_transcript_archive(&original_messages, context.session.session_id());

    // ── 输入超限死锁预判（日志实证的恶性循环）──────────────────────────
    // LLM 摘要请求的请求体**就携带完整待压缩历史**——若历史本身已超 Provider
    // 有效输入上限，摘要请求必然 400（"Input token exceed the limit"），
    // 且每轮自动压缩都会重发这条注定失败的巨型请求：压缩永不收敛、每轮开头
    // 多一段漫长的无响应。预判命中时跳过 doomed 请求，直接本地机械兜底
    // （尾部保留 + 说明头，不依赖 LLM），让上下文水位立即回落到可工作区间。
    let pending: Vec<ChatMessage> = {
        let mut v = Vec::with_capacity(1 + keep_messages.len());
        v.push(compression_msg.clone());
        v.extend(keep_messages.iter().cloned());
        v
    };
    let overhead_tokens =
        compression::estimate_request_overhead(&compression::get_compression_prompt(), ctx).await;
    let pending_tokens: usize = pending
        .iter()
        .map(compression::estimate_message_tokens)
        .sum();
    let effective_limit = orchestrator.context_limit as usize;
    if pending_tokens + overhead_tokens > effective_limit {
        plugin_warn!(
            "session",
            "[Compress] {log_tag}: summary request itself exceeds input limit ({} + {} > {}), \
             applying local emergency tail compression instead of a doomed LLM call",
            pending_tokens,
            overhead_tokens,
            effective_limit
        );
        // 机械兜底目标：压到有效上限的一半（给后续对话留出增长空间，
        // 避免刚兜底完又立刻越线）
        let target = effective_limit / 2;
        let (mut new_messages, removed) = compression::emergency_tail_compression(
            &original_messages,
            target,
            transcript_path.as_deref(),
        );
        if removed > 0 {
            // 与 LLM 快照同款的 meta 指纹（协议版本标记 emergency 路径）
            if let Some(head) = new_messages.first_mut() {
                let mut meta = head.meta.clone().unwrap_or_else(|| serde_json::json!({}));
                meta["protocol_version"] =
                    serde_json::json!(super::compression::COMPRESSION_PROTOCOL_VERSION);
                head.meta = Some(meta);
            }
            context.messages = new_messages;
            let _ = context
                .session
                .replace_messages(context.messages.clone())
                .await;
            plugin_info!(
                "session",
                "[Compress] {log_tag}: emergency tail compression removed {removed} messages"
            );
            // post_tokens 按兜底后的内容水位返回（迟滞比较的读取侧口径）
            let post_tokens: usize = context
                .messages
                .iter()
                .map(compression::estimate_message_tokens)
                .sum();
            return Some(post_tokens);
        }
        // 兜底也无需截断（理论上不可达：能进压缩说明已越线）——回滚放弃
        context.messages = original_messages;
        return None;
    }

    context.messages = vec![compression_msg];

    // 诉求3：专用压缩 system 提示词（模板只在本次请求出现，与主对话隔离）
    let compression_prompt = compression::get_compression_prompt();
    let root_id = short_id();
    let mut summary = match send_compression_request(
        orchestrator,
        &compression_prompt,
        &context.messages,
        &root_id,
        channel,
        abort_flag,
    )
    .await
    {
        Ok(s) => s,
        // **任何失败都不在压缩阶段向上冒泡**（用户中止 / 限流 / 空摘要 /
        // 流中断 / 网关错误等一律优雅降级）：
        //
        // 压缩发生在本轮 Turn 创建之前（调用点的 emit_streaming_start 在其后）。
        // 若在此处向上冒泡，消费循环的 Error 帧分支会调用 persist_failure，
        // 而 collected 中最后一个根级 Turn 是上一轮已成功定稿的 Turn——
        // 其"已落库 Completed 强制回滚 Failed"逻辑会误回滚成功 Turn。
        //
        // 就地回滚到未压缩历史后返回 Ok(None)，让主循环继续创建本轮
        // Turn；真正的 LLM 请求若同样中止 / 限流 / 失败，会以标准链路
        // （在途 Turn → persist_failure）呈现在本轮 Turn 上：
        // - Aborted → 消费循环 ABORTED 分支 → Failed + "用户手动中止了
        //   本次回复"（前端错误条 + 重试入口，docs/turn-tool-mechanisms.md 2.4）；
        // - RateLimited / 其他 → 消费循环非中止分支 → Failed + 错误原因。
        Err(e) => {
            plugin_warn!("session",
                "[Compress] {log_tag} compression failed ({}), falling back to uncompressed context",
                e
            );
            context.messages = original_messages;
            return None;
        }
    };

    // 快照校验：从输出提取 <state_snapshot>；缺失则纠正重试一次；
    // 仍失败则降级为纯文�快照（有总比无好，且标注为降级产物）。
    // 旧版只检查非空——模型输出散文/scratchpad 泄漏/截断时，残缺内容原样成为唯一记忆。
    let mut validated = compression::extract_snapshot(&summary_text(&summary));
    if validated.is_none() {
        // 重试：附纠正指令，要求严格按 XML 结构输出
        let retry_msg = ChatMessage {
            id: short_id(),
            role: Some(MessageRole::User),
            msg_type: Some(MessageType::Text),
            content: Some(MessageContent::Text(
                "Your previous reply did not contain a valid <state_snapshot> XML block. \
                 Reply again with ONLY the <state_snapshot> block, following the requested structure."
                    .to_string(),
            )),
            status: Some(MessageStatus::Completed),
            ..Default::default()
        };
        context.messages.push(retry_msg);
        let retry = send_compression_request(
            orchestrator,
            &compression_prompt,
            &context.messages,
            &short_id(),
            channel,
            abort_flag,
        )
        .await;
        if let Ok(s) = retry {
            if let Some(snapshot) = compression::extract_snapshot(&summary_text(&s)) {
                validated = Some(snapshot);
                summary = s;
            }
        }
    }
    let snapshot_text = validated
        .or_else(|| compression::fallback_snapshot(&summary_text(&summary)))
        .unwrap_or_default();
    if snapshot_text.is_empty() {
        // 连兜底都拿不到内容（两次请求均为空流）：回滚，下一轮再试
        context.messages = original_messages;
        return None;
    }
    context.messages.clear();

    // 落库前渲染为纯文本分节（诉求3：历史中不残留 XML 标签，切断格式模仿链）
    let snapshot_display = compression::render_snapshot_for_history(&snapshot_text);
    // 快照消息：meta 记录压缩标记、压缩后估算（迟滞依据）、转存路径。
    // post_tokens 口径 = 压缩完成后的内容水位（快照 + 保留区内容，不含请求级
    // overhead），与 should_start_compression 迟滞比较的读取侧对齐。旧口径只算
    // 快照、漏掉保留区，迟滞地板被低估 → 压缩后很快再次越线 → 循环压缩。
    let post_tokens = compression::estimate_message_tokens(&ChatMessage {
        content: Some(MessageContent::Text(snapshot_display.clone())),
        ..Default::default()
    }) + keep_messages
        .iter()
        .map(compression::estimate_message_tokens)
        .sum::<usize>();
    let mut meta = serde_json::json!({
        "compacted": true,
        "post_tokens": post_tokens,
    });
    if let Some(p) = &transcript_path {
        meta["transcript_path"] = serde_json::json!(p);
    }
    if let Some(hints) = extra_hints {
        if !hints.trim().is_empty() {
            meta["compact_hints"] = serde_json::json!(hints);
        }
    }

    // P2-3：快照指纹 —— 记录压缩协议版本与提示词指纹，
    // 使"提示词强化是否生效"可从产物侧（快照 meta）验证。
    meta["protocol_version"] = serde_json::json!(super::compression::COMPRESSION_PROTOCOL_VERSION);
    meta["prompt_fingerprint"] =
        serde_json::json!(super::compression::compression_prompt_fingerprint());

    let snapshot_message = ChatMessage {
        id: root_id.to_string(),
        // 快照作为压缩后的首条消息，必须是 user 角色（多数 provider 要求对话以 user 开头）
        role: Some(MessageRole::User),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text(format!(
            "[CONTEXT SNAPSHOT — 压缩的历史记忆，基于它继续任务]\n{snapshot_display}"
        ))),
        status: Some(MessageStatus::Completed),
        meta: Some(meta),
        ..Default::default()
    };

    let mut new_messages = vec![snapshot_message];
    new_messages.extend(keep_messages);
    context.messages = new_messages;

    let _ = context
        .session
        .replace_messages(context.messages.clone())
        .await;

    Some(post_tokens)
}

/// 取消息纯文本（快照校验用）
fn summary_text(m: &ChatMessage) -> String {
    m.content.as_ref().map(|c| c.to_text()).unwrap_or_default()
}

/// 主动压缩工具（context_compact）执行体——目标四的核心。
///
/// 关键正确性约束：**进行中的 Turn 必须从其用户指令起整体保留**。
/// 调用时本 Turn 的 ToolCall 消息已在上下文中（请求后 extend），但其工具结果
/// 子节点尚未产生。切分点必须落在当前用户指令（Turn 根之前）：
/// - 保留区 = [用户指令, Turn, 子节点...]，Turn 子树 parent 链完整；
/// - 若切在 Turn 根与 ToolCall 之间：Turn 根入快照、子 ToolCall 留保留区 →
///   parent 悬空 → 孤儿消息被 drop_orphan_messages 剔除 → 请求包里 Tool 结果
///   失去父节点 → provider 400。
///
/// `split_user_idx`：主循环传入的切分下标（当前用户指令，见 find_turn_user_split_idx）。
/// 返回 `(是否执行了压缩, 估算压缩前 tokens, 估算压缩后 tokens)`。
async fn run_context_compact(
    orchestrator: &ChatOrchestrator,
    context: &mut SessionContext,
    channel: &mut PluginChannel,
    ctx: &Arc<dyn InvokeRequest>,
    abort_flag: &Arc<AtomicBool>,
    split_user_idx: usize,
    hints: Option<&str>,
) -> (bool, usize, usize) {
    if split_user_idx == 0 {
        return (false, 0, 0);
    }

    // 待压缩历史 = [.., split_user_idx)，当前用户指令起的任务上下文整体留在保留区
    let history: Vec<ChatMessage> = context.messages[..split_user_idx].to_vec();
    let before_tokens: usize = history
        .iter()
        .map(compression::estimate_message_tokens)
        .sum();
    if before_tokens < compression::MIN_COMPACT_TOKENS {
        return (false, before_tokens, before_tokens);
    }

    let keep_messages: Vec<ChatMessage> = context.messages[split_user_idx..].to_vec();
    let compression_msg = compression::build_compression_request(&history, hints);

    // 压缩流水线与被动自动压缩共用同一核心（transcript 转存 → LLM 压缩请求 →
    // 快照校验/纠正重试/降级兜底 → meta 构造 → 保留区拼接落库）；失败已在核心内
    // 就地回滚，这里只把它翻译为工具结果的 (compressed, before, after) 三元组。
    let post_tokens = compress_with_snapshot_core(
        orchestrator,
        context,
        channel,
        ctx,
        abort_flag,
        compression_msg,
        keep_messages,
        hints,
        "manual",
    )
    .await;

    match post_tokens {
        Some(after) => (true, before_tokens, after),
        // 未执行 / 失败：压缩放弃（历史已回滚），工具结果如实反映无收益
        None => (false, before_tokens, before_tokens),
    }
}

/// 压缩前把完整历史转存为 JSON transcript（best-effort）。
/// 落在会话存储目录内（`<homedir>/plugins/session/<id>/transcripts/`，跟随会话生命周期），
/// 而非系统临时目录（旧存档的教训：无 GC、跨会话堆积、脱离会话管理）。
/// 路径派生统一走 paths 模块（safe_id / 会话根目录的唯一权威实现）。
fn save_transcript_archive(messages: &[ChatMessage], session_id: &str) -> Option<String> {
    let root = super::paths::session_subdir(session_id, super::paths::TRANSCRIPTS_SUBDIR);
    std::fs::create_dir_all(&root).ok()?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let n = messages.len();
    let path = root.join(format!("transcript_{ts}_{n}.json"));
    let body = serde_json::to_string_pretty(messages).ok()?;
    std::fs::write(&path, body).ok()?;
    path.to_str().map(|s| s.to_string())
}

#[allow(clippy::too_many_arguments)]
async fn send_compression_request(
    orchestrator: &ChatOrchestrator,
    system_prompt: &str,
    messages: &[ChatMessage],
    root_id: &str,
    channel: &mut PluginChannel,
    abort_flag: &Arc<AtomicBool>,
) -> Result<ChatMessage, PluginError> {
    // 压缩是**内部 LLM 请求**，不是对话轮次：其流式帧（Turn 起始 / 思考 / 正文 delta）
    // 绝不能进入对话流——否则前端会多出一个永远停在"正在思考…"的空 Turn（压缩请求
    // 从不 finalize，快照也只落库不广播），且随每次自动压缩/主动压缩逐个累积。
    // 长会话才会触发压缩，因此该泄漏只在长任务后复现，极易误判为渲染层问题。
    //
    // 通道隔离的不对称设计：
    // - **tx（出帧）完全静默**：哑 sender + drain task，压缩 delta 一律丢弃（编译期
    //   不可泄漏——本函数内所有 emit 都走 muted.tx）；
    // - **rx（入帧）临时移交真实主通道**：用户停止时 Abort 帧只会进入主通道队列，
    //   而消费循环此刻正 await 在压缩请求上——若 rx 也是哑的，Abort 永远收不到，
    //   压缩请求将无视中止跑完整整轮 LLM 流（此前还曾因哑 rx 立即关闭被误判
    //   Aborted，导致每轮重试巨型压缩请求）。压缩结束后 rx 归还主通道。
    let (mute_tx, mut mute_rx) = tokio::sync::mpsc::channel::<PluginFrame>(64);
    tokio::spawn(async move { while mute_rx.recv().await.is_some() {} });
    let dummy_rx = tokio::sync::mpsc::channel::<PluginFrame>(1).1;
    // 出栈时通过 mem::replace 归还真实 rx（下方统一在请求结束后归还）
    let real_rx = std::mem::replace(&mut channel.rx, dummy_rx);
    let mut muted = PluginChannel {
        tx: mute_tx,
        rx: real_rx,
        cancel_token: tokio_util::sync::CancellationToken::new(),
    };

    // 压缩请求窗口日志：此窗口内出帧静默、消费循环收不到任何流式帧，
    // 若无日志，长压缩请求表现为"整段时间无任何输出"（用户视角的卡死）。
    // 压缩走完整 LLM 流，长上下文时可能耗时数分钟，必须显式标注开始/结束。
    let compression_started = std::time::Instant::now();
    crate::plugin_info!(
        "session",
        "[Compress] 压缩 LLM 请求开始：root_id={root_id}, 历史消息数={}，约 {} 字符（此窗口内前端无流式输出属正常）",
        messages.len(),
        messages.iter().map(|m| m.content.as_ref().map(|c| c.to_text().len()).unwrap_or(0)).sum::<usize>()
    );

    let result = run_compression_llm(
        orchestrator,
        system_prompt,
        messages,
        root_id,
        &mut muted,
        abort_flag,
    )
    .await;

    match &result {
        Ok(msg) => {
            let text_len = msg.content.as_ref().map(|c| c.to_text().len()).unwrap_or(0);
            crate::plugin_info!(
                "session",
                "[Compress] 压缩 LLM 请求完成：摘要 {} 字符，耗时 {}s",
                text_len,
                compression_started.elapsed().as_secs()
            );
        }
        Err(e) => {
            crate::plugin_error!(
                "session",
                "[Compress] 压缩 LLM 请求失败（耗时 {}s）：{e}",
                compression_started.elapsed().as_secs()
            );
        }
    }

    // 无论成败，立即把真实 rx 归还主通道（Abort 帧的消费权交还消费循环）
    let dummy_rx = tokio::sync::mpsc::channel::<PluginFrame>(1).1;
    channel.rx = std::mem::replace(&mut muted.rx, dummy_rx);

    result
}

/// 压缩摘要的实际 LLM 调用：出帧全部静默（muted.tx），入帧收真实主通道 Abort。
///
/// 注意：这里**绝不发射 Turn 帧**（不发 emit_streaming_start）。压缩是内部请求、
/// 不是对话轮次——Turn 帧在哑通道上是纯死代码，而历史上它曾走主通道泄漏，在前端
/// 留下永远"正在思考…"的空 Turn 骨架（每轮压缩尝试累积一个）。从源头删除调用点，
/// 使"内部请求泄漏可见帧"这一类问题在结构上不可能再发生。
async fn run_compression_llm(
    orchestrator: &ChatOrchestrator,
    system_prompt: &str,
    messages: &[ChatMessage],
    root_id: &str,
    muted: &mut PluginChannel,
    abort_flag: &Arc<AtomicBool>,
) -> Result<ChatMessage, PluginError> {
    use crate::symbio_core::schemas::session::chat_message::MessageContent;

    // 压缩路径与对话轮次共用同一模型契约：provider.execute_turn（tools 为空）。
    // 出帧仍全部静默（muted.tx），入帧收真实主通道 Abort——语义与此前手动
    // prepare_request + execute_post_with_abort + parse_sse_stream 组合一致，
    // 但协议细节（请求构造 / 重试 / SSE 解析）收敛进 model 插件实现。
    let out = orchestrator
        .provider
        .execute_turn(system_prompt, messages, &[], root_id, muted, abort_flag)
        .await?;

    if abort_flag.load(Ordering::SeqCst) {
        return Err(PluginError::Aborted);
    }

    let effective = out.effective_text(0).to_owned();
    if effective.is_empty() {
        return Err(PluginError::InternalError(
            // 语义说明：压缩摘要请求的 SSE 流正常结束，但未产出任何文本/推理内容
            // （常见于上游网关错误被吞掉、或模型只回了空流）。此前误用
            // "Compression produced empty result"，与上下文压缩的语义完全对不上，
            // 已在 auto_compress_process 中改为优雅降级，不再中断整个 turn。
            "Compression summary request returned no content".to_string(),
        ));
    }

    Ok(ChatMessage {
        id: root_id.to_string(),
        role: Some(crate::symbio_core::schemas::session::chat_message::MessageRole::Assistant),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text(effective)),
        status: Some(MessageStatus::Completed),
        ..Default::default()
    })
}

/// 触发 Stop 钩子（委托给 [`StopSignal`]，幂等；上下文已在信号创建时绑定）。
///
/// `run_chat_loop` 的**每一个**出口都应先调用本函数（软上限出口用
/// `context.messages`，其余用入参 `messages`），以携带准确的 `last_message`。
/// 即便全部出口都漏调，`StopSignal::drop` 也会兜底补发一次——不变式
/// 「一个请求生命周期内 Stop 恰好一次」因此由生命周期保证，而非依赖人工记忆。
async fn fire_stop_hook(orchestrator: &ChatOrchestrator, messages: &[ChatMessage]) {
    orchestrator.stop.fire(messages).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::{CapabilityVisitor, DefaultToolVisitor, SimpleRequest};

    /// 构造带 CAPABILITY_VISITOR 的上下文；`prompts` 为 (名称, 内容) 注册表。
    async fn ctx_prompts(prompts: &[(&str, &str)]) -> Arc<dyn InvokeRequest> {
        let ctx: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
        let visitor = Arc::new(DefaultToolVisitor::new());
        for (name, text) in prompts {
            visitor.register_system_prompt(name, text.to_string()).await;
        }
        ctx.set(
            crate::symbio_core::CAPABILITY_VISITOR,
            visitor as Arc<dyn CapabilityVisitor>,
        );
        ctx
    }

    #[tokio::test]
    async fn explicit_prompt_wins() {
        let ctx = ctx_prompts(&[("default", "from-visitor")]).await;
        let got = resolve_system_prompt(Some("explicit"), Some("openai"), &ctx).await;
        assert_eq!(got, "explicit");
    }

    #[tokio::test]
    async fn empty_explicit_prompt_falls_through_to_default() {
        let ctx = ctx_prompts(&[("openai", "from-openai"), ("default", "from-default")]).await;
        let got = resolve_system_prompt(Some(""), Some("openai"), &ctx).await;
        assert_eq!(got, "from-default", "default 优先于 provider_id 匹配");
    }

    #[tokio::test]
    async fn provider_id_matches_registered_key() {
        let ctx = ctx_prompts(&[("anthropic", "a"), ("openai", "o")]).await;
        let got = resolve_system_prompt(None, Some("openai"), &ctx).await;
        assert_eq!(got, "o");
    }

    #[tokio::test]
    async fn first_registered_when_no_default_or_provider_match() {
        let ctx = ctx_prompts(&[("anthropic", "a"), ("openai", "o")]).await;
        let got = resolve_system_prompt(None, Some("unknown"), &ctx).await;
        assert_eq!(got, "a", "保序取首个注册项");
    }

    #[tokio::test]
    async fn fallback_without_visitor() {
        let ctx: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
        let got = resolve_system_prompt(None, None, &ctx).await;
        assert_eq!(got, "You are a helpful MODEL assistant.");
    }

    #[tokio::test]
    async fn empty_visitor_registry_uses_fallback() {
        let ctx = ctx_prompts(&[]).await;
        let got = resolve_system_prompt(None, None, &ctx).await;
        assert_eq!(got, "You are a helpful MODEL assistant.");
    }
}

/// Stop 恰好一次的契约测试（session-mechanism-unification.md §4.4）。
///
/// 走真实 `fire_hook` 链路（自建 recorder 插件，不 mock 内部函数），
/// 验证显式触发 / 生命周期兜底 / 二者叠加时的幂等性。
#[cfg(test)]
mod stop_signal_tests {
    use super::*;
    use crate::symbio_core::{InvokeResponse, PluginMeta, PluginPayload, SimpleRequest};
    use async_trait::async_trait;
    use serde_json::Value;
    use std::sync::Mutex as StdMutex;

    /// 记录收到的 Hook 事件（按 `fire_hook` 的 payload 协议解析）。
    #[derive(Default)]
    struct HookRecorder {
        stops: StdMutex<Vec<String>>,
        others: StdMutex<usize>,
    }

    impl HookRecorder {
        fn stop_count(&self) -> usize {
            self.stops.lock().unwrap().len()
        }
        fn last_stop_message(&self) -> String {
            self.stops
                .lock()
                .unwrap()
                .last()
                .cloned()
                .unwrap_or_default()
        }
        fn other_count(&self) -> usize {
            *self.others.lock().unwrap()
        }
    }

    #[async_trait]
    impl Plugin for HookRecorder {
        fn meta(&self) -> PluginMeta {
            PluginMeta {
                id: "test-hook-recorder".into(),
                name: "test-hook-recorder".into(),
                description: None,
                version: None,
                author: None,
            }
        }

        async fn route(
            self: Arc<Self>,
            ctx: Arc<dyn InvokeRequest>,
        ) -> InvokeResponse<PluginPayload> {
            if ctx.get(crate::symbio_core::PATH).as_deref() != Some("hook/fire") {
                *self.others.lock().unwrap() += 1;
                return Ok(PluginPayload::new(&Value::Null));
            }
            let payload: Value = ctx.payload::<Value>().ok().unwrap_or(Value::Null);
            let event = payload.get("event");
            if event
                .and_then(|e: &Value| e.get("event"))
                .and_then(|v: &Value| v.as_str())
                == Some("Stop")
            {
                let msg = event
                    .and_then(|e: &Value| e.get("data"))
                    .and_then(|d: &Value| d.get("last_message"))
                    .and_then(|v: &Value| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                self.stops.lock().unwrap().push(msg);
            } else {
                *self.others.lock().unwrap() += 1;
            }
            Ok(PluginPayload::new(&Value::Null))
        }

        async fn traverse(
            self: Arc<Self>,
            _path: String,
            _ctx: Arc<dyn InvokeRequest>,
        ) -> InvokeResponse<PluginPayload> {
            Ok(PluginPayload::new(&Value::Null))
        }
    }

    fn text_msg(id: &str, text: &str) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            content: Some(MessageContent::Text(text.to_string())),
            ..Default::default()
        }
    }

    fn recorder_signal() -> (Arc<StopSignal>, Arc<HookRecorder>) {
        let recorder = Arc::new(HookRecorder::default());
        let parent = Some(recorder.clone() as Arc<dyn Plugin>);
        let ctx: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
        (Arc::new(StopSignal::new(parent, ctx)), recorder)
    }

    /// 显式触发：携带准确末条消息，且第二次调用不再外发。
    #[tokio::test]
    async fn explicit_fire_is_exactly_once_and_carries_last_message() {
        let (stop, recorder) = recorder_signal();
        assert!(!stop.fired());

        let msgs = vec![text_msg("1", "user turn"), text_msg("2", "assistant turn")];
        assert!(stop.fire(&msgs).await, "首次显式触发应生效");
        assert!(!stop.fire(&msgs).await, "第二次显式触发应被幂等吞掉");
        assert!(stop.fired());

        assert_eq!(recorder.stop_count(), 1);
        assert_eq!(recorder.last_stop_message(), "assistant turn");
        assert_eq!(recorder.other_count(), 0);

        drop(stop);
        assert_eq!(recorder.stop_count(), 1, "Drop 不得重复补发");
    }

    /// 空 transcript：仍然触发一次，`last_message` 为空串。
    #[tokio::test]
    async fn explicit_fire_with_empty_transcript() {
        let (stop, recorder) = recorder_signal();
        assert!(stop.fire(&[]).await);
        assert_eq!(recorder.stop_count(), 1);
        assert_eq!(recorder.last_stop_message(), "");
    }

    /// 兜底触发：显式触发点未执行时补发一次（异步 detached 任务）。
    #[tokio::test]
    async fn fallback_fire_covers_missing_explicit_call() {
        let (stop, recorder) = recorder_signal();
        stop.fire_fallback();
        // fire_fallback 投递 detached 任务，让运行时调度若干次
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert_eq!(recorder.stop_count(), 1);
        assert_eq!(recorder.last_stop_message(), "");

        // 兜底之后再显式触发也不得外发
        assert!(!stop.fire(&[text_msg("1", "late")]).await);
        assert_eq!(recorder.stop_count(), 1);
    }

    /// 兜底幂等：多次 `fire_fallback` 只外发一次。
    #[tokio::test]
    async fn fallback_fire_is_idempotent() {
        let (stop, recorder) = recorder_signal();
        stop.fire_fallback();
        stop.fire_fallback();
        stop.fire_fallback();
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert_eq!(recorder.stop_count(), 1);
    }

    /// 最后防线：`StopSignal` 被 Drop 时补发（模拟 chat_loop 提前 return）。
    #[tokio::test]
    async fn drop_emits_fallback_stop() {
        let (stop, recorder) = recorder_signal();
        drop(stop);
        for _ in 0..10 {
            tokio::task::yield_now().await;
        }
        assert_eq!(recorder.stop_count(), 1);
    }

    /// 无父插件（standalone 会话）：仍然置位 fired，且不产生任何外发/告警噪声。
    #[tokio::test]
    async fn signal_without_parent_stays_silent() {
        let ctx: Arc<dyn InvokeRequest> = Arc::new(SimpleRequest::new(None, None));
        let stop = Arc::new(StopSignal::new(None, ctx));
        assert!(stop.fire(&[text_msg("1", "x")]).await);
        assert!(stop.fired());
        stop.fire_fallback();
        drop(stop);
        tokio::task::yield_now().await;
    }
}
