//! 请求入口：`chat/send` 与 `chat/abort` 两个 one-off 入口。
//!
//! - `resolve_session_params`：`mode` / `risk_level` / `provider_id` 三者的解析链
//!   （`req` 字段 > `session.metadata` > 默认值），结果同时 `set` 回 `ctx`；
//! - `handle_chat_send_oneoff`：装配层（参数解析 + 落库 + 自动命名 + ctx 装配 →
//!   交付 chat_ctx 给 `consume.rs` 的消费循环）；
//! - `handle_chat_abort_oneoff`：中止入口；
//! - `ensure_auto_title`：首个用户消息落盘后的自动命名（顺带发 VDFS 变更）。
//!
//! 可见性：`handle_chat_send_oneoff` / `handle_chat_abort_oneoff` 被 `plugin.rs`
//! 的路由与 `heartbeat.rs` 跨模块调用（原本即 `pub`）。

use super::*;

impl SessionPlugin {
    // ============ One-off 模式（统一事件总线场景）============

    /// 统一解析会话参数（mode / risk_level / provider_id）。
    ///
    /// 解析链（三者完全对称）：`req` 字段 > `session.metadata` > 默认值。
    /// - mode: 默认 `auto`
    /// - risk_level: 默认 `medium`
    /// - provider_id: 默认 `none`（Model 插件使用默认 Provider）
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
            .unwrap_or_else(|| "auto".to_string());
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

