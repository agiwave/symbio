//! MODEL 聊天主循环
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
    session::chat_message::{
        assign_seq, max_seq, ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
    },
    session::session_open,
    system::hook::HookEvent,
};
use crate::symbio_core::{
    ChatSession, ChatSessionHandle, InvokeRequest, InvokeRequestExt, PluginChannel, PluginError,
    PluginFrame, PluginPayload, SESSION_OPEN,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::compression;
use super::context::{compress_temporary_messages, ChatOrchestrator, PostResult};
use super::message_builder::short_id;
use super::protocol::execute_post_with_abort;
use super::tool_executor::{fire_hook, process_tool_calls_async};
use super::turn_processor::TurnProcessor;
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

pub async fn run_chat_loop(
    orchestrator: &ChatOrchestrator,
    ctx: Arc<dyn InvokeRequest>,
    mut channel: PluginChannel,
) -> Result<(), PluginError> {
    let mut req: model_chat::Request = ctx.payload()?;

    plugin_info!(
        "model",
        ">>> NEW SESSION START (Protocol: {:?})",
        orchestrator.config.api_protocol
    );
    plugin_info!(
        "model",
        "[DIAG] run_chat_loop: configured_max_tool_rounds={:?} (None=无上限), auto_compress={}, msg_id_in_payload={:?}",
        req.max_tool_rounds,
        req.auto_compress.unwrap_or(true),
        req.single_message.as_ref().map(|m| m.id.clone())
    );

    // 用户明确要求**不要**设置 max_tool_rounds 硬性上限（智能体会话轮次越来越多）。
    // 因此默认（request 未显式给出）=「无上限」；仅在调用方**显式**设置时才作为软上限并给出提示。
    let configured_max_tool_rounds = req.max_tool_rounds;
    let auto_compress = req.auto_compress.unwrap_or(true);

    let mut tool_rounds: usize = 0;
    let mut continuation_count: u32 = 0;
    // 水位提醒（nudge）一次性标记：每次用户请求生命周期内最多注入一次；
    // 主动压缩成功后重置（上下文回落后允许再次提醒）。
    let mut nudged_this_request = false;
    const MAX_CONTINUE_ROUNDS: u32 = 3;
    // 轮次老化淡化的激活阈值与保留窗口：超过该轮次后，较早的工具结果逐步 head/tail 摘要，
    // 始终保持最近 K 轮的原文与全部 assistant 文本/推理，最小化对思维链的破坏。
    const FADE_ACTIVATE_ROUNDS: usize = 40;
    const FADE_KEEP_RECENT_TURNS: usize = 12;

    let session = open_chat_session(&orchestrator.parent, &ctx).await;
    let mut single_message = req.single_message;
    let mut context = SessionContext {
        messages: Vec::new(),
        session,
    };

    let abort_flag = Arc::new(AtomicBool::new(false));
    let turn_processor = TurnProcessor::new(orchestrator);

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
        match crate::plugins::model::resume::process_resume(
            orchestrator,
            &ctx,
            &mut channel,
            &abort_flag,
            &context.session,
            tr,
        )
        .await
        {
            Ok(crate::plugins::model::resume::ResumeOutcome::Continue) => {
                // 成功：turn 循环会从 session 加载含新工具结果的历史
            }
            Ok(crate::plugins::model::resume::ResumeOutcome::Done) => {
                fire_stop_hook(orchestrator, &[], &ctx).await;
                return Ok(());
            }
            Err(e) => {
                plugin_warn!("model", "[Resume] process_resume failed: {}", e);
                fire_stop_hook(orchestrator, &[], &ctx).await;
                return Err(e);
            }
        }
    }

    loop {
        // 每轮开始时从 ChatSession 获取最新上下文（滑动窗口/工具上下文窗口/压缩全部生效）
        // 心跳任务等场景可设置 `load_history = false`：仅用本次 single_message，不加载任何历史。
        context.messages = if req.load_history.unwrap_or(true) {
            context
                .session
                .get_context_messages(None, req.tool_context_window)
                .await
                .unwrap_or_default()
        } else {
            // 不加载历史：保留上一轮在本内存中累积的消息（single + 响应 + 工具结果），
            // 但绝不从会话存储读取之前的对话历史。
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
                plugin_info!(
                    "model",
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
            plugin_info!(
                "model",
                "[DIAG] run_chat_loop: abort_flag true at top of turn {}",
                tool_rounds
            );
            fire_stop_hook(orchestrator, &context.messages, &ctx).await;
            return Ok(());
        }

        plugin_info!("model", "--- TURN {} START ---", tool_rounds);

        if check_abort(&abort_flag).await {
            plugin_info!(
                "model",
                "[DIAG] run_chat_loop: check_abort returned true at turn {}",
                tool_rounds
            );
            fire_stop_hook(orchestrator, &context.messages, &ctx).await;
            return Ok(());
        }

        // ── 被动语义压缩（L5：70% 触发）────────────────────────────────
        let system_prompt_for_request = req
            .system_prompt
            .as_deref()
            .unwrap_or("You are a helpful MODEL assistant.");
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
                        "model",
                        "Context compressed: {} messages -> 1 message",
                        history_count
                    );
                    last_saved = context.messages.len();
                }
                Ok(None) => {}
                Err(e) => {
                    plugin_info!(
                        "model",
                        "[DIAG] run_chat_loop: auto_compress_process Err({})",
                        e
                    );
                    fire_stop_hook(orchestrator, &context.messages, &ctx).await;
                    return Err(e);
                }
            }
        }

        // ── 水位提醒（nudge，目标四）─────────────────────────────────────
        // 估算用量 ≥ 55% 有效上限时，注入一条一次性系统提示，引导模型在
        // "阶段间隙"主动调用 context_compact（比 70% 硬触发更早、时机更优）。
        // 去重：已存在比最近快照更新的提醒时不重复注入（压缩会吞掉旧提醒，自然重置）。
        if auto_compress && !nudged_this_request {
            let has_standing_nudge = {
                let last_nudge = context.messages.iter().rposition(|m| {
                    m.meta
                        .as_ref()
                        .and_then(|meta| meta.get("kind"))
                        .and_then(|v| v.as_str())
                        == Some("context_nudge")
                });
                let last_snapshot = context.messages.iter().rposition(|m| {
                    m.meta
                        .as_ref()
                        .map(|meta| meta.get("compacted") == Some(&serde_json::json!(true)))
                        .unwrap_or(false)
                });
                match (last_nudge, last_snapshot) {
                    (Some(n), Some(s)) => n > s,
                    (Some(_), None) => true,
                    (None, _) => false,
                }
            };
            if !has_standing_nudge {
                let effective_limit = (orchestrator.config.max_context_tokens
                    - orchestrator.config.reserved_tokens)
                    as usize;
                let overhead =
                    compression::estimate_request_overhead(system_prompt_for_request, &ctx).await;
                if compression::should_emit_context_nudge(
                    &context.messages,
                    effective_limit,
                    overhead,
                ) {
                    nudged_this_request = true;
                    plugin_info!(
                        "model",
                        "[Compress] context nudge emitted (~55% of limit), suggesting context_compact"
                    );
                    context.messages.push(ChatMessage {
                        id: short_id(),
                        role: Some(MessageRole::User),
                        msg_type: Some(MessageType::Text),
                        content: Some(MessageContent::Text(
                            "[system note] Context usage is approaching the limit. If you are at a \
                             natural stage boundary, call the context_compact tool now to distill \
                             older history and continue seamlessly; otherwise keep working and it \
                             will be compacted automatically. Do not respond to this note directly."
                                .to_string(),
                        )),
                        status: Some(MessageStatus::Completed),
                        meta: Some(serde_json::json!({ "kind": "context_nudge" })),
                        ..Default::default()
                    });
                }
            }
        }

        let root_id: String = short_id();
        emit_streaming_start(&mut channel, &root_id, Some(tool_rounds)).await;

        apply_message_level_compression(orchestrator, &ctx, &mut context.messages).await;

        let mut tools = if let Some(tool_manager) = ctx.get(crate::symbio_core::CAPABILITY_MANAGER) {
            tool_manager.list_capability().await
        } else {
            Vec::new()
        };
        // 主动压缩工具（目标四）：仅当压缩功能启用时暴露给模型。
        // 执行不走 CapabilityManager 分发，由下方拦截逻辑处理（需要编排器内部链路）。
        if auto_compress {
            tools.push(compression::context_compact_tool_meta());
        }

        plugin_info!(
            "model",
            "[DIAG] run_chat_loop: about to call turn_processor.send_request, ctx_msg_count={}, tool_count={}",
            context.messages.len(),
            tools.len()
        );

        let result = turn_processor
            .send_request(
                req.system_prompt
                    .as_deref()
                    .unwrap_or("You are a helpful MODEL assistant."),
                &context.messages,
                &tools,
                &root_id,
                &mut channel,
                &abort_flag,
            )
            .await;

        plugin_info!(
            "model",
            "[DIAG] run_chat_loop: turn_processor.send_request returned, is_ok={}",
            result.is_ok()
        );

        let mut out = match result {
            Err(PluginError::RetryWithoutContextId) => {
                plugin_info!(
                    "model",
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
                plugin_warn!(
                    "model",
                    "[DIAG] run_chat_loop: send_request -> Aborted, returning Ok(())"
                );
                fire_stop_hook(orchestrator, &context.messages, &ctx).await;
                return Ok(());
            }
            Err(e) => {
                plugin_warn!(
                    "model",
                    "[DIAG] run_chat_loop: send_request -> Err({}), returning Err",
                    e
                );
                fire_stop_hook(orchestrator, &context.messages, &ctx).await;
                return Err(e);
            }
            Ok(out) => out,
        };

        if abort_flag.load(Ordering::SeqCst) {
            plugin_warn!(
                "model",
                "[DIAG] run_chat_loop: abort_flag became true after send_request, returning Ok(())"
            );
            fire_stop_hook(orchestrator, &context.messages, &ctx).await;
            return Ok(());
        }

        let tools_done = out.tool_accumulator.get_completed();
        // 提前取出本轮的结束原因 / 用量 / 是否出现过工具调用 / 文本子节点 id，
        // 因为 `out.into_messages` 会按值消费 out，之后无法再读这些字段。
        let had_tool = out.tool_accumulator.had_any_tool_call();
        let finish = out.finish.clone();
        let usage = out.usage;
        let rtid = out.response_text_child_id.clone();
        let rrid = out.reasoning_child_id.clone();

        turn_processor
            .finalize(&root_id, &out, &tools_done, &channel)
            .await;

        // 用 provider 返回的真实用量滚动校准 token 估算（中文/代码场景收益最大；
        // 估算长期偏低会直接导致 400 而非过早压缩）。
        if let Some(u) = usage {
            let tok = crate::symbio_core::default_tokenizer();
            // 反馈必须用原始启发式估算（count_raw）；用校准后的 count() 自反馈
            // 会让校准系数收敛到 √(真实比值)（见 CalibratedTokenizer::feedback 文档）。
            let estimated = tok.count_raw(&out.text) + tok.count_raw(&out.reasoning);
            crate::symbio_core::report_provider_usage(estimated, u.output);
        }

        context
            .messages
            .extend(out.into_messages(&root_id, tools_done.len()));

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
                    plugin_info!(
                        "model",
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
            plugin_info!(
                "model",
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
        let (compact_calls, other_calls): (Vec<_>, Vec<_>) = tools_done
            .into_iter()
            .partition(|tc| {
                tc.name
                    .as_deref()
                    .map(|n| n == compression::CONTEXT_COMPACT_TOOL_NAME)
                    .unwrap_or(false)
            });

        let mut tool_results: Vec<ChatMessage> = Vec::new();
        let mut parent_updates: Vec<ChatMessage> = Vec::new();

        if !compact_calls.is_empty() {
            // 本 Turn 首个 ToolCall 消息的下标：压缩切分的安全上界
            let split_tool_call_idx = context
                .messages
                .iter()
                .position(|m| m.msg_type == Some(MessageType::ToolCall))
                .unwrap_or(0);
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
                    system_prompt_for_request,
                    split_tool_call_idx,
                    hints.as_deref(),
                )
                .await;
                if ok {
                    // 压缩成功：上下文回落后允许再次水位提醒
                    nudged_this_request = false;
                    plugin_info!(
                        "model",
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
                parent_updates.push(ChatMessage {
                    id: call_id.clone(),
                    status: Some(MessageStatus::Completed),
                    meta: Some(meta),
                    ..Default::default()
                });
                tool_results.push(super::message_builder::build_tool_message(
                    &call_id,
                    &result_text,
                    Some(ok),
                    None,
                ));
            }
            // 同批多余的 compact 调用：直接标记跳过
            for extra in compact_calls.iter().skip(1) {
                if let Some(cid) = &extra.id {
                    parent_updates.push(ChatMessage {
                        id: cid.clone(),
                        status: Some(MessageStatus::Completed),
                        meta: Some(serde_json::json!({
                            "success": false,
                            "kind": "context_compact",
                            "skipped": true
                        })),
                        ..Default::default()
                    });
                    tool_results.push(super::message_builder::build_tool_message(
                        cid,
                        "Skipped: another context_compact call in this batch was executed.",
                        Some(false),
                        None,
                    ));
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
                plugin_warn!("model", "[Session] 父节点状态持久化失败: {}", e);
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
            // 给「因失败而暂停会话」的工具调用打 recoverable 标记（服务端唯一真相）。
            // 语义分界：auto 模式下工具失败 = 信息性（错误结果已喂给 LLM 继续处理，
            // 重试无意义）；仅当循环因该失败退出、会话停在等待恢复态时，重试/补参
            // 才有效（resume 在会话忙碌时会拒绝）。前端只对 recoverable 的 Failed
            // ToolCall 渲染重试/补参入口。
            // 注意：user_prompt(WaitingUserAction) 驱动的暂停不在此列（其恢复走
            // approve/reject/answer，且父节点状态是 WaitingUserAction 而非 Failed）。
            let mut recover_updates: Vec<ChatMessage> = Vec::new();
            for p in parent_updates.iter() {
                if p.status == Some(MessageStatus::Failed)
                    && p.meta
                        .as_ref()
                        .and_then(|m| m.get("failure_kind"))
                        .and_then(|v| v.as_str())
                        .is_some()
                {
                    let mut meta = p.meta.clone().unwrap_or_else(|| serde_json::json!({}));
                    meta["recoverable"] = serde_json::json!(true);
                    let patch = ChatMessage {
                        id: p.id.clone(),
                        meta: Some(meta),
                        ..Default::default()
                    };
                    // 广播 + 持久化，保证刷新后标记仍在
                    let _ = channel.tx.send(PluginFrame::Data(
                        serde_json::to_value(session_chat_response::StreamEvent::Update {
                            message: patch.clone(),
                        })
                        .unwrap_or_default(),
                    ))
                    .await;
                    recover_updates.push(patch);
                }
            }
            if !recover_updates.is_empty() {
                if let Err(e) = context.session.update_messages(recover_updates).await {
                    plugin_warn!("model", "[Session] recoverable 标记持久化失败: {}", e);
                }
            }
            plugin_info!(
                "model",
                "[DIAG] run_chat_loop: 工具待用户恢复（mode={}），退出本轮",
                mode
            );
            fire_stop_hook(orchestrator, &context.messages, &ctx).await;
            return Ok(());
        }

        // 轮次计数 + 老化淡化：轮次过多时，对较早的工具结果做 head/tail 摘要，
        // 始终保持最近 K 轮的原文与全部 assistant 文本/推理，最小化对思维链的破坏
        // （用户明确要求：不要硬性上限，而是"有效压缩或淡化历时轮次"）。
        tool_rounds += 1;
        if tool_rounds > FADE_ACTIVATE_ROUNDS {
            super::compression::fade_aged_tool_results(&mut context.messages, FADE_KEEP_RECENT_TURNS);
        }
    }
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
        plugin_warn!("model", "[Session] {}", msg);
        let _ = channel
            .tx
            .send(PluginFrame::Data(
                serde_json::to_value(session_chat_response::StreamEvent::Error { error: msg })
                    .unwrap_or_default(),
            ))
            .await;
    }
}

async fn open_chat_session(
    parent: &Option<Arc<dyn crate::symbio_core::Plugin>>,
    ctx: &Arc<dyn InvokeRequest>,
) -> Arc<dyn ChatSession> {
    let p = match parent {
        Some(p) => p,
        None => return Arc::new(FallbackChatSession::default()),
    };

    let open_ctx = ctx.fork();
    open_ctx.set(crate::symbio_core::PATH, SESSION_OPEN.to_string());
    let _ = open_ctx.set_payload(session_open::Request {
        session_id: ctx.get(crate::symbio_core::SESSION_ID),
    });

    let resp = match p.clone().route(open_ctx).await {
        Ok(r) => r,
        Err(_) => return Arc::new(FallbackChatSession::default()),
    };

    if let PluginPayload::Native(obj) = resp {
        if let Ok(handle) = obj.downcast::<ChatSessionHandle>() {
            return handle.0.clone();
        }
    }

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
        _tool_context_window: Option<usize>,
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

    let root_id = short_id();
    let summary = match send_compression_request(
        orchestrator,
        system_prompt,
        &context.messages,
        &root_id,
        channel,
        abort_flag,
    )
    .await
    {
        Ok(s) => s,
        // 用户中止 / 触发限流：向上透传，走既有的中止与限流提示链路。
        Err(e @ PluginError::Aborted) | Err(e @ PluginError::RateLimited(_)) => return Err(e),
        // 其他失败（空摘要 / 流中断 / 网关错误等）：**优雅降级**。
        //
        // 原始行为是 `?` 直接把错误抛给 run_chat_loop → 整个 turn 以
        // "Compression produced empty result" 之类的错误告终，用户连正常对话都发不出去。
        // 压缩只是上下文超限时的优化手段，失败不应阻断对话：
        // 回滚到未压缩历史，记录警告后继续（后续真正的 LLM 请求若同样失败，
        // 会以真实错误呈现在该轮 Turn 上）。
        Err(e) => {
            plugin_warn!(
                "model",
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
            system_prompt,
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

    // 快照消息：meta 记录压缩标记、压缩后估算（迟滞依据）、转存路径。
    let post_tokens = compression::estimate_message_tokens(&ChatMessage {
        content: Some(MessageContent::Text(snapshot_text.clone())),
        ..Default::default()
    });
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
        role: Some(MessageRole::Assistant),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text(format!(
            "[CONTEXT SNAPSHOT — 压缩的历史记忆，基于它继续任务]\n{snapshot_text}"
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
/// 关键正确性约束：**进行中的 Turn 必须整体保留**。
/// 调用时本 Turn 的 ToolCall 消息已在上下文中（请求后 extend），但其工具结果
/// 子节点尚未产生。切分点必须 ≤ 本 Turn 首个 ToolCall 的下标，否则：
/// - 快照把"请求工具"压进去、结果却还在保留区 → 孤儿消息被 drop_orphan_messages 剔除；
/// - 请求包里 Tool 结果失去父节点 → provider 400。
///
/// `split_tool_call_idx`：主循环传入的本 Turn 首个 ToolCall 消息下标（工具调用分派前计算）。
/// 返回 `(是否执行了压缩, 估算压缩前 tokens, 估算压缩后 tokens)`。
#[allow(clippy::too_many_arguments)]
async fn run_context_compact(
    orchestrator: &ChatOrchestrator,
    context: &mut SessionContext,
    channel: &mut PluginChannel,
    ctx: &Arc<dyn InvokeRequest>,
    abort_flag: &Arc<AtomicBool>,
    system_prompt: &str,
    split_tool_call_idx: usize,
    hints: Option<&str>,
) -> (bool, usize, usize) {
    if split_tool_call_idx == 0 {
        return (false, 0, 0);
    }

    // 待压缩历史 = [.., split_tool_call_idx)
    let history: Vec<ChatMessage> = context.messages[..split_tool_call_idx].to_vec();
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

    let summary = match send_compression_request(
        orchestrator,
        system_prompt,
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
            system_prompt,
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
    let post_tokens = compression::estimate_message_tokens(&ChatMessage {
        content: Some(MessageContent::Text(snapshot_text.clone())),
        ..Default::default()
    });
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
        role: Some(MessageRole::Assistant),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text(format!(
            "[CONTEXT SNAPSHOT — 压缩的历史记忆，基于它继续任务]\n{snapshot_text}"
        ))),
        status: Some(MessageStatus::Completed),
        meta: Some(meta),
        ..Default::default()
    };

    let mut new_messages = vec![snapshot_message];
    // 保留区：当前用户指令 + 进行中 Turn（含本批 ToolCall）及之后的一切
    new_messages.extend_from_slice(&original_messages[split_tool_call_idx..]);
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
    use super::protocol::parse_sse_stream;
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
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn InvokeRequest>,
    messages: &mut [ChatMessage],
) {
    if let Some(session_id) = ctx.get(crate::symbio_core::SESSION_ID) {
        compress_temporary_messages(&orchestrator.parent, &session_id, messages, ctx.clone()).await;
    }
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
