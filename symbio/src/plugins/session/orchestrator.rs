use super::active::{ActiveSessionState, REQUEST_ID_COUNTER};
use super::plugin::SessionPlugin;
use crate::symbio_core::event_bus::EventBus;
use crate::symbio_core::schemas::{
    model::model_chat,
    session::chat_message as cm,
    session::{session_append, session_chat, session_chat_response},
};
use crate::symbio_core::{
    attach_capabilities, collect_capabilities, InvokeRequest, InvokeRequestExt, InvokeResponse,
    Plugin, PluginError, PluginFrame, PluginPayload, take_errors, MODE, MODEL_CHAT, PROVIDER_ID,
    RISK_LEVEL, SESSION_ID, WORKDIR,
};
use serde_json::json;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
/// 工作态守卫：保障 `is_working` 在任何退出路径（包括 panic 崩溃）下都会收敛。
///
/// ## 背景
///
/// 旧实现里 `handle_chat_message` 的 `tokio::spawn` 任务若发生 **panic**，
/// 该任务会被 tokio 静默终止，但 `is_working` 永远停留在 `true`，
/// 导致前端永久显示"AI 处理中"且无法恢复（CHAT_FLOW_ANALYSIS 核心问题）。
///
/// ## 机制
///
/// - 在 spawn 任务开始时构造本守卫，`done` 初始为 `false`。
/// - **正常结束路径**在收尾前把 `done` 置 `true`，使 Drop 成为 no-op
///   （正常路径已自行 `is_working=false` + 广播 idle）。
/// - **panic 路径**：Rust unwind 会执行局部变量析构，`Drop` 被调用，
///   此时 `done==false`，守卫会 spawn 一个 detached 任务：
///   1. 若该 request_id 仍是当前活跃请求，把 `is_working` 复位；
///   2. 把仍处 streaming/pending 的 AI 消息标 `Failed` + 错误并持久化；
///   3. 广播 idle，使前端 UI 状态收敛。
struct WorkingGuard {
    state: Arc<ActiveSessionState>,
    plugin: Arc<SessionPlugin>,
    collected: Arc<tokio::sync::Mutex<Vec<cm::ChatMessage>>>,
    /// 真实会话 id：`persist_failure` 需要它来定位并写入存储。
    /// 注意：绝不能传 `request_id` 的字符串——那是请求序号（如 "42"），
    /// 会令 `open_chat_session` 找不到会话而提前返回，导致崩溃失败永不落库。
    session_id: String,
    done: bool,
}

impl Drop for WorkingGuard {
    fn drop(&mut self) {
        if self.done {
            return;
        }
        // panic / 异常退出路径：尽力重置状态并持久化失败，避免前端永久卡死。
        let state = self.state.clone();
        let plugin = self.plugin.clone();
        let collected = self.collected.clone();
        let session_id = self.session_id.clone();
        let crash_msg = "会话处理异常中断（后台任务崩溃），请重试".to_string();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                // 仅当本会话仍在进行中时才复位。
                // 注：旧的「同 request_id 才复位」检查依赖 ActiveSessionStateInner.request_id，
                // 字段已重构到 WorkingGuard.request_id（per-request）；
                // 此处直接靠 `is_working` 兜底：若本会话已被新一轮请求接管，
                // 持久化失败会自然被 persist_failure 内部的版本检查拦截。
                {
                    let mut inner = state.inner.write().await;
                    if inner.is_working {
                        inner.is_working = false;
                    }
                }
                // 把"仍在进行中"的 AI 消息持久化为 Failed + 错误原因
                // （切回会话时能看到上次失败的终态，目标 3）。
                plugin
                    .persist_failure(&state, &session_id, &collected, &crash_msg)
                    .await;
                // 同时向前端广播一条业务级 Error 事件，使 UI 立即显示错误
                // （否则前端只会收到 idle，那条 streaming 消息会一直显示"回复中…"）。
                plugin
                    .broadcast_frame(
                        &state,
                        PluginFrame::Data(json!(session_chat_response::StreamEvent::Error {
                            error: crash_msg.clone()
                        })),
                    )
                    .await;
                plugin.broadcast_status(&state, "idle").await;
            });
        }
    }
}