        // 1. 统一 session_id 解析与校验（头 → 请求体 → 报错，见 resolve_required_session_id）
        let session_id = resolve_required_session_id(&ctx, req.session_id.as_deref())?;

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
        // 两种情况互斥：历史上只校验"至少有一个"，两者同时提供时并不报错，而是
        // 把 message 追加进存储、同时按 resume 分支发起请求 —— 用户消息落盘却
        // 不作为本轮驱动输入（仅因 load_history=true 才间接可见），属静默的语义分裂。
        // 现显式拒绝，让调用方二选一。
        if resume.is_none() && user_msg.is_none() {
            return Err(PluginError::ValidationError(
                "必须提供 message 或 resume".into(),
            ));
        }
        if resume.is_some() && user_msg.is_some() {
            return Err(PluginError::ValidationError(
                "message 与 resume 互斥，不能同时提供".into(),
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
            this_spawn
                .emit_session_state(&state_spawn, SessionStateChange::Working)
                .await;

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
                    // 自动命名：首个用户消息落盘后，尚无标题的会话从内容生成并持久化
                    this_spawn.ensure_auto_title(&sid_spawn).await;
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

            // ── 向 model/chat 交付会话引擎句柄（SESSION_HANDLE）──
            // model 不再反向路由 session/open，直接从 ctx 读句柄；
            // 交付失败仅记日志：model 侧回退不落盘的内存会话
            // （`PersistentChatSession::detached`，同一引擎逻辑 + `SessionStore::ephemeral`，
            // 审计 B1）
            // （与原路由失败路径等价，不阻断会话）。
            if let Ok(session) = this_spawn
                .open_session_handle(Some(sid_spawn.clone()))
                .await
            {
                chat_ctx.set(
                    super::super::chat_session::SESSION_HANDLE,
                    std::sync::Arc::new(super::super::chat_session::ChatSessionHandle::new(
                        session,
                    )),
                );
            } else {
                crate::plugin_warn!("session", "会话引擎句柄构造失败，chat 将回退内存会话");
            }

            let tool_visitor = collect_capabilities(Some(&parent_spawn), &chat_ctx).await;

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

            attach_capabilities(&chat_ctx, tool_visitor);

            // ── 基础提示词：**不在这里拼** ──
            // 各层指令与记忆（智能体自身目录归 `agent`，工作区归 `work`，会话归本插件）
            // 都由插件在能力收集期经 `register_system_prompt` 注册，由
            // `chat_loop::inputs::resolve_system_prompt` 统一拼接。曾经在这里把
            // `{homedir}/AGENTS.md` 装配成 `req.system_prompt` 传入，后果有二：
            // ① 同一份工作区 AGENTS.md 被 work 插件再注入一次（进两次上下文）；
            // ② 显式 `system_prompt` 会**顶掉**模型插件注册的人格——用户写了一份指令，
            // 模型却换了个人格。两处都已按「一个作用域一个所有者」收口。
            //
            // 因此下面各分支一律传 `system_prompt: None`（ping 分支除外：那是刻意的
            // 极简探活提示，不参与正常编排）。

            // 构造 model_chat::Request：会话派生字段收敛为单一 base（历史上三个分支
            // 近重复构造 10 个字段，改一处漏两处），分支只覆盖真正不同的 4 个字段。
            let req_base = model_chat::Request {
                system_prompt: None,
                single_message: None,
                stream: Some(true),
                // 0 = 不限制 → None（chat_loop 的无限轮次语义），>0 才是显式软上限
                max_tool_rounds: session_cfg.model_chat_max_tool_rounds(),
                tool_context_window: Some(session_cfg.tool_context_window),
                auto_compress: Some(session_cfg.auto_compress),
                enable_compact_tool: Some(session_cfg.enable_compact_tool),
                provider_id: pid_clone.clone(),
                load_history: None,
                resume: None,
            };
            let chat_input = if let Some(tr) = resume_spawn {
                json!(model_chat::Request {
                    system_prompt: None,
                    // resume 必须加载历史以定位目标消息
                    load_history: Some(true),
                    resume: Some(tr),
                    ..req_base
                })
            } else if is_ping {
                json!(model_chat::Request {
                    system_prompt: Some("You are a helpful assistant.".to_string()),
                    single_message: user_msg_spawn,
                    load_history: include_history,
                    ..req_base
                })
            } else {
                // 普通发送：时间 / 工作区上下文挂在用户消息的 LLM prompt 上
                // （`prompt` 不持久化，每轮发送重新生成，模型始终知道"现在几点、在哪个工作区"）。
                let single = user_msg_spawn.map(|mut m| {
                    if m.role == Some(cm::MessageRole::User) {
                        m.prompt = Some(super::super::prompt::temporal_context(Some(
                            w_clone.as_str(),
                        )));
                    }
                    m
                });
                json!(model_chat::Request {
                    system_prompt: None,
                    single_message: single,
                    load_history: include_history,
                    ..req_base
                })
            };

            // Phase E-②：会话编排进程内执行，chat_ctx 仅承载 payload（不再设置跨插件 PATH）
            chat_ctx.set_payload(chat_input).ok();

            // 调用统一的 chat_loop 任务执行器（内部：entry 解析 + 限流 + spawn）
            // run_chat_loop 内部会区分 resume（turn 前处理）与 single_message（正常 turn）
            this_spawn
                .clone()
                .run_chat_loop_task(
                    state_spawn,
                    chat_ctx,
                    sid_spawn,
                    parent_spawn,
                    pid_clone.clone(),
                    rid,
                )
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
        // 会话 id：头 → 请求体（此前只看头，`unwrap_or("default")` 使其必然报错，
        // 令 RPC 直连调用方无法通过 payload 指定会话）
        let body_id = ctx.payload::<serde_json::Value>().ok().and_then(|v| {
            v.get("session_id")
                .and_then(|s| s.as_str())
                .map(|s| s.to_string())
        });
        let session_id = resolve_required_session_id(&ctx, body_id.as_deref())?;
        let state = self.active_mgr.get_or_create(&session_id).await;
        self.handle_abort(&state).await;
        Ok(PluginPayload::new(&serde_json::json!({
            "status": "aborted",
            "session_id": session_id
        })))
    }

    /// 自动命名：会话尚无显式标题（metadata.title）时，从会话内容生成并持久化。
    ///
    /// 在首个用户消息落盘后调用；规则与 [`super::super::types::Session::display_title`]
    /// 一致（首条用户文本消息首行、限长）。
    ///
    /// 落盘后发一条 VDFS 变更（`notify_change`）——标题变更不该因发起者不同而走
    /// 不同链路。VDFS 变更同时到达两类订阅者：会话清单 store 与左栏导航，二者都
    /// 按 path 重读 `vdfs/stat` 取得最新标题，因此这里无需（也无法）在事件里携带载荷。
    pub(crate) async fn ensure_auto_title(&self, session_id: &str) {
        let Ok(mut session) = self.get_or_create_session(session_id).await else {
            return;
        };
        let has_title = session
            .metadata
            .get("title")
            .and_then(|v| v.as_str())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false);
        if has_title {
            return;
        }
        let Some(title) = super::super::types::derive_session_title(&session.messages) else {
            return;
        };
        if let Some(obj) = session.metadata.as_object_mut() {
            obj.insert("title".to_string(), json!(title));
        }
        session.updated_at = crate::symbio_core::now_ms();
        if self.save_session(&session).await.is_err() {
            return;
        }

        // 标题变更 = 该会话节点的一次 `updated`，**带节点视图**：前端清单项与
        // 聊天头部标题因此一次变更即收敛，不必再回读 `vdfs/stat`
        // （自动命名与手动改名走同一条链路，不因发起者不同而分流）。
        self.notify_session_state(session_id).await;
    }
}
