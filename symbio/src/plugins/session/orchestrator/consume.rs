//! 消费循环：`run_chat_loop` 的启动、帧消费与终态收尾。
//!
//! - `fail_before_loop`：进入循环**之前**的失败降级（显式补发 Stop，保证
//!   Working → Stop 严格配对，不走 `StopSignal::drop` 兜底以免污染告警通道）；
//! - `run_chat_loop_task`：主消费循环 —— spawn `run_chat_loop` → 收 sub_channel 帧
//!   （Error → 持久化失败 + 广播；Data → 合并收集 + 透传广播）→ 正常结束清理；
//! - `handle_abort`：中止入口（投 Abort 帧 + 等 chat_loop 收敛）。
//!
//! ⚠️ **事故敏感区**：帧合并顺序与 `StopSignal` 配对语义本次**逐字未改**。
//!
//! 可见性：`run_chat_loop_task` 被 `entry.rs` 的 `handle_chat_send_oneoff` 调用，
//! 故标 `pub(super)`。

use super::*;

impl SessionPlugin {
    /// 进入 chat_loop **之前**就失败的统一收尾（provider 解析 / 能力访问器缺失等）。
    ///
    /// 与正常出口一样保证 Working → Stop 严格配对：这里**显式**补发一次 Stop，
    /// 而不是依赖 `StopSignal::drop` 兜底——兜底路径会打"显式触发点未执行"的
    /// warn（那是为真正的漏调准备的告警），而本函数是已知的生命周期终点，
    /// 属于正常语义，不应污染告警通道（session-mechanism-unification.md §4.4）。
    ///
    /// 返回 `()`；调用方随后置 `guard.done = true` 并 return。
    async fn fail_before_loop(
        &self,
        state: &Arc<ActiveSessionState>,
        session_id: &str,
        collected: &Arc<tokio::sync::Mutex<Vec<cm::ChatMessage>>>,
        stop: &super::super::chat_loop::StopSignal,
        error: impl Into<String>,
    ) {
        let err = error.into();
        crate::plugin_error!("session", "{}", &err);
        self.persist_failure(state, session_id, collected, &err)
            .await;
        // 本轮无 transcript（loop 未产生任何消息）→ last_message 为空串
        stop.fire(&[]).await;
        self.broadcast_error_with_idle(state, err).await;
    }