/// Merge a `StreamEvent::Update` patch into an already-collected message.
///
/// Mirrors the front-end `handleChatEvent` semantics:
/// - `content` is **delta-appended** for `Text` / `Reasoning` (matches the SSE
///   incremental stream); for `ToolCall` (and ContentPart arrays) the new content
///   is a full replacement.
/// - `status`, `role`, `msg_type`, `name`, `timestamp`, `parent_id` are replaced
///   whenever present in the patch.
/// - `meta` is shallow-merged (existing + new keys).
///
/// This is what guarantees that a session whose stream was observed by the
/// backend will be persisted with its final status (e.g. `Completed` /
/// `WaitingUserAction`), instead of being frozen at the first `Streaming` frame.
fn merge_message_patch(existing: &mut cm::ChatMessage, patch: &cm::ChatMessage) {
    if let Some(role) = &patch.role {
        existing.role = Some(role.clone());
    }
    if let Some(t) = &patch.msg_type {
        existing.msg_type = Some(t.clone());
    }
    if let Some(n) = &patch.name {
        existing.name = Some(n.clone());
    }
    if let Some(p) = &patch.parent_id {
        existing.parent_id = Some(p.clone());
    }
    if let Some(s) = &patch.status {
        existing.status = Some(s.clone());
    }
    if let Some(ts) = patch.timestamp {
        existing.timestamp = Some(ts);
    }
    if let Some(rid) = &patch.response_id {
        existing.response_id = Some(rid.clone());
    }

    if let Some(new_content) = &patch.content {
        match existing.msg_type {
            Some(cm::MessageType::ToolCall) => {
                // Tool-call args are emitted as full JSON in each frame
                existing.content = Some(new_content.clone());
            }
            Some(cm::MessageType::Text) | Some(cm::MessageType::Reasoning) => {
                // SSE delta: append
                match (&mut existing.content, new_content) {
                    (Some(existing_c), cm::MessageContent::Text(new_text)) => {
                        if let cm::MessageContent::Text(buf) = existing_c {
                            buf.push_str(new_text);
                        } else {
                            // Type mismatch (rare) — fall back to replacement
                            *existing_c = cm::MessageContent::Text(new_text.clone());
                        }
                    }
                    (None, cm::MessageContent::Text(new_text)) => {
                        existing.content = Some(cm::MessageContent::Text(new_text.clone()));
                    }
                    _ => {
                        // Parts arrays or type mismatch — replace
                        existing.content = Some(new_content.clone());
                    }
                }
            }
            _ => {
                // Turn / unknown: full replace
                existing.content = Some(new_content.clone());
            }
        }
    }

    if let Some(new_meta) = &patch.meta {
        match &mut existing.meta {
            Some(existing_meta_obj) => {
                if let (Some(existing_obj), Some(new_obj)) =
                    (existing_meta_obj.as_object_mut(), new_meta.as_object())
                {
                    for (k, v) in new_obj {
                        existing_obj.insert(k.clone(), v.clone());
                    }
                } else {
                    existing.meta = Some(new_meta.clone());
                }
            }
            None => {
                existing.meta = Some(new_meta.clone());
            }
        }
    }
}

impl SessionPlugin {
    /// 集中发送"业务错误 + 状态收敛"：复位 `is_working` → broadcast Error 事件 → broadcast Status idle
    ///
    /// **目的**：所有可恢复错误路径都必须保证 `is_working` 收敛到 `false`，
    /// 否则前端会一直显示"AI 处理中"且 30 分钟内无任何信号能清；
    /// 同时后端 `is_working` 不复位会导致后续 resume 请求被 `session_busy` 守卫静默拒绝。
    async fn broadcast_error_with_idle(
        &self,
        state: &Arc<ActiveSessionState>,
        error: impl Into<String>,
    ) {
        let err = error.into();
        // 先复位 is_working，确保后续 resume 请求不会被 session_busy 守卫拒绝
        {
            let mut inner = state.inner.write().await;
            if inner.is_working {
                inner.is_working = false;
            }
        }
        self.broadcast_frame(
            state,
            PluginFrame::Data(json!(session_chat_response::StreamEvent::Error {
                error: err,
            })),
        )
        .await;
        self.broadcast_status(state, "idle").await;
    }

