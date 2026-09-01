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
    session::chat_message::{ChatMessage, MessageStatus, MessageType},
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
        "[DIAG] run_chat_loop: max_tool_rounds={}, auto_compress={}, msg_id_in_payload={:?}",
        req.max_tool_rounds.unwrap_or(15),
        req.auto_compress.unwrap_or(true),
        req.single_message.as_ref().map(|m| m.id.clone())
    );

    let max_tool_rounds = req.max_tool_rounds.unwrap_or(15);
    let auto_compress = req.auto_compress.unwrap_or(true);

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
            },
            Ok(crate::plugins::model::resume::ResumeOutcome::Done) => {
                fire_stop_hook(orchestrator, &[], &ctx).await;
                return Ok(());
            },
            Err(e) => {
                plugin_warn!("model", "[Resume] process_resume failed: {}", e);
                fire_stop_hook(orchestrator, &[], &ctx).await;
                return Err(e);
            },
        }
    }

    for turn in 0..max_tool_rounds {
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
        if turn == 0 {
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

        if abort_flag.load(Ordering::SeqCst) {
            // SYS-002: 早期 return 路径上的副作用（last_saved 尚未用作流式增量锚点，
            // 此分支里不更新，但保留 last_saved 维持语义对称）。
            plugin_info!(
                "model",
                "[DIAG] run_chat_loop: abort_flag true at top of turn {}",
                turn
            );
            fire_stop_hook(orchestrator, &context.messages, &ctx).await;
            return Ok(());
        }

        plugin_info!("model", "--- TURN {} START ---", turn);

        if check_abort(&abort_flag).await {
            plugin_info!(
                "model",
                "[DIAG] run_chat_loop: check_abort returned true at turn {}",
                turn
            );
            fire_stop_hook(orchestrator, &context.messages, &ctx).await;
            return Ok(());
        }

        if auto_compress {
            match auto_compress_process(
                orchestrator,
                &mut context,
                &mut channel,
                &ctx,
                &abort_flag,
                req.system_prompt
                    .as_deref()
                    .unwrap_or("You are a helpful MODEL assistant."),
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
                },
                Ok(None) => {},
                Err(e) => {
                    plugin_info!(
                        "model",
                        "[DIAG] run_chat_loop: auto_compress_process Err({})",
                        e
                    );
                    fire_stop_hook(orchestrator, &context.messages, &ctx).await;
                    return Err(e);
                },
            }
        }

        let root_id: String = short_id();
        emit_streaming_start(&mut channel, &root_id, Some(turn)).await;

        apply_message_level_compression(orchestrator, &ctx, &mut context.messages).await;

        let tools = if let Some(tool_manager) = ctx.get(crate::symbio_core::CAPABILITY_MANAGER) {
            tool_manager.list_capability().await
        } else {
            Vec::new()
        };

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

        let out = match result {
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
            },
            Err(PluginError::Aborted) => {
                plugin_warn!(
                    "model",
                    "[DIAG] run_chat_loop: send_request -> Aborted, returning Ok(())"
                );
                fire_stop_hook(orchestrator, &context.messages, &ctx).await;
                return Ok(());
            },
            Err(e) => {
                plugin_warn!(
                    "model",
                    "[DIAG] run_chat_loop: send_request -> Err({}), returning Err",
                    e
                );
                fire_stop_hook(orchestrator, &context.messages, &ctx).await;
                return Err(e);
            },
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
        turn_processor
            .finalize(&root_id, &out, &tools_done, &channel)
            .await;

        context
            .messages
            .extend(out.into_messages(&root_id, tools_done.len()));
        if tools_done.is_empty() {
            plugin_info!(
                "model",
                "[DIAG] run_chat_loop: no tool calls, finalizing turn {}, text_added={}, returning Ok(())",
                turn,
                context.messages.len()
            );
            persist_messages(&context, last_saved, &channel).await;
            fire_stop_hook(orchestrator, &context.messages, &ctx).await;
            return Ok(());
        }

        let (tool_results, parent_updates) = process_tool_calls_async(
            tools_done,
            &orchestrator.parent,
            &mut channel,
            &abort_flag,
            ctx.clone(),
        )
        .await;
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
        // - user_prompt(WaitingUserAction)：confirm/ask_user（始终退出）
        // - interactive 模式下 Failed-with-failure_kind：工具执行失败（用户可重试/补充）
        // auto 模式下工具失败不退出，错误结果传 LLM 继续处理。
        let mode = ctx.get(crate::symbio_core::MODE).unwrap_or_default();
        let needs_user_action = tool_results.iter().any(|m| {
            m.msg_type == Some(MessageType::UserPrompt)
                && m.status == Some(MessageStatus::WaitingUserAction)
        }) || parent_updates.iter().any(|p| {
            p.status == Some(MessageStatus::WaitingUserAction)
                || (mode == "interactive"
                    && p.status == Some(MessageStatus::Failed)
                    && p.meta
                        .as_ref()
                        .and_then(|m| m.get("failure_kind"))
                        .and_then(|v| v.as_str())
                        .is_some())
        });

        if needs_user_action {
            plugin_info!(
                "model",
                "[DIAG] run_chat_loop: 工具待用户恢复（mode={}），退出本轮",
                mode
            );
            fire_stop_hook(orchestrator, &context.messages, &ctx).await;
            return Ok(());
        }
    }

    plugin_info!(
        "model",
        "[DIAG] run_chat_loop: loop exhausted, returning Ok(())"
    );
    fire_stop_hook(orchestrator, &context.messages, &ctx).await;
    Ok(())
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
        store.extend(messages);
        Ok(store.len())
    }

    async fn replace_messages(&self, messages: Vec<ChatMessage>) -> Result<(), PluginError> {
        let mut store = self.messages.lock().await;
        *store = messages;
        Ok(())
    }

    async fn update_messages(&self, messages: Vec<ChatMessage>) -> Result<(), PluginError> {
        let mut store = self.messages.lock().await;
        for patch in messages {
            if let Some(existing) = store.iter_mut().find(|m| m.id == patch.id) {
                *existing = patch;
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
) -> Result<Option<usize>, PluginError> {
    let effective_context_limit =
        (orchestrator.config.max_context_tokens - orchestrator.config.reserved_tokens) as usize;

    if !compression::should_start_compression(&context.messages, effective_context_limit, false) {
        return Ok(None);
    }

    let (compression_msg, _history_to_compress, history_to_keep) =
        match compression::prepare_compression(&context.messages) {
            Some(v) => v,
            None => return Ok(None),
        };

    let original_count = context.messages.len();
    let _ = fire_hook(&orchestrator.parent, HookEvent::PreCompact, ctx.clone()).await;

    context.messages = vec![compression_msg];

    let root_id = short_id();
    let summary = send_compression_request(
        orchestrator,
        system_prompt,
        &context.messages,
        &root_id,
        channel,
        abort_flag,
    )
    .await?;

    context.messages.clear();

    let mut new_messages = vec![summary];
    new_messages.extend(history_to_keep);
    context.messages = new_messages;

    let _ = context
        .session
        .replace_messages(context.messages.clone())
        .await;

    Ok(Some(original_count))
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
            "Compression produced empty result".to_string(),
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