    /// 统一的 chat_loop 任务执行器（`handle_chat_message` 与 continuation 共用）。
    ///
    /// 职责：
    /// - 构造 `WorkingGuard`（保障 panic 时 `is_working` 收敛 + 失败持久化）
    /// - 解析 provider（entry 回退链）→ 限流 → 进程内 spawn `run_chat_loop`
    /// - 接收 sub_channel 帧：Error → 持久化失败 + 广播；Data → 合并收集 + 透传广播
    /// - 正常结束：清理 `is_working` + 广播 idle
    ///
    /// `chat_ctx` 应已设置好 PATH/SESSION_ID/AGENT_ID/WORKDIR/payload 等所有字段。
    pub(super) async fn run_chat_loop_task(
        self: Arc<Self>,
        state: Arc<ActiveSessionState>,
        chat_ctx: Arc<dyn InvokeRequest>,
        session_id: String,
        parent: Arc<dyn Plugin>,
        provider_id: Option<String>,
        rid: u64,
    ) {
        crate::plugin_info!(
            "session",
            "[Session] Turn 任务启动：session={session_id}, rid={rid}, provider={:?}",
            provider_id
        );
        // 在途转写缓冲**取自会话状态本身**（不是新建第二份）：VDFS 转写列表
        // 因此能在流式期间读到本轮消息，而不必等 `persist_messages` 落库。
        // 开始前清空——上一轮的收尾已清过一次，这里再清一次是防 panic 残留。
        let collected_ai_messages = state.live_messages.clone();
        collected_ai_messages.lock().await.clear();

        let stop = Arc::new(super::super::chat_loop::StopSignal::new(
            Some(parent.clone()),
            chat_ctx.fork(),
        ));

        // 工作态守卫：保障 panic / 异常退出时 is_working 收敛 + 失败持久化 + Stop 兜底。
        // 正常结束路径会在收尾前把 `done` 置 true（见块末尾）。
        let mut guard = WorkingGuard {
            state: state.clone(),
            plugin: self.clone(),
            collected: collected_ai_messages.clone(),
            session_id: session_id.clone(),
            stop: stop.clone(),
            done: false,
        };

        // ── 进程内直连：解析 provider entry → 限流 → spawn run_chat_loop ──
        // Provider 解析回退链：
        // 精确 id → is_default → 首个已注册（traverse 仅注册 enabled provider）。
        let manager = match chat_ctx.get(crate::symbio_core::CAPABILITY_VISITOR) {
            Some(m) => m,
            None => {
                self.fail_before_loop(
                    &state,
                    &session_id,
                    &collected_ai_messages,
                    &stop,
                    "CAPABILITY_VISITOR 不可用，无法解析 Model Provider".to_string(),
                )
                .await;
                // 收尾已在本分支自行完成（persist_failure + Stop + Error 广播 + idle）：
                // 必须置 done=true，否则 WorkingGuard::drop 会再跑一遍崩溃恢复，
                // 导致同一失败被重复落库、前端收到两条 Error 事件。
                guard.done = true;
                return;
            }
        };
        // provider_id 参数仅为错误文案保留：model 插件 traverse 已按
        // ctx[PROVIDER_ID] > default > 首个 enabled 完成解析并注册唯一生效 Provider，
        // session 侧直接取用，不重复回退链。
        let provider = match manager.get_model_provider().await {
            Some(p) => p,
            None => {
                self.fail_before_loop(
                    &state,
                    &session_id,
                    &collected_ai_messages,
                    &stop,
                    format!("未找到可用的 Model Provider（requested={provider_id:?}）"),
                )
                .await;
                // 收尾已在本分支自行完成（persist_failure + Stop + Error 广播 + idle）：
                // 必须置 done=true，否则 WorkingGuard::drop 会再跑一遍崩溃恢复，
                // 导致同一失败被重复落库、前端收到两条 Error 事件。
                guard.done = true;
                return;
            }
        };

        // Provider 级限流（RATE_LIMITER 归属 session 插件，0 表示不限流）
        super::super::rate_limit::RATE_LIMITER
            .wait(provider.provider_id(), provider.rate_limit_ms())
            .await;

        // 进程内双向通道：host 侧（消费循环 + abort 控制）/ plugin 侧（run_chat_loop）。
        // 运行时上下文收敛在 provider.effective_context_tokens() 内部完成（服务端
        // 上报值与用户设置取 min）；此处仅保留降档可见日志。
        let (host_chan, plugin_chan) = PluginChannel::pair(4096);
        let context_limit = provider.effective_context_tokens().await;
        if context_limit < provider.max_context_tokens() {
            crate::plugin_info!(
                "session",
                "模型服务上报最大上下文 {context_limit}，低于用户设置 {}，运行时采用较小值",
                provider.max_context_tokens()
            );
        }
        let orchestrator = super::super::chat_loop::ChatOrchestrator::new(
            provider,
            Some(parent),
            context_limit,
            stop.clone(),
        );
        let ctx_clone = chat_ctx.fork();
        let error_tx = plugin_chan.tx.clone();

        // keepalive：保证 `plugin_chan.rx` 在 loop 运行期间至少有一个 sender，
        // 避免 `wait_for_abort_signal` 在真正发起请求前误判"通道关闭 → Aborted"。
        let host_tx_keepalive = host_chan.tx.clone();

        tokio::spawn(async move {
            // 持有 keepalive 直到 chat_loop 结束
            let _host_tx_keepalive = host_tx_keepalive;

            let result = tokio::task::spawn(async move {
                super::super::chat_loop::run_chat_loop(&orchestrator, ctx_clone, plugin_chan).await
            })
            .await;

            match result {
                Ok(Ok(_)) => {
                    crate::plugin_info!("session", "[ChatLoop] run_chat_loop 正常结束");
                }
                Ok(Err(e)) => {
                    crate::plugin_error!(
                        "session",
                        "[ChatLoop] run_chat_loop 异常退出: {e} (code={})",
                        e.code()
                    );
                    // 复用 PluginError::to_frame：错误码的线上表示只有一处定义
                    let _ = error_tx.send(e.to_frame()).await;
                }
                Err(e) => {
                    // join error（含 panic）：面向前端只回通用文案（不泄漏内部细节），
                    // 但**日志侧必须保留 panic 载荷**——`JoinError::to_string()` 含 panic
                    // 消息与 `file:line:col`，是定位的唯一线索；若只依赖 stderr 的
                    // 默认 panic hook，日志文件/结构化日志里将查不到。
                    let detail = e.to_string();
                    let msg = if e.is_panic() {
                        crate::plugin_error!(
                            "session",
                            "[ChatLoop] chat_loop 任务 panic，已被 join 隔离：{detail}"
                        );
                        "服务器内部发生未预期的错误".to_string()
                    } else {
                        crate::plugin_error!(
                            "session",
                            "[ChatLoop] 任务 join 失败（cancelled）：{detail}"
                        );
                        format!("Chat loop task failed: {detail}")
                    };
                    // 错误码显式标注 INTERNAL_ERROR：前端可据此区分"服务端异常"
                    // 与"业务失败"；缺省（None）会让落库/展示侧丢失分类信息。
                    let _ = error_tx
                        .send(crate::symbio_core::PluginFrame::Error(
                            msg,
                            Some(serde_json::json!({
                                "code": crate::symbio_core::ErrorCode::InternalError.as_str()
                            })),
                        ))
                        .await;
                }
            }
        });

        // 消费循环：消费 host 侧通道（原消费 parent.route 返回的 sub_channel，
        // 帧处理逻辑零改动）。
        //
        // 控制通道登记/注销成对出现，注销由 `AiControlGuard` 的 Drop 兜底——
        // 因此提前 `return` 与 panic 都不会漏掉清理（历史上正是漏清理导致
        // `handle_abort` 的"子任务是否仍在运行"判据永久为假）。
        {
            let mut sub_channel = host_chan;
            let mut ai_control_guard = {
                let mut inner = state.inner.write().await;
                inner.ai_control_tx = Some(sub_channel.tx.clone());
                AiControlGuard {
                    state: state.clone(),
                    armed: true,
                }
            };

            plugin_debug!(
                "session",
                "[Consume] 消费循环启动（session={session_id}, rid={rid}）"
            );

            let mut consume_frames: u64 = 0;
            let consume_started = std::time::Instant::now();
            loop {
                // 1800s 无任何帧 → 判定链路挂死（LLM 流挂起且未触发空闲超时等）。
                // 旧实现 `while let Ok(...)` 在超时后**静默退出**：无日志、在途 Turn
                // 不落库、前端 Turn 永远停在 Streaming——这是"卡死但任务不结束"的
                // 根因之一。现在显式收尾：日志 + persist_failure + Error 广播。
                let frame_opt = match tokio::time::timeout(
                    Duration::from_secs(1800),
                    sub_channel.rx.recv(),
                )
                .await
                {
                    Ok(f) => f,
                    Err(_) => {
                        let msg = format!(
                            "消费循环超时：{} 秒内未收到任何帧，疑似 LLM/工具链路挂死，已强制收尾（在途 Turn 标记为失败）",
                            1800
                        );
                        crate::plugin_error!("session", "[Consume] {}", &msg);
                        self.persist_failure(&state, &session_id, &collected_ai_messages, &msg)
                            .await;
                        self.broadcast_error_with_idle(&state, msg).await;
                        guard.done = true;
                        // 本出口已自行完成收尾，直接 return（不再走下方 idle 广播，
                        // 避免前端收到第二条状态事件）；先显式注销控制通道登记，
                        // 让 handle_abort 的轮询立即感知（`AiControlGuard::drop`
                        // 只是 panic 兜底，正常路径不依赖 try_write 的成功）。
                        ai_control_guard.disarm().await;
                        return;
                    }
                };
                let frame = match frame_opt {
                    Some(f) => f,
                    None => {
                        crate::plugin_info!(
                            "session",
                            "[Consume] 通道关闭（chat_loop 已结束），消费循环退出：session={session_id}, 帧数={consume_frames}, 运行时长={}s",
                            consume_started.elapsed().as_secs()
                        );
                        break;
                    }
                };
                consume_frames += 1;
                if !state.inner.read().await.is_working
                    || state.request_id.load(Ordering::SeqCst) != rid
                {
                    crate::plugin_info!(
                        "session",
                        "[Consume] 会话状态已复位或请求已更换（is_working/request_id 变更），消费循环退出：帧数={consume_frames}"
                    );
                    break;
                }
                match &frame {
                    PluginFrame::Error(msg, _) => {
                        crate::plugin_error!(
                            "session",
                            "[Consume] 收到 Error 帧（code={:?}）：{msg}",
                            frame.error_code()
                        );
                        // 用户手动中止（run_chat_loop 冒泡的 Err(PluginError::Aborted)，
                        // 错误帧携带 code=ABORTED）：在途 Turn 落库为 Failed + error，
                        // 前端据此渲染错误条与重试入口；但不广播业务 Error 事件——
                        // 中止不是错误，Status idle 足以收敛 UI（handle_abort 随后
                        // 广播的 Abort 事件负责清理流式动画）。
                        // 旧实现 run_chat_loop 对 Aborted 直接 return Ok(())，在途
                        // Turn 既不落库也无重试入口（刷新即消失的幽灵节点）。
                        // 分派依据为类型化错误码（ErrorCode::Aborted），不再对
                        // meta["code"] 做字符串字面量比较（session-mechanism-unification.md §4.2）。
                        let is_abort = frame.is_abort();
                        if is_abort {
                            // 在途 Turn 落库为 Failed + error（前端错误条 + 重试入口），
                            // 随后 break 走循环后的统一收尾：注销 ai_control_tx（让
                            // handle_abort 的轮询立即感知、免等 3s 兜底）→ 复位
                            // is_working → guard.done → 广播 idle。
                            // 中止**不是**提前收尾出口（不像 watchdog/业务 Error 那样
                            // 自行广播过 idle），因此必须 break 到统一收尾。
                            self.persist_failure(
                                &state,
                                &session_id,
                                &collected_ai_messages,
                                "用户手动中止了本次回复",
                            )
                            .await;
                            break;
                        }
                        // 透传 plugin-level Error 帧作为业务级 Error 事件。
                        // 同时把"仍在进行中"的 AI 消息持久化为 Failed + 错误原因，
                        // 这样切回会话时能看到上次失败的终态。
                        self.persist_failure(&state, &session_id, &collected_ai_messages, msg)
                            .await;
                        // 复位 is_working + 广播 Error + 广播 idle：
                        // 必须复位 is_working，否则后续 resume 请求会被
                        // `handle_chat_send_oneoff` 的 session_busy 守卫静默拒绝，
                        // 导致用户点重试无任何反应（LLM 失败重试不生效 bug 的根因）。
                        self.broadcast_error_with_idle(&state, msg.clone()).await;
                        guard.done = true;
                        // 同 watchdog 出口：本出口已自行完成 persist_failure + Error
                        // 广播 + idle，直接 return；显式注销控制通道登记（Drop 仅兜底）。
                        ai_control_guard.disarm().await;
                        return;
                    }
                    PluginFrame::Data(data) => {
                        // 收集 Model 响应消息：StreamEvent::Update 增量合并，
                        // 确保持久化的终态反映最后已知状态（而非首个 Streaming 帧）。
                        //
                        // 选此处做「合并 + 广播」的收口，而非某个 `emit_update`
                        // 调用点：本循环是**全部**补丁（模型流式、工具执行、嵌套子
                        // 会话、审批节点、恢复重写）汇入前端的必经之路——上游有多少
                        // 个发射点都无所谓，到这里只剩一个。`session_id` 也在作用域内。
                        match serde_json::from_value::<session_chat_response::StreamEvent>(
                            data.clone(),
                        ) {
                            Ok(session_chat_response::StreamEvent::Update { message }) => {
                                // 三件事一次算清：是否已存在、追加了什么、合并后的全貌。
                                // `merged` 是**变更载荷**的来源（`created` / `updated`
                                // 要带上完整节点视图与内容快照），与下发给前端的
                                // 增量 `message` 是两回事，不可互相替代。
                                let (existed, appended, merged) = {
                                    let mut collected = collected_ai_messages.lock().await;
                                    match collected.iter_mut().find(|m| m.id == message.id) {
                                        Some(existing) => {
                                            let delta = merge_message_patch(existing, &message);
                                            (true, delta, existing.clone())
                                        }
                                        None => {
                                            collected.push(message.clone());
                                            (false, None, message.clone())
                                        }
                                    }
                                };
                                self.emit_message_patch(
                                    &state,
                                    &session_id,
                                    message,
                                    &merged,
                                    existed,
                                    appended,
                                )
                                .await;
                            }
                            // 删除帧（工具恢复时删除旧的 pending/failed 子节点）：
                            // 转成 VDFS `deleted` 变更——它同样是**消息级变更**，
                            // 必须与 `created` / `updated` / `appended` 走同一条通道，
                            // 否则 VDFS 列表会残留一个已被删掉的节点（且永不纠正）。
                            Ok(session_chat_response::StreamEvent::Delete { message_id }) => {
                                self.change_subs.notify(&vdfs::VdfsChange::new(
                                    super::super::plugin::message_path(&session_id, &message_id),
                                    vdfs::VDFS_CHANGE_DELETED,
                                ));
                            }
                            // 非 Update 帧（Status / Error / Abort / Connected …）
                            // 不含消息补丁，原样透传。
                            _ => self.broadcast_frame(&state, frame).await,
                        }
                    }
                }
            }
            // 正常通道关闭：chat_loop 已自行收尾，此处注销控制通道登记。
            // 显式 `disarm` 而非等 Drop——避免多一次调度延迟；两条路径都幂等安全。
            ai_control_guard.disarm().await;
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
        // 走到这里说明是「正常通道关闭」或「abort 后 break」两条出口之一——
        // 两者都还没做过状态收敛，统一在此收尾。
        // （业务 Error 帧 / 消费超时两条提前出口已在循环内自行完成
        //  `persist_failure` + Error 广播 + idle 并直接 return，不会走到这里，
        //  因此前端不会收到第二条 idle 状态事件。）
        //
        // 在途缓冲此刻可以清空：通道关闭意味着 `run_chat_loop` 已返回，而
        // `chat_loop::persist_messages` 在返回前就已把本轮消息落库——转写的
        // 权威副本已经回到存储，继续叠加在途副本只会让同一条消息出现两次。
        collected_ai_messages.lock().await.clear();
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
        crate::plugin_info!(
            "session",
            "[Abort] 收到中止请求：控制通道{}（{}）",
            if abort_sent {
                "存在，Abort 帧已发送"
            } else {
                "已置空，走 3s 兜底强制收敛"
            },
            if abort_sent {
                "等待 chat_loop 自行退出"
            } else {
                "is_working 将被直接复位"
            }
        );

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
}