    /// 统一的 chat_loop 任务执行器（从 `handle_chat_message` 提取，供 continuation 复用）。
    ///
    /// 职责：
    /// - 构造 `WorkingGuard`（保障 panic 时 `is_working` 收敛 + 失败持久化）
    /// - 调用 `parent.route(chat_ctx)` 启动 model 插件的 chat_loop
    /// - 接收 sub_channel 帧：Error → 持久化失败 + 广播；Data → 合并收集 + 透传广播
    /// - 正常结束：清理 `is_working` + 广播 idle
    ///
    /// `chat_ctx` 应已设置好 PATH/SESSION_ID/AGENT_ID/WORKDIR/payload 等所有字段。
    async fn run_chat_loop_task(
        self: Arc<Self>,
        state: Arc<ActiveSessionState>,
        chat_ctx: Arc<dyn InvokeRequest>,
        session_id: String,
        parent: Arc<dyn Plugin>,
        rid: u64,
    ) {
        let collected_ai_messages =
            Arc::new(tokio::sync::Mutex::new(Vec::<cm::ChatMessage>::new()));

        // 工作态守卫：保障 panic / 异常退出时 is_working 收敛 + 失败持久化。
        // 正常结束路径会在收尾前把 `done` 置 true（见块末尾）。
        let mut guard = WorkingGuard {
            state: state.clone(),
            plugin: self.clone(),
            collected: collected_ai_messages.clone(),
            session_id: session_id.clone(),
            done: false,
        };

        match parent.route(chat_ctx).await {
            Ok(payload) => {
                if let PluginPayload::Session(mut sub_channel) = payload {
                    {
                        let mut inner = state.inner.write().await;
                        inner.ai_control_tx = Some(sub_channel.tx.clone());
                    }

                    while let Ok(frame_opt) =
                        tokio::time::timeout(Duration::from_secs(1800), sub_channel.rx.recv()).await
                    {
                        let frame = match frame_opt {
                            Some(f) => f,
                            None => break,
                        };
                        if !state.inner.read().await.is_working
                            || state.request_id.load(Ordering::SeqCst) != rid
                        {
                            break;
                        }
                        match &frame {
                            PluginFrame::Error(msg, _) => {
                                // 透传 plugin-level Error 帧作为业务级 Error 事件。
                                // 同时把"仍在进行中"的 AI 消息持久化为 Failed + 错误原因，
                                // 这样切回会话时能看到上次失败的终态。
                                self.persist_failure(
                                    &state,
                                    &session_id,
                                    &collected_ai_messages,
                                    msg,
                                )
                                .await;
                                // 复位 is_working + 广播 Error + 广播 idle：
                                // 必须复位 is_working，否则后续 resume 请求会被
                                // `handle_chat_send_oneoff` 的 session_busy 守卫静默拒绝，
                                // 导致用户点重试无任何反应（LLM 失败重试不生效 bug 的根因）。
                                self.broadcast_error_with_idle(&state, msg.clone()).await;
                                guard.done = true;
                                return;
                            }
                            PluginFrame::Data(data) => {
                                // 收集 Model 响应消息：StreamEvent::Update 增量合并，
                                // 确保持久化的终态反映最后已知状态（而非首个 Streaming 帧）。
                                if let Ok(session_chat_response::StreamEvent::Update { message }) =
                                    serde_json::from_value::<session_chat_response::StreamEvent>(
                                        data.clone(),
                                    )
                                {
                                    let mut collected = collected_ai_messages.lock().await;
                                    if let Some(existing) =
                                        collected.iter_mut().find(|m| m.id == message.id)
                                    {
                                        merge_message_patch(existing, &message);
                                    } else {
                                        collected.push(message.clone());
                                    }
                                }
                                self.broadcast_frame(&state, frame).await;
                            }
                        }
                    }
                    {
                        let mut inner = state.inner.write().await;
                        inner.ai_control_tx = None;
                    }
                } else {
                    let msg = "Model 插件未返回预期的会话载荷".to_string();
                    crate::plugin_error!("session", "{}", &msg);
                    self.persist_failure(&state, &session_id, &collected_ai_messages, &msg)
                        .await;
                    self.broadcast_error_with_idle(&state, msg).await;
                    return;
                }
            }
            Err(e) => {
                let msg = format!("调用 Model 插件失败: {}", e);
                crate::plugin_error!("session", "{}", &msg);
                self.persist_failure(&state, &session_id, &collected_ai_messages, &msg)
                    .await;
                self.broadcast_error_with_idle(&state, msg).await;
                return;
            }
        }

        // NOTE: Model 响应消息**不在这里再次持久化**。
        // `chat_loop::persist_messages` 已在每轮结束时把完整 `into_messages` 写到存储。
        // 这里 `collected_ai_messages` 仅用于向前端广播实时流式事件。
        {
            let ai_msgs = collected_ai_messages.lock().await;
            if !ai_msgs.is_empty() {
                crate::plugin_info!(
                    "session",
                    "Model 流式收集 {} 条消息（由 Model 插件持久化，session 插件不再重复写入）",
                    ai_msgs.len()
                );
            }
        }
        // 正常结束路径：清理 working 状态 + 广播 idle。
        {
            let mut inner = state.inner.write().await;
            if inner.is_working {
                inner.is_working = false;
            }
        }
        guard.done = true;
        self.broadcast_status(&state, "idle").await;
    }

