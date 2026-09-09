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

use crate::plugin_info;
use crate::plugin_warn;
use crate::symbio_core::schemas::{
    model::model_chat,
    model::model_config::ModelConfig,    session::chat_message::{
        assign_seq, max_seq, ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
    },
    system::hook::HookEvent,
};
use crate::symbio_core::{
    ChatSession, InvokeRequest, InvokeRequestExt, ModelProvider, Plugin, PluginChannel,
    PluginError, PluginFrame, SESSION_HANDLE,
};
use crate::symbio_core::turn::{
    build_tool_message, emit_status, emit_update, execute_post_with_abort, parse_sse_stream,
    short_id, PostResult, ToolCallInfo, TurnOutput,
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

/// 会话编排器（自 model/context.rs 迁入，Phase E-②）：
/// 持有模型配置、父插件钩子通道与协议适配器（session 确定性持有）。
/// 轮次收尾状态机 `finalize_assistant_turn` 随之一并迁入；
/// turn_processor 薄委托层消亡（chat_loop 直调
/// `protocol.execute_turn` 与 `finalize_assistant_turn`）。
pub struct ChatOrchestrator {
    pub config: ModelConfig,
    pub parent: Option<Arc<dyn Plugin>>,
    /// Phase E-②：协议适配器经 `ModelProviderEntry.provider`（`Arc<dyn ModelProvider>`）
    /// 从 CAPABILITY_MANAGER 取得，这里持有 Arc 共享引用（原为 Box 独占）。
    pub protocol: Arc<dyn ModelProvider>,
}

impl ChatOrchestrator {
    pub fn new(
        config: ModelConfig,
        parent: Option<Arc<dyn Plugin>>,
        protocol: Arc<dyn ModelProvider>,
    ) -> Self {
        Self {
            config,
            parent,
            protocol,
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

pub async fn run_chat_loop(
    orchestrator: &ChatOrchestrator,
    ctx: Arc<dyn InvokeRequest>,
    mut channel: PluginChannel,
) -> Result<(), PluginError> {
    let mut req: model_chat::Request = ctx.payload()?;

    plugin_info!("session",
        ">>> NEW SESSION START (Protocol: {:?})",
        orchestrator.config.api_protocol
    );
    plugin_info!("session",
        "[DIAG] run_chat_loop: configured_max_tool_rounds={:?} (None=无上限), auto_compress={}, enable_compact_tool={}, msg_id_in_payload={:?}",
        req.max_tool_rounds,
        req.auto_compress.unwrap_or(true),
        req.enable_compact_tool.unwrap_or(false),
        req.single_message.as_ref().map(|m| m.id.clone())
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
    //   重新执行工具、创建新结果子节点并持久化。CAPABILITY_MANAGER 已由 agent chat
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
                fire_stop_hook(orchestrator, &[], &ctx).await;
                return Ok(());
            }
            Err(e) => {
                plugin_warn!("session", "[Resume] process_resume failed: {}", e);
                fire_stop_hook(orchestrator, &[], &ctx).await;
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
                plugin_info!("session",
                    "[DIAG] run_chat_loop: 达到显式设置的上限 max_tool_rounds={}", max
                );
                let _ = channel.tx.send(PluginFrame::Data(
                    serde_json::to_value(session_chat_response::StreamEvent::Error {
                        error: format!(
                            "已达到本轮工具调用上限（{max}）。如需继续，请再次发送消息。"
                        ),
                    })
                    .unwrap_or_default(),
                )).await;
                persist_messages(&context, last_saved, &channel).await;
                fire_stop_hook(orchestrator, &context.messages, &ctx).await;
                return Ok(());
            }
        }

        if abort_flag.load(Ordering::SeqCst) {
            // SYS-002: 早期 return 路径上的副作用（last_saved 尚未用作流式增量锚点，
            // 此分支里不更新，但保留 last_saved 维持语义对称）。
            plugin_info!("session",
                "[DIAG] run_chat_loop: abort_flag true at top of turn {}",
                tool_rounds
            );
            fire_stop_hook(orchestrator, &context.messages, &ctx).await;
            return Ok(());
        }

        plugin_info!("session", "--- TURN {} START ---", tool_rounds);

        if check_abort(&abort_flag).await {
            plugin_info!("session",
                "[DIAG] run_chat_loop: check_abort returned true at turn {}",
                tool_rounds
            );
            fire_stop_hook(orchestrator, &context.messages, &ctx).await;
            return Ok(());
        }

        // ── 被动语义压缩（L5：70% 触发）────────────────────────────────
        // 系统提示词解析（Phase B 统一收集机制）：
        // 1. 请求显式指定（req.system_prompt）
        // 2. 统一收集机制注册的系统提示词：优先 "default" 键，其次请求指定的
        //    provider_id 键，再退首个注册项
        // 3. 硬编码兜底（维持既有行为）
        let system_prompt_owned = match req.system_prompt.as_deref() {
            Some(sp) => sp.to_string(),
            None => {
                let collected = match ctx.get(crate::symbio_core::CAPABILITY_MANAGER) {
                    Some(tool_manager) => tool_manager.list_system_prompts().await,
                    None => Vec::new(),
                };
                collected
                    .iter()
                    .find(|(n, _)| n == "default")
                    .or_else(|| {
                        collected
                            .iter()
                            .find(|(n, _)| Some(n.as_str()) == req.provider_id.as_deref())
                    })
                    .or_else(|| collected.first())
                    .map(|(_, p)| p.clone())
                    .unwrap_or_else(|| "You are a helpful MODEL assistant.".to_string())
            }
        };
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
                    plugin_info!("session",
                        "Context compressed: {} messages -> 1 message",
                        history_count
                    );
                    last_saved = context.messages.len();
                }
                Ok(None) => {}
                Err(e) => {
                    plugin_info!("session",
                        "[DIAG] run_chat_loop: auto_compress_process Err({})",
                        e
                    );
                    fire_stop_hook(orchestrator, &context.messages, &ctx).await;
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
            let effective_limit = (orchestrator.config.max_context_tokens
                - orchestrator.config.reserved_tokens)
                as usize;
            let overhead =
                compression::estimate_request_overhead(system_prompt_for_request, &ctx).await;
            if compression::should_emit_context_nudge(&context.messages, effective_limit, overhead)
            {
                nudged_this_request = true;
                inject_nudge = true;
                plugin_info!("session",
                    "[Compress] context nudge emitted (~55% of limit), suggesting context_compact"
                );
            }
        }

        let root_id: String = short_id();
        emit_streaming_start(&mut channel, &root_id, Some(tool_rounds)).await;

        apply_message_level_compression(&ctx, &mut context.messages).await;

        let mut tools = if let Some(tool_manager) = ctx.get(crate::symbio_core::CAPABILITY_MANAGER) {
            tool_manager.list_capability().await
        } else {
            Vec::new()
        };
        // 主动压缩工具（目标四）：仅当工具压缩启用时暴露给模型（独立于自动压缩开关）。
        // 执行不走 CapabilityManager 分发，由下方拦截逻辑处理（需要编排器内部链路）。
        if enable_compact_tool {
            tools.push(compression::context_compact_tool_meta());
        }

        // 请求视图（唯一入口 build_request_view）：存储视图之上叠加三项**不落库**的
        // 裁剪，全部只作用于本次 send_request 的请求包，不回写 context.messages——
        // 存储保持完整历史，last_saved 锚点与 persist_messages 切片不会错位。
        // 1) fade：轮次过多时淡化较早的工具结果（存储保留全文，视图每轮重建，天然幂等）；
        // 2) 工具级骨架化：从 CapabilityManager 的能力声明（context_retention）动态解析
        //    保留策略，LastOnly/LastN → 更早调用的参数与结果替换为占位文案
        //    （ToolCall↔Tool 配对完整保留，不会造成大模型逻辑断联）；
        // 3) nudge：水位提醒请求级注入（不落库、不占轮次窗口的 User 计数）。
        let request_view: Vec<ChatMessage> = {
            let window = req.tool_context_window.unwrap_or(15);
            let retention: std::collections::HashMap<String, crate::symbio_core::ToolContextRetention> =
                tools
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
                inject_nudge,
            )
        };
        let request_messages: &[ChatMessage] = request_view.as_slice();

        plugin_info!("session",
            "[DIAG] run_chat_loop: about to call protocol.execute_turn, ctx_msg_count={}, tool_count={}",
            context.messages.len(),
            tools.len()
        );

        // Turn 创建后的首个 abort 检查点：覆盖"压缩阶段中止"等 send_request
        // 之前置位的场景。压缩失败已就地降级（不冒泡），但 abort_flag 仍为
        // true 且 abort 帧已被压缩请求消费——若不在此拦截，execute_turn 会
        // 发起一次多余的 LLM 请求。此处 Turn 已创建（上方 emit_streaming_start），
        // 冒泡 Err(Aborted) → 消费循环 ABORTED 分支 → persist_failure 把本轮
        // Turn 收尾为 Failed + "用户手动中止了本次回复"（错误条 + 重试入口），
        // 不会波及上一轮已成功的 Turn（persist_failure 按 failing_turn 子树收窄）。
        if abort_flag.load(Ordering::SeqCst) {
            plugin_warn!("session",
                "[DIAG] run_chat_loop: abort_flag true before execute_turn, propagating Err(Aborted)"
            );
            fire_stop_hook(orchestrator, &context.messages, &ctx).await;
            return Err(PluginError::Aborted);
        }

        let result = orchestrator
            .protocol
            .execute_turn(
                &orchestrator.config,
                req.system_prompt
                    .as_deref()
                    .unwrap_or("You are a helpful MODEL assistant."),
                request_messages,
                &tools,
                &root_id,
                &mut channel,
                &abort_flag,
            )
            .await;

        plugin_info!("session",
            "[DIAG] run_chat_loop: protocol.execute_turn returned, is_ok={}",
            result.is_ok()
        );

        let mut out = match result {
            Err(PluginError::RetryWithoutContextId) => {
                plugin_info!("session",
                    "[DIAG] run_chat_loop: send_request -> RetryWithoutContextId, continuing"
                );
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
                plugin_warn!("session",
                    "[DIAG] run_chat_loop: send_request -> Aborted, propagating Err(Aborted)"
                );
                fire_stop_hook(orchestrator, &context.messages, &ctx).await;
                return Err(PluginError::Aborted);
            }
            Err(e) => {
                plugin_warn!("session",
                    "[DIAG] run_chat_loop: send_request -> Err({}), returning Err",
                    e
                );
                fire_stop_hook(orchestrator, &context.messages, &ctx).await;
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
            plugin_warn!("session",
                "[DIAG] run_chat_loop: abort_flag became true after send_request, propagating Err(Aborted)"
            );
            fire_stop_hook(orchestrator, &context.messages, &ctx).await;
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
        // 由 run_chat_loop 在构建 LLM 请求前从 CapabilityManager 动态解析，
        // 节点 name 即 LLM 可见工具名，与声明名直接匹配。
        context.messages.extend(new_msgs);

        // 被长度截断的 Turn 打标（供前端「继续」按钮与回溯），不再静默结束（修复"对话突然结束"）。
        if finish.is_length() {
            for m in context.messages.iter_mut() {
                if m.id == rtid || m.id == rrid {
                    let mut meta = m
                        .meta
                        .clone()
                        .unwrap_or_else(|| serde_json::json!({}));
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
                    plugin_info!("session",
                        "[DIAG] run_chat_loop: finish=Length，自动续写 ({}/{})",
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
            plugin_info!("session",
                "[DIAG] run_chat_loop: no tool calls, finalizing turn {}, text_added={}, returning Ok(())",
                tool_rounds,
                context.messages.len()
            );
            persist_messages(&context, last_saved, &channel).await;
            fire_stop_hook(orchestrator, &context.messages, &ctx).await;
            return Ok(());
        }

        // ── 主动压缩工具拦截（目标四）─────────────────────────────────
        // context_compact 不走 CapabilityManager 分发：它需要编排器内部的
        // 压缩链路（LLM 摘要 + 上下文替换 + 会话持久化）。
        // 在此拆分：压缩调用就地执行并生成合成工具结果；其余工具正常分发。
        // 门控：仅当工具压缩开关开启时拦截；开关关闭时工具不暴露，模型幻觉
        // 调用则归入标准工具链，以"未知路径"错误返回（不执行内部压缩链路）。
        let (compact_calls, other_calls): (Vec<_>, Vec<_>) = if enable_compact_tool {
            tools_done
                .into_iter()
                .partition(|tc| {
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
            let split_user_idx =
                compression::find_turn_user_split_idx(&context.messages, &root_id);
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
                    plugin_info!("session",
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
                let mut tool_msg = build_tool_message(
                    &call_id,
                    &result_text,
                    Some(ok),
                    None,
                );
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
        }) || parent_updates.iter().any(|p| {
            p.status == Some(MessageStatus::WaitingUserAction)
        });

        if needs_user_action {
            // 注：信息性策略下工具失败的父 ToolCall 已标 Completed（错误结果作为
            // 合法 tool 结果喂回 LLM，loop 不中断），不存在「Failed 父节点等待
            // 恢复」的场景——旧版在此处给 Failed+failure_kind 父节点打 recoverable
            // 标记的代码属不可达遗留，已删除（docs/turn-tool-mechanisms.md 1.5）。
            // user_prompt(WaitingUserAction) 驱动的暂停走 approve/reject/answer 恢复。
            plugin_info!("session",
                "[DIAG] run_chat_loop: 工具待用户恢复（mode={}），退出本轮",
                mode
            );
            fire_stop_hook(orchestrator, &context.messages, &ctx).await;
            return Ok(());
        }

        // 轮次计数：用户明确要求不设硬性上限，超长对话的规模控制由请求视图层的
        // fade / 骨架化（build_request_view）承担——存储保持完整历史，视图逐轮裁剪。
        tool_rounds += 1;
    }
}

async fn broadcast_message_update(channel: &PluginChannel, message: ChatMessage) {
    let _ = channel.tx.send(PluginFrame::Data(
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

    plugin_warn!("session",
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
    let effective_context_limit =
        (orchestrator.config.max_context_tokens - orchestrator.config.reserved_tokens) as usize;

    // 请求级固定开销（system prompt + 工具定义）必须计入阈值判断，
    // 否则上下文实际占用被低估，压缩触发过晚 → 撞 provider 的 context-length 400。
    let overhead = compression::estimate_request_overhead(system_prompt, ctx).await;

    if !compression::should_start_compression(&context.messages, effective_context_limit, force, overhead)
    {
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

    let original_count = context.messages.len();
    // 保存原始历史：压缩失败时回滚，绝不能让 `[compression_msg]` 残留在上下文里。
    let original_messages = context.messages.clone();
    let _ = fire_hook(&orchestrator.parent, HookEvent::PreCompact, ctx.clone()).await;

    // 可回溯原则：压缩前把完整历史转存为 transcript，路径记入快照 meta。
    // 旧版直接 replace_messages，被压掉的历史在物理层"凭空消失"，
    // 旧存档文件成为孤儿，事后无法审计。
    let transcript_path =
        save_transcript_archive(&original_messages, context.session.session_id());

    context.messages = vec![compression_msg];

    // 诉求3：专用压缩 system 提示词（模板只在本次请求出现，与主对话隔离；
    // system_prompt 参数仍用于请求开销估算）
    let compression_prompt = compression::get_compression_prompt();
    let root_id = short_id();
    let summary = match send_compression_request(
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
                "[Compress] auto compression failed ({}), falling back to uncompressed context",
                e
            );
            context.messages = original_messages;
            return Ok(None);
        }
    };

    // 快照校验：从输出提取 <state_snapshot>；缺失则纠正重试一次；
    // 仍失败则降级为纯文�快照（有总比无好，且标注为降级产物）。
    // 旧版只检查非空——模型输出散文/scratchpad 泄漏/截断时，残缺内容原样成为唯一记忆。
    let mut summary = summary;
    let mut validated = compression::extract_snapshot(&summary_text(&summary));
    if validated.is_none() {
        // 重试：附纠正指令，要求严格按 XML 结构输出
        let retry_msg = ChatMessage {
            id: uuid::Uuid::new_v4().to_string(),
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
        return Ok(None);
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
    }) + history_to_keep
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
    new_messages.extend(history_to_keep);
    context.messages = new_messages;

    let _ = context
        .session
        .replace_messages(context.messages.clone())
        .await;

    Ok(Some(original_count))
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
    let before_tokens: usize = history.iter().map(compression::estimate_message_tokens).sum();
    if before_tokens < compression::MIN_COMPACT_TOKENS {
        return (false, before_tokens, before_tokens);
    }

    let original_messages = context.messages.clone();
    let _ = fire_hook(&orchestrator.parent, HookEvent::PreCompact, ctx.clone()).await;
    let transcript_path =
        save_transcript_archive(&original_messages, context.session.session_id());

    let compression_msg = compression::build_compression_request(&history, hints);
    context.messages = vec![compression_msg];

    // 诉求3：专用压缩 system 提示词（模板只在本次请求出现，与主对话隔离）
    let compression_prompt = compression::get_compression_prompt();
    let summary = match send_compression_request(
        orchestrator,
        &compression_prompt,
        &context.messages,
        &short_id(),
        channel,
        abort_flag,
    )
    .await
    {
        Ok(s) => s,
        // 中止/限流透传主流程；其他失败回滚（工具结果按"压缩失败"返回）。
        Err(_e) => {
            context.messages = original_messages;
            return (false, before_tokens, before_tokens);
        }
    };

    // 快照校验：提取 → 纠正重试一次 → 降级兜底（与被动压缩同一链路）。
    let mut validated = compression::extract_snapshot(&summary_text(&summary));
    if validated.is_none() {
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
        if let Ok(s) = send_compression_request(
            orchestrator,
            &compression_prompt,
            &context.messages,
            &short_id(),
            channel,
            abort_flag,
        )
        .await
        {
            if let Some(snapshot) = compression::extract_snapshot(&summary_text(&s)) {
                validated = Some(snapshot);
            }
        }
    }
    let snapshot_text = match validated {
        Some(s) => s,
        None => match compression::fallback_snapshot(&summary_text(&summary)) {
            Some(s) => s,
            None => {
                // 两次均无有效输出：回滚，压缩放弃
                context.messages = original_messages;
                return (false, before_tokens, before_tokens);
            }
        },
    };

    // 新上下文 = [快照] + [当前用户指令 + 进行中 Turn 及之后]
    // 落库前渲染为纯文本分节（诉求3：历史中不残留 XML 标签，切断格式模仿链）
    // post_tokens 口径 = 压缩完成后的内容水位（快照 + 保留区内容，不含请求级
    // overhead），与 should_start_compression 迟滞比较的读取侧对齐；返回值中的
    // after_tokens 同样按此口径，压缩前后日志才反映真实收益。
    let snapshot_display = compression::render_snapshot_for_history(&snapshot_text);
    let post_tokens = compression::estimate_message_tokens(&ChatMessage {
        content: Some(MessageContent::Text(snapshot_display.clone())),
        ..Default::default()
    }) + original_messages[split_user_idx..]
        .iter()
        .map(compression::estimate_message_tokens)
        .sum::<usize>();
    let mut meta = serde_json::json!({ "compacted": true, "post_tokens": post_tokens });
    if let Some(p) = &transcript_path {
        meta["transcript_path"] = serde_json::json!(p);
    }
    if let Some(h) = hints {
        if !h.trim().is_empty() {
            meta["compact_hints"] = serde_json::json!(h);
        }
    }

    let snapshot_message = ChatMessage {
        id: short_id(),
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
    // 保留区：当前用户指令 + 进行中 Turn（含本批 ToolCall）及之后的一切
    new_messages.extend_from_slice(&original_messages[split_user_idx..]);
    context.messages = new_messages;

    let _ = context
        .session
        .replace_messages(context.messages.clone())
        .await;

    (true, before_tokens, post_tokens)
}

/// 压缩前把完整历史转存为 JSON transcript（best-effort）。
/// 落在会话存储目录内（`<homedir>/plugins/session/<id>/transcripts/`，跟随会话生命周期），
/// 而非系统临时目录（旧存档的教训：无 GC、跨会话堆积、脱离会话管理）。
fn save_transcript_archive(messages: &[ChatMessage], session_id: &str) -> Option<String> {
    use crate::symbio_core::HomedirRegistry;

    let safe_id = session_id.replace(['/', '\\', ':'], "_");
    let root = HomedirRegistry::get()
        .join("plugins")
        .join("session")
        .join(safe_id)
        .join("transcripts");
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
    use crate::symbio_core::schemas::session::chat_message::MessageContent;

    emit_streaming_start(channel, root_id, None).await;

    let turn_config = orchestrator.config.clone();
    let body = orchestrator
        .protocol
        .prepare_request(&turn_config, system_prompt, messages, &[]);

    let response = match execute_post_with_abort(
        &orchestrator.protocol.get_api_url(&turn_config),
        orchestrator.protocol.get_headers(&turn_config),
        &body,
        channel,
        abort_flag,
    )
    .await
    {
        PostResult::Aborted => return Err(PluginError::Aborted),
        PostResult::RetryWithoutContextId => return Err(PluginError::RetryWithoutContextId),
        PostResult::Err(e) => return Err(PluginError::InternalError(e)),
        PostResult::RateLimited(e) => return Err(PluginError::RateLimited(e)),
        PostResult::Ok(resp) => resp,
    };

    let out = parse_sse_stream(
        response,
        root_id,
        channel,
        abort_flag,
        orchestrator.protocol.as_ref(),
    )
    .await
    .map_err(PluginError::StreamError)?;

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

async fn apply_message_level_compression(
    ctx: &Arc<dyn InvokeRequest>,
    messages: &mut [ChatMessage],
) {
    compression::compress_temporary_messages(ctx, messages).await;
}

async fn fire_stop_hook(
    orchestrator: &ChatOrchestrator,
    messages: &[ChatMessage],
    ctx: &Arc<dyn InvokeRequest>,
) {
    let last_message = messages
        .last()
        .map(|m| m.content.as_ref().map(|c| c.to_text()).unwrap_or_default())
        .unwrap_or_default();

    let _ = fire_hook(
        &orchestrator.parent,
        HookEvent::Stop {
            last_message: last_message.to_string(),
        },
        ctx.clone(),
    )
    .await;
}
