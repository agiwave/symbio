//! 主循环的**副作用出口**：落库、广播、流式占位、开会话、生命周期钩子。
//!
//! 集中在此的目的：让主循环与单轮逻辑只表达"做什么"，把"写到哪里"收在一处。

use super::*;

/// 封根 Turn：广播本轮组合节点的终态（**唯一**发射点）。
///
/// 调用位置固定在 `close_turn` 返回之后——Turn 是组合节点（仅分组、无正文），
/// 终态必须**跟随子树**，而 `close_turn` 归来恰好意味着本轮全部子节点都已收敛
/// （ToolCall 由执行方定格、结果子节点已就位）。放在 `finalize_assistant_turn`
/// （LLM 流结束那一刻）会早整整一个执行窗口，前端于是看到"轮次已完成、其中的
/// 工具调用仍在运行"。
///
/// 发的是**权威转写里的那条节点**（`build_assistant_messages` 的产物，也是
/// `persist_messages` 即将落库的那份），不是手拼的半截快照：帧里带的身份、`meta`
/// 与状态就是权威副本上那份，手拼会漏掉先前帧写下的场景字段。
/// 节点不在转写里 = 本轮根本没建立过（异常路径），静默返回。
pub(crate) async fn finalize_turn_root(sink: &EventSink, context: &SessionContext, root_id: &str) {
    let Some(mut node) = context.messages.iter().find(|m| m.id == root_id).cloned() else {
        return;
    };
    node.status = Some(MessageStatus::Completed);
    // Turn 是组合节点（仅分组、无正文）⇒ 状态帧。
    emit_state(sink, node).await;
}

pub(crate) async fn persist_messages(
    context: &SessionContext,
    last_saved: usize,
    sink: &EventSink,
) {
    let new_messages = &context.messages[last_saved..];
    if new_messages.is_empty() {
        return;
    }

    if let Err(e) = context.session.append_messages(new_messages.to_vec()).await {
        // 持久化失败（可恢复）：不静默吃错误，也不中断对话（消息仍在内存中，
        // chat_loop 继续）。错误是**状态**不是事件——发 `Warn` 由消费循环写入
        // 会话节点 `attributes.warning`，前端按状态渲染；新一轮请求开始时清除。
        let msg = format!("消息持久化失败（消息仍在内存中）: {}", e);
        plugin_warn!("session", "[Session] {}", msg);
        // 告警是会话节点状态（VDFS watch 域），经出口的告警通道下发，不是消息帧。
        sink.warn(Some(msg)).await;
    }
}

/// 从 ctx 读取 session 编排器交付的会话引擎句柄（SESSION_HANDLE）。
///
/// session 编排在路由 `model/chat` 前已将构造好的会话引擎实例放入 chat_ctx，
/// 本函数按无状态协议工作，不反向路由 `session/open`。仅句柄缺失（异常编排路径）
/// 时回退内存会话：复用 [`PersistentChatSession::detached`]（默认配置 + 内存存储后端），
/// 不再另写一份 `ChatSession` 实现（审计 B1）——原先的 `FallbackChatSession` 与
/// `EphemeralChatSession` 是同一契约的额外两份实现，缺孤儿清理与轮次窗口，与持久版行为漂移。
pub(crate) async fn open_chat_session(ctx: &Arc<dyn InvokeRequest>) -> Arc<dyn ChatSession> {
    if let Some(handle) = ctx.get(SESSION_HANDLE) {
        return handle.0.clone();
    }

    plugin_warn!(
        "session",
        "[Session] 上下文未交付 SESSION_HANDLE，回退内存会话（无持久化）"
    );
    Arc::new(PersistentChatSession::detached(
        "ephemeral",
        SessionConfig::default(),
    ))
}

pub(crate) async fn fire_user_prompt_submit_hook(
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

pub(crate) async fn emit_streaming_start(sink: &EventSink, root_id: &str, turn: Option<usize>) {
    let meta = turn.map(|t| serde_json::json!({"turn": t}));
    // Turn 组合节点无正文：身份 + 状态一帧到位。
    sink.emit(ChatMessage {
        id: root_id.to_string(),
        // Turn 是根级节点，与 User 互为兄弟（请求/响应由 MessageRole 区分）
        parent_id: None,
        role: Some(MessageRole::Assistant),
        msg_type: Some(MessageType::Turn),
        status: Some(MessageStatus::Streaming),
        meta,
        ..Default::default()
    })
    .await;
}

/// 触发 Stop 钩子（委托给 [`StopSignal`]，幂等；上下文已在信号创建时绑定）。
///
/// `run_chat_loop` 的**每一个**出口都应先调用本函数（软上限出口用
/// `context.messages`，其余用入参 `messages`），以携带准确的 `last_message`。
/// 即便全部出口都漏调，`StopSignal::drop` 也会兜底补发一次——不变式
/// 「一个请求生命周期内 Stop 恰好一次」因此由生命周期保证，而非依赖人工记忆。
pub(crate) async fn fire_stop_hook(orchestrator: &ChatOrchestrator, messages: &[ChatMessage]) {
    orchestrator.stop.fire(messages).await;
}

// ── 测试（实现与测试分文件）────────────────────────────────────────────