    pub async fn handle_abort(&self, state: &Arc<ActiveSessionState>) {
        let abort_sent = {
            let inner = state.inner.read().await;
            if let Some(tx) = inner.ai_control_tx.as_ref() {
                let _ = tx.send(PluginFrame::Data(json!({ "type": "abort" }))).await;
                true
            } else {
                false
            }
        };

        if abort_sent {
            // 轮询等待 ai_control_tx 主动置空（最迟 3s 兜底，避免无限等待）
            //
            // 历史：原代码用 20×100ms=2s 硬编码 sleep 等候 AI 子任务退出；
            // 现在轮询 ai_control_tx 是否被 Model 插件主动清空（见 run_chat_loop
            // Err(PluginError::Aborted) 分支），更准确反映子任务结束时机。
            // 3s 是兜底上限：若子任务未实现 Aborted 信号，仍能 3s 内强制收敛。
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            loop {
                if state.inner.read().await.ai_control_tx.is_none() {
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    crate::plugin_warn!("session", "abort: ai_control_tx 未在 3s 内清空，强制收敛");
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }

        {
            let mut inner = state.inner.write().await;
            inner.is_working = false;
            inner.ai_control_tx = None;
        }

        self.broadcast_frame(
            state,
            PluginFrame::Data(json!(session_chat_response::StreamEvent::Abort)),
        )
        .await;
        self.broadcast_status(state, "idle").await;
    }

    // ============ One-off 模式（统一事件总线场景）============

    /// 统一解析会话参数（mode / risk_level / provider_id）。
    ///
    /// 解析链（三者完全对称）：`req` 字段 > `session.metadata` > 默认值。
    /// - mode: 默认 `interactive`
    /// - risk_level: 默认 `medium`
    /// - provider_id: 默认 `None`（Model 插件使用默认 Provider）
    ///
    /// 解析结果同时 `set` 到 `ctx`，供下游 `chat_loop` / `continuation` 通过 `ctx.fork()` 继承。
    /// 返回 `(mode, risk_level, provider_id)` 供调用方构造 `model_chat::Request` 时使用。
    async fn resolve_session_params(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
        session_id: &str,
        req: &session_chat::Request,
    ) -> (String, String, Option<String>) {
        // 只调用一次 get_or_create_session，复用 session 对象取三个回退值
        let session = self.get_or_create_session(session_id).await.ok();
        let meta = session.as_ref().and_then(|s| s.metadata.as_object());

        let mode = req
            .mode
            .as_ref()
            .filter(|s| !s.is_empty())
            .cloned()
            .or_else(|| {
                meta.and_then(|m| m.get("mode"))
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .unwrap_or_else(|| "interactive".to_string());
        ctx.set(MODE, mode.clone());

        let risk_level = req
            .risk_level
            .as_ref()
            .filter(|s| !s.is_empty())
            .cloned()
            .or_else(|| {
                meta.and_then(|m| m.get("risk_level"))
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .unwrap_or_else(|| "medium".to_string());
        ctx.set(RISK_LEVEL, risk_level.clone());

        let provider_id = req
            .provider_id
            .as_ref()
            .filter(|p| !p.is_empty())
            .cloned()
            .or_else(|| {
                meta.and_then(|m| m.get("provider_id"))
                    .and_then(|v| v.as_str())
                    .map(String::from)
            })
            .filter(|p| !p.is_empty());
        if let Some(pid) = &provider_id {
            ctx.set(PROVIDER_ID, pid.clone());
        }

        (mode, risk_level, provider_id)
    }

    /// one-off 统一聊天入口（send + resume 共用）。
    ///
    /// 两种情况互斥，统一走同一路径，区别仅在构造 `model_chat::Request` 时：
    /// - `req.message` 存在 → 追加用户消息，`single_message=Some(msg)`，从 root 会话流开始
    /// - `req.resume` 存在 → 不追加消息，`resume=Some(req)`，由 `run_chat_loop`
    ///   在 turn 循环前处理（删除旧消息 → 重新执行 → 创建新子节点）
    ///
    /// ## 会话编排权归 session（重构要点）
    ///
    /// 本方法是**会话的唯一编排入口**，不再把请求转交给 agent 插件：
    /// 1. `agent_id` **可选**——未选择智能体的会话以"纯工具模式"照常运行
    /// 2. 自行经 `collect_capabilities` 广播 `traverse` 收集全部插件的工具
    ///    （local / web / mcp / skill / agent… 全部同一机制，agent 仅在
    ///    `ctx[AGENT_ID]` 存在时贡献智能体工具与人格）
    /// 3. 组装与智能体无关的基础提示词（`AGENTS.md` 全局 / 工作区指令）
    /// 4. 直接路由 `model/chat`
    ///
    /// 响应立刻返回，流式事件由 bus 推送。
    pub async fn handle_chat_send_oneoff(
        self: Arc<Self>,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        let req: session_chat::Request = ctx.payload()?;

        // 1. 统一 session_id 解析与校验
        let session_id = ctx
            .get(SESSION_ID)
            .or_else(|| req.session_id.clone())
            .unwrap_or_else(|| "default".to_string());
        if session_id.is_empty() || session_id == "default" {
            return Err(PluginError::ValidationError("session_id 不能为空".into()));
        }

        // 2. 统一参数解析（mode/risk_level/provider_id）—— send 和 resume 共用
        let (_mode, _risk_level, provider_id) =
            self.resolve_session_params(&ctx, &session_id, &req).await;

        let state = self.active_mgr.get_or_create(&session_id).await;
        let parent = self
            .get_parent()
            .ok_or_else(|| PluginError::InternalError("父插件未设置".into()))?;

        // 3. 提取互斥字段
        let resume = req.resume.clone();
        let user_msg = req.message.clone();

        // 4. 分支校验 + is_working 守卫
        if resume.is_none() && user_msg.is_none() {
            return Err(PluginError::ValidationError(
                "必须提供 message 或 resume".into(),
            ));
        }
        if resume.is_some() {
            // Resume 分支：is_working 守卫（忙碌时拒绝，避免并发 resume 竞争）
            let inner = state.inner.read().await;
            if inner.is_working {
                return Ok(PluginPayload::new(&json!({
                    "status": "session_busy",
                    "session_id": session_id
                })));
            }
        }
        // message 分支：无 is_working 守卫（允许新消息覆盖旧请求）

        // 5. workdir 解析（ctx.workdir > session.metadata.workdir）—— send 和 resume 共用
        let workdir = match ctx.get(WORKDIR) {
            Some(w) if !w.is_empty() => w,
            _ => match self.get_or_create_session(&session_id).await {
                Ok(s) => match s
                    .metadata
                    .get("workdir")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                {
                    Some(w) if !w.is_empty() => w,
                    _ => {
                        let msg = "会话未绑定工作目录，且上下文未提供".to_string();
                        crate::plugin_error!("session", &msg);
                        self.broadcast_error_with_idle(&state, msg).await;
                        return Ok(PluginPayload::new(&json!({
                            "status": "accepted",
                            "session_id": session_id
                        })));
                    }
                },
                Err(e) => {
                    let msg = format!("读取会话元数据失败: {}", e);
                    crate::plugin_error!("session", &msg);
                    self.broadcast_error_with_idle(&state, msg).await;
                    return Ok(PluginPayload::new(&json!({
                        "status": "accepted",
                        "session_id": session_id
                    })));
                }
            },
        };

        // 6. set is_working + busy + rid
        let rid = REQUEST_ID_COUNTER.fetch_add(1, Ordering::SeqCst);
        state.request_id.store(rid, Ordering::SeqCst);
        {
            let mut inner = state.inner.write().await;
            inner.is_working = true;
            inner.last_content.clear();
            inner.last_tool_calls.clear();
        }

        // 7. spawn 统一任务（send 和 resume 共用 run_chat_loop_task）
        let parent_spawn = Arc::clone(&parent);
        let state_spawn = Arc::clone(&state);
        let sid_spawn = session_id.clone();
        let this_spawn = self.clone();
        let w_clone = workdir.clone();
        let ctx_spawn = ctx.fork();
        let pid_clone = provider_id.clone();
        let agent_id_from_req = req.agent_id.clone();
        let include_history = req.include_history;
        let resume_spawn = resume;
        let user_msg_spawn = user_msg;

        tokio::spawn(async move {
            this_spawn.broadcast_status(&state_spawn, "busy").await;

            let is_ping = user_msg_spawn
                .as_ref()
                .map(|m| {
                    m.content
                        .as_ref()
                        .map(|c| c.to_text() == "ping")
                        .unwrap_or(false)
                })
                .unwrap_or(false);

            // 记录最近一次"有效活动"时间（心跳任务据此判断会话是否已空闲足够久）。
            // ping（健康检查）不计入活动，避免误触发心跳计时器重置。
            // resume 也计入活动（用户主动操作），避免心跳在用户审批期间误触发。
            if !is_ping {
                this_spawn.mark_activity(&sid_spawn).await;
            }

            // 仅 message 分支（非 ping）追加用户消息到存储
            if let Some(msg) = &user_msg_spawn {
                if !is_ping {
                    let append_req = session_append::Request {
                        session_id: sid_spawn.clone(),
                        messages: vec![msg.clone()],
                    };
                    let _ = ctx_spawn.set_payload(append_req);
                    if let Err(e) = this_spawn.invoke_append(ctx_spawn.clone()).await {
                        let msg = format!("追加用户消息到存储失败: {}", e);
                        crate::plugin_error!("session", "{}", &msg);
                        this_spawn
                            .broadcast_error_with_idle(&state_spawn, msg)
                            .await;
                        return;
                    }
                }
            }

            // ── agent_id 解析（可选）──
            // 优先级：请求显式指定 > 会话元数据绑定；两者皆缺 → 未选择智能体，
            // 会话以"纯工具模式"运行（agent 插件不贡献任何工具，其余插件不受影响）。
            let agent_id = if let Some(id) = agent_id_from_req.filter(|s| !s.trim().is_empty()) {
                Some(id)
            } else {
                match this_spawn.get_or_create_session(&sid_spawn).await {
                    Ok(s) => s
                        .metadata
                        .get("agent_id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty()),
                    Err(e) => {
                        // 元数据读取失败只影响"回退不可用"：按未选择处理并记日志，
                        // 不阻断会话（显式携带 agent_id 的请求不受影响）。
                        crate::plugin_warn!(
                            "session",
                            "读取会话元数据失败（agent_id 回退不可用）: {}",
                            e
                        );
                        None
                    }
                }
            };
            let session_cfg = this_spawn.config.read().await.clone();

            // ── 会话编排核心：收集全部插件的工具（统一 traverse 机制）──
            // local / web / mcp / skill / agent 全部在同一机制下贡献工具；
            // agent 仅在 AGENT_ID 存在时贡献智能体工具与人格，无智能体会话照常运行。
            let chat_ctx = ctx_spawn.fork();
            chat_ctx.set(WORKDIR, w_clone.clone());
            chat_ctx.set(SESSION_ID, sid_spawn.clone());
            if let Some(aid) = &agent_id {
                chat_ctx.set(crate::symbio_core::AGENT_ID, aid.clone());
            }

            let tool_manager = collect_capabilities(Some(&parent_spawn), &chat_ctx).await;

            // 收集期硬错误（如会话绑定了不存在的智能体）→ 中止并明确报错，
            // 绝不静默降级成"没有人格的通用助手"。
            if let Some(first) = take_errors(&chat_ctx).await.into_iter().next() {
                let msg = format!("[{}] {}", first.plugin, first.message);
                crate::plugin_error!("session", "能力收集失败: {}", &msg);
                this_spawn
                    .broadcast_error_with_idle(&state_spawn, msg)
                    .await;
                return;
            }

            attach_capabilities(&chat_ctx, tool_manager);

            // ── 基础提示词（与智能体无关）：AGENTS.md 全局 / 工作区指令 ──
            // 智能体人格不在这里——由 agent_identity 工具说明承载。
            let base_prompt = super::prompt::build_system_prompt(Some(w_clone.as_str())).await;
            let system_prompt_opt = if base_prompt.trim().is_empty() {
                None
            } else {
                Some(base_prompt)
            };

            // 构造 model_chat::Request：resume 分支 vs message 分支（含 ping 特殊处理）
            let chat_input = if let Some(tr) = resume_spawn {
                json!(model_chat::Request {
                    system_prompt: system_prompt_opt,
                    single_message: None,
                    thinking: None,
                    stream: Some(true),
                    max_tool_rounds: Some(session_cfg.max_tool_rounds),
                    tool_context_window: Some(session_cfg.tool_context_window),
                    auto_compress: Some(session_cfg.auto_compress),
                    provider_id: pid_clone.clone(),
                    load_history: Some(true), // resume 必须加载历史以定位目标消息
                    resume: Some(tr),
                })
            } else if is_ping {
                json!(model_chat::Request {
                    system_prompt: Some("You are a helpful assistant.".to_string()),
                    single_message: user_msg_spawn,
                    thinking: None,
                    stream: Some(true),
                    max_tool_rounds: Some(session_cfg.max_tool_rounds),
                    tool_context_window: Some(session_cfg.tool_context_window),
                    auto_compress: Some(session_cfg.auto_compress),
                    provider_id: pid_clone.clone(),
                    load_history: include_history,
                    resume: None,
                })
            } else {
                // 普通发送：时间 / 工作区上下文挂在用户消息的 LLM prompt 上
                // （`prompt` 不持久化，每轮发送重新生成，模型始终知道"现在几点、在哪个工作区"）。
                let single = user_msg_spawn.map(|mut m| {
                    if m.role == Some(cm::MessageRole::User) {
                        m.prompt = Some(super::prompt::temporal_context(Some(w_clone.as_str())));
                    }
                    m
                });
                json!(model_chat::Request {
                    system_prompt: system_prompt_opt,
                    single_message: single,
                    thinking: None,
                    stream: Some(true),
                    max_tool_rounds: Some(session_cfg.max_tool_rounds),
                    tool_context_window: Some(session_cfg.tool_context_window),
                    auto_compress: Some(session_cfg.auto_compress),
                    provider_id: pid_clone.clone(),
                    load_history: include_history,
                    resume: None,
                })
            };

            // 直接路由 model/chat——会话编排归 session，不再经过 agent/chat
            chat_ctx.set(crate::symbio_core::PATH, MODEL_CHAT.to_string());
            chat_ctx.set_payload(chat_input).ok();

            // 调用统一的 chat_loop 任务执行器
            // run_chat_loop 内部会区分 resume（turn 前处理）与 single_message（正常 turn）
            this_spawn
                .clone()
                .run_chat_loop_task(state_spawn, chat_ctx, sid_spawn, parent_spawn, rid)
                .await;
        });

        // 立即返回 success（事件流经 bus 推送）
        Ok(PluginPayload::new(&json!({
            "status": "accepted",
            "session_id": session_id
        })))
    }

    /// one-off 中止
    pub async fn handle_chat_abort_oneoff(
        self: Arc<Self>,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        let session_id = ctx
            .get(crate::symbio_core::SESSION_ID)
            .unwrap_or_else(|| "default".to_string());
        if session_id.is_empty() || session_id == "default" {
            return Err(PluginError::ValidationError("session_id 不能为空".into()));
        }
        let state = self.active_mgr.get_or_create(&session_id).await;
        self.handle_abort(&state).await;
        Ok(PluginPayload::new(&serde_json::json!({
            "status": "aborted",
            "session_id": session_id
        })))
    }

    pub async fn broadcast_frame(&self, state: &Arc<ActiveSessionState>, frame: PluginFrame) {
        let mut inner = state.inner.write().await;
        let mut to_remove = Vec::new();
        for (idx, tx) in inner.frontends.iter().enumerate() {
            if tx.send(frame.clone()).await.is_err() {
                to_remove.push(idx);
            }
        }
        for idx in to_remove.into_iter().rev() {
            inner.frontends.remove(idx);
        }

        // 同时通过 EventBus 转发（供前端单连接订阅使用）
        if let PluginFrame::Data(data) = &frame {
            EventBus::try_publish("session", Some(&state.request_id_str()), data.clone());
        }
    }

    pub async fn broadcast_status(&self, state: &Arc<ActiveSessionState>, status: &str) {
        self.broadcast_frame(
            state,
            PluginFrame::Data(json!(session_chat_response::StreamEvent::Status {
                status: status.to_string()
            })),
        )
        .await;
    }

    /// 把"仍在进行中"的 AI 响应消息持久化为 `Failed` + 错误原因。
    ///
    /// ## 触发场景
    ///
    /// - 业务错误路径（Model 插件返回 Err / 透传 PluginFrame::Error）
    /// - 后台任务 **panic 崩溃**（由 `WorkingGuard::drop` 调用）
    ///
    /// ## 持久化策略
    ///
    /// 直接 `replace_messages(collected)` 会**覆盖掉整段历史**（用户之前的消息也会被清掉），
    /// 所以这里走"载入整会话 → 按 id 合并 collected → 整体替换"：
    /// - 已存在的消息（Model 插件此前可能已落库为 Completed）→ 标 `Failed` + `error`
    /// - 不存在的消息（例如崩溃时刚流式出来、尚未落库的那一条）→ 追加并标 `Failed`
    ///
    /// 这样切回会话时（`get_messages`）能看到上次的失败终态与原因（CHAT_FLOW_ANALYSIS 目标 3）。
    async fn persist_failure(
        &self,
        state: &Arc<ActiveSessionState>,
        session_id: &str,
        collected: &Arc<tokio::sync::Mutex<Vec<cm::ChatMessage>>>,
        error: &str,
    ) {
        // 1. 先在本任务的内存镜像里把 streaming/pending 的消息标 Failed + error，
        //    供下面的持久化与任何仍持有该 Arc 的读取方使用。
        {
            let mut c = collected.lock().await;
            for m in c.iter_mut() {
                if matches!(
                    m.status,
                    None | Some(cm::MessageStatus::Streaming) | Some(cm::MessageStatus::Pending)
                ) {
                    m.status = Some(cm::MessageStatus::Failed);
                    if m.error.is_none() {
                        m.error = Some(error.to_string());
                    }
                }
            }
        }

        // 2. 载入整会话并合并。
        let chat_session = match self.open_chat_session(session_id).await {
            Ok(cs) => cs,
            Err(e) => {
                crate::plugin_error!("session", "persist_failure: open session failed: {}", e);
                return;
            }
        };
        let mut all = match chat_session.get_messages().await {
            Ok(m) => m,
            Err(e) => {
                crate::plugin_error!("session", "persist_failure: get_messages failed: {}", e);
                return;
            }
        };

        let c = collected.lock().await;
        for cm in c.iter() {
            // 失败任务的根级 Turn（msg_type=Turn 且 parent_id 为空）必须回滚为 Failed：
            // 即便它已被 finalize 为 Completed（例如 turn 循环在 finalize 之后、工具阶段或
            // 下一轮迭代才抛出 PluginFrame::Error，或 panic 发生在 finalize 之后），
            // 也应强制标 Failed 并持久化。否则前端实时看到的「失败 + 重试」在会话重载后
            // 变回 Completed，错误显示与重试入口消失（Bug 1：错误状态未正确持久化）。
            // 非根消息（如 resume 已 Completed 的工具结果）仍沿用原保护逻辑，避免误标——
            // resume 重跑工具时若自身再 panic，其 Completed 结果不应被回滚为 Failed。
            let is_root_turn =
                cm.msg_type == Some(cm::MessageType::Turn) && cm.parent_id.is_none();
            match all.iter_mut().find(|m| m.id == cm.id) {
                Some(existing) => {
                    if is_root_turn
                        || matches!(
                            existing.status,
                            None | Some(cm::MessageStatus::Streaming)
                                | Some(cm::MessageStatus::Pending)
                        )
                    {
                        existing.status = Some(cm::MessageStatus::Failed);
                        if existing.error.is_none() {
                            existing.error = Some(error.to_string());
                        }
                    }
                }
                None => {
                    let mut new = cm.clone();
                    new.status = Some(cm::MessageStatus::Failed);
                    new.error = Some(error.to_string());
                    all.push(new);
                }
            }
        }
        drop(c);

        // 3. 先基于「最终将被持久化的 `all` 镜像」收集失败终态快照，
        //    再 replace（replace_messages 会拿走 all 所有权），最后广播快照，
        //    保证前端实时态与存储态完全一致（服务端权威推送 Failed 终态，前端只信服务端）。
        let failed_snapshot: Vec<cm::ChatMessage> = all
            .iter()
            .filter(|m| m.status == Some(cm::MessageStatus::Failed))
            .cloned()
            .collect();

        if let Err(e) = chat_session.replace_messages(all).await {
            crate::plugin_error!("session", "persist_failure: replace_messages failed: {}", e);
        }

        // 广播失败终态（含 Turn）：Turn 失败 → 组级错误条 + retry_turn 入口；
        // 工具结果失败 → 工具行重试。此前只落库、实时画面靠前端启发式标记，
        // 刷新前后可能不一致；现改为服务端权威推送 Failed 终态，前端只信服务端。
        // 推送发生在 Error 事件之前（调用方先 persist_failure 再 broadcast_error_with_idle），
        // Error 事件仅承担 transport 级兜底语义。
        for m in failed_snapshot {
            self.broadcast_frame(
                state,
                PluginFrame::Data(json!(session_chat_response::StreamEvent::Update {
                    message: m,
                })),
            )
            .await;
        }
    }
}
