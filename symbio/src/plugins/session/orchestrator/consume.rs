//! 消费循环：`run_chat_loop` 的启动、等待与终态收尾。
//!
//! ## 收口后：这里不再是「消费循环」
//!
//! 收口前，本模块是执行期的**帧消费器**：`run_chat_loop` 把消息节点事件序列化成
//! `PluginFrame::Data` 发进来，这里 `serde_json::from_value::<NodeOp>` 解回来，
//! 再按变体分派到转写或会话状态。整套机制存在的原因是——**当时「出口」只能是通道**。
//!
//! 现在出口是 [`ExecEventSink`]（进程内直连转写唯一写入点），中止是 [`ExecAbortSignal`]，
//! 于是这里只剩**一件事**：管住一个 Turn 任务的生命周期。
//!
//! | 收口前 | 收口后 |
//! |---|---|
//! | `spawn` 外层任务 → 内层 `spawn` → `join` → 把结果投回通道 → 本循环再收 | 一次 `spawn`，直接 `await` 它的 `JoinHandle` |
//! | 每帧 `from_value::<NodeOp>` + 变体分派 | 不存在（分派已随出口一起内联） |
//! | `keepalive` sender 防误判通道关闭 | 不存在（中止是信号，不是通道关闭） |
//! | `recv` 超时 + 每帧查 `is_working` | `select!` 三臂：任务结束 / 看门狗 / 状态被接管 |
//!
//! ## 出口结局与收尾（与收口前逐条对齐）
//!
//! | 结局 | 收尾 |
//! |---|---|
//! | 任务正常结束 | 清在途转写 → 复位 `is_working` → 广播 idle（`completed`） |
//! | 任务返回 `Err(Aborted)` | 在途 Turn 落库 `Aborted` → 结局 `aborted` → 走统一收尾 |
//! | 任务返回 `Err(其他)` | `persist_failure(Failed)` + Error 广播 + idle，**就地 return** |
//! | 任务 panic / 被取消 | 同上（panic 载荷只进日志，前端只回通用文案） |
//! | 1800s 无进展（看门狗） | `persist_failure(Failed)` + Error 广播 + idle，**就地 return** |
//! | 会话状态被外部复位 / 换请求 | 走统一收尾（`completed`） |
//!
//! 「就地 return」的两条出口自行完成收尾，因此前端不会收到第二条 idle 事件。
//!
//! ⚠️ **事故敏感区**：`StopSignal` 配对语义与「中止不是失败」的结局判定曾出过事故
//! （189 个 Turn 被误标 Failed），改动前先读 `docs/DECISIONS.md` 对应条目。
//!
//! 可见性：`run_chat_loop_task` 被 `entry.rs` 的 `handle_chat_send_oneoff` 调用，
//! 故标 `pub(super)`。

use super::sink::TranscriptSink;
use super::*;

/// 一个 Turn 任务的等待结局。
enum TurnOutcome {
    /// 任务结束（含 panic / 被取消）。
    Finished(Result<Result<(), PluginError>, tokio::task::JoinError>),
    /// 看门狗：1800s 内既无结束也无状态变化，判定链路挂死。
    Watchdog,
    /// 会话状态被外部复位或请求已更换（新一轮接管），本任务让位。
    Superseded,
}

/// 等待会话状态被复位或请求被更换（非阻塞轮询）。
///
/// 收口前这个判定写在帧循环体内（每收到一帧查一次）；现在没有帧了，改为独立
/// 定时器臂。200ms 粒度与收口前的实际行为相当——流式期间帧的到达间隔远小于此。
async fn wait_state_superseded(state: &Arc<ActiveSessionState>, rid: u64) {
    loop {
        tokio::time::sleep(Duration::from_millis(200)).await;
        if !state.inner.read().await.is_working || state.request_id.load(Ordering::SeqCst) != rid {
            return;
        }
    }
}

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
        collected: &Arc<tokio::sync::Mutex<crate::plugins::session::transcript::Transcript>>,
        stop: &super::super::chat_loop::StopSignal,
        error: impl Into<String>,
    ) {
        let err = error.into();
        crate::plugin_error!("session", "{}", &err);
        self.persist_failure(session_id, collected, &err, cm::MessageStatus::Failed)
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
    /// - 登记中止信号（[`AbortGuard`]）→ 等待 Turn 任务的四种结局之一 → 统一收尾
    ///
    /// `chat_ctx` 应已设置好 PATH/SESSION_ID/AGENT_ID/WORKDIR/payload 等所有字段。
    pub(super) async fn run_chat_loop_task(
        self: Arc<Self>,
        state: Arc<ActiveSessionState>,
        chat_ctx: Arc<dyn PluginInvokeRequest>,
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
        // 会话转写**取自会话状态本身**（不是新建第二份）：VDFS 转写列表的在途
        // 叠加与实时流读的是同一个 `Transcript`。开始前清空在途图——上一轮的
        // 收尾已清过一次，这里再清一次是防 panic 残留。
        let transcript = state.transcript.clone();
        transcript.lock().await.clear();

        let stop = Arc::new(super::super::chat_loop::StopSignal::new(
            Some(parent.clone()),
            chat_ctx.fork(),
        ));

        // 工作态守卫：保障 panic / 异常退出时 is_working 收敛 + 失败持久化 + Stop 兜底。
        // 正常结束路径会在收尾前把 `done` 置 true（见块末尾）。
        let mut guard = WorkingGuard {
            state: state.clone(),
            plugin: self.clone(),
            collected: transcript.clone(),
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
                    &transcript,
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
                    &transcript,
                    &stop,
                    format!("未找到可用的 Model Provider（requested={provider_id:?}）"),
                )
                .await;
                // 收尾已在本分支自行完成：必须置 done=true（理由同上）。
                guard.done = true;
                return;
            }
        };

        // Provider 级限流（RATE_LIMITER 归属 session 插件，0 表示不限流）
        super::super::rate_limit::RATE_LIMITER
            .wait(provider.provider_id(), provider.rate_limit_ms())
            .await;

        // 运行时上下文收敛在 provider.effective_context_tokens() 内部完成（服务端
        // 上报值与用户设置取 min）；此处仅保留降档可见日志。
        let context_limit = provider.effective_context_tokens().await;
        if context_limit < provider.max_context_tokens() {
            crate::plugin_info!(
                "session",
                "模型服务上报最大上下文 {context_limit}，低于用户设置 {}，运行时采用较小值",
                provider.max_context_tokens()
            );
        }
        // 压缩期要能把「正在压缩」作为**会话阶段**下发：压缩发生在 Turn 创建之前，
        // 且出帧被刻意静音，整段窗口内没有任何消息节点可渲染（长上下文时可达数
        // 分钟，用户视角＝卡死），前端只能靠会话节点的 `attributes.phase` 给出等待
        // 提示。发射器需要插件 + 会话状态：`notify_session_state` 要用 store（取会话
        // 摘要）与 `change_subs`（投递），两者都在插件上。
        let phase = Some(std::sync::Arc::new(
            super::super::chat_loop::CompressionEmitter::new(self.clone(), state.clone()),
        ));

        // ── 本 Turn 的两个执行期原语 ────────────────────────────────────────
        //
        // 出口：进程内直连转写唯一写入点。执行期的每一次节点变更直接落进
        // `Transcript::apply`（内存图 → seq → 一行核心日志 → 发布），不再经过
        // 「序列化成帧 → 反序列化回来」的往返。
        let sink =
            ExecEventSink::direct(Arc::new(TranscriptSink::new(self.clone(), state.clone())));
        // 中止：本 Turn 唯一的入方向原语。登记进会话状态后，`handle_abort` 直接
        // 置位（无帧、无轮询）；守卫注销时一并置位——这正是收口前「消费循环
        // drop 掉通道 ⇒ 执行方中止」那条隐式语义的显式化。
        let abort = ExecAbortSignal::new();
        let mut abort_guard = AbortGuard::register(state.clone(), abort.clone()).await;

        let orchestrator = Arc::new(super::super::chat_loop::ChatOrchestrator::new(
            provider,
            Some(parent),
            context_limit,
            stop.clone(),
            phase,
        ));

        let ctx_clone = chat_ctx.fork();
        let task_orchestrator = orchestrator.clone();
        let task_abort = abort.clone();
        let handle = tokio::spawn(async move {
            super::super::chat_loop::run_chat_loop(&task_orchestrator, ctx_clone, sink, task_abort)
                .await
        });

        plugin_debug!(
            "session",
            "[Consume] Turn 任务已派发（session={session_id}, rid={rid}）"
        );

        let started = std::time::Instant::now();
        let outcome = tokio::select! {
            r = handle => TurnOutcome::Finished(r),
            // 1800s 无任何进展 → 判定链路挂死（LLM 流挂起且未触发空闲超时等）。
            // 收口前这条判定挂在 `recv` 超时上；现在挂在任务本身——语义更强：
            // 不看「有没有帧」，只看「这个 Turn 到底结束了没有」。
            _ = tokio::time::sleep(Duration::from_secs(1800)) => TurnOutcome::Watchdog,
            _ = wait_state_superseded(&state, rid) => TurnOutcome::Superseded,
        };

        // 本 Turn 的**出口结局**：中止与「正常结束」走同一段收尾，因此必须在这里
        // 记下"是不是中止"，否则收尾会把用户中止报成「正常完成」。
        // 与 `handle_abort` 共用 `SessionStateChange::aborted()` 这一个构造点——
        // 两个写者写同一个值，时序竞态因此无害。
        let mut exit_state = SessionStateChange::completed();

        match outcome {
            TurnOutcome::Finished(Ok(Ok(()))) => {
                crate::plugin_info!(
                    "session",
                    "[Consume] Turn 任务正常结束（运行时长={}s）",
                    started.elapsed().as_secs()
                );
            }
            TurnOutcome::Finished(Ok(Err(e))) => {
                crate::plugin_error!(
                    "session",
                    "[ChatLoop] run_chat_loop 异常退出: {e} (code={})",
                    e.code()
                );
                // 分派依据为类型化错误码（PluginErrorCode::Aborted），不再对
                // meta["code"] 做字符串字面量比较（session-mechanism-unification.md §4.2）。
                if matches!(e, PluginError::Aborted) {
                    // 用户手动中止：在途 Turn 落库为 Aborted + error，前端据此渲染
                    // 错误条与重试入口；但**不收敛为「失败」结局**——中止不是错误：
                    // 会话节点回 `active` + `outcome = aborted`（由 handle_abort 收口，
                    // 流式动画随之停止）。
                    //
                    // 这里与 `handle_abort` 的 `converge_inflight` 写同一个值，因此
                    // 谁先谁后都不影响结果（此前两者分别是 `Failed` 与 `Completed`，
                    // 重试入口的出现与否取决于 abort 信号落在哪个检查点）。
                    self.persist_failure(
                        &session_id,
                        &transcript,
                        "用户手动中止了本次回复",
                        cm::MessageStatus::Aborted,
                    )
                    .await;
                    exit_state = SessionStateChange::aborted();
                } else {
                    // 业务级失败：把"仍在进行中"的 AI 消息持久化为 Failed + 错误原因，
                    // 这样切回会话时能看到上次失败的终态。
                    self.persist_failure(
                        &session_id,
                        &transcript,
                        &e.to_string(),
                        cm::MessageStatus::Failed,
                    )
                    .await;
                    // 复位 is_working + 广播 Error + 广播 idle：
                    // 必须复位 is_working，否则后续 resume 请求会被
                    // `handle_chat_send_oneoff` 的 session_busy 守卫静默拒绝，
                    // 导致用户点重试无任何反应（LLM 失败重试不生效 bug 的根因）。
                    self.broadcast_error_with_idle(&state, e.to_string()).await;
                    guard.done = true;
                    abort_guard.disarm().await;
                    return;
                }
            }
            TurnOutcome::Finished(Err(join_err)) => {
                // join error（含 panic）：面向前端只回通用文案（不泄漏内部细节），
                // 但**日志侧必须保留 panic 载荷**——`JoinError::to_string()` 含 panic
                // 消息与 `file:line:col`，是定位的唯一线索；若只依赖 stderr 的
                // 默认 panic hook，日志文件/结构化日志里将查不到。
                let detail = join_err.to_string();
                let msg = if join_err.is_panic() {
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
                self.persist_failure(&session_id, &transcript, &msg, cm::MessageStatus::Failed)
                    .await;
                self.broadcast_error_with_idle(&state, msg).await;
                guard.done = true;
                abort_guard.disarm().await;
                return;
            }
            TurnOutcome::Watchdog => {
                let msg = format!(
                    "消费循环超时：{} 秒内 Turn 任务未结束，疑似 LLM/工具链路挂死，已强制收尾（在途 Turn 标记为失败）",
                    1800
                );
                crate::plugin_error!("session", "[Consume] {}", &msg);
                self.persist_failure(&session_id, &transcript, &msg, cm::MessageStatus::Failed)
                    .await;
                self.broadcast_error_with_idle(&state, msg).await;
                guard.done = true;
                abort_guard.disarm().await;
                return;
            }
            TurnOutcome::Superseded => {
                crate::plugin_info!(
                    "session",
                    "[Consume] 会话状态已复位或请求已更换（is_working/request_id 变更），Turn 任务让位：运行时长={}s",
                    started.elapsed().as_secs()
                );
            }
        }

        // 走到这里说明是「正常结束」或「中止后收敛」两条出口之一——两者都还没做过
        // 状态收敛，统一在此收尾。
        //
        // 在途缓冲此刻可以清空：Turn 任务已结束，而 `chat_loop::persist_messages`
        // 在返回前就已把本轮消息落库——转写的权威副本已经回到存储，继续叠加在途
        // 副本只会让同一条消息出现两次。
        transcript.lock().await.clear();
        {
            let mut inner = state.inner.write().await;
            if inner.is_working {
                inner.is_working = false;
            }
        }
        guard.done = true;
        abort_guard.disarm().await;
        // 正常收尾：运行态收敛为「上一轮结束」。结局由 `exit_state` 决定——
        // 中止出口是 `aborted`（不是 `completed`），与 `handle_abort` 同一构造点。
        self.emit_session_state(&state, exit_state).await;
    }

    pub async fn handle_abort(&self, state: &Arc<ActiveSessionState>) {
        // 登记的中止信号即「在途 Turn」：取出克隆后置位（不 take——注销由
        // `AbortGuard` 负责，`handle_abort` 只负责发出中止意图）。
        let signal = state.inner.read().await.abort_signal.clone();
        let registered = signal.is_some();
        if let Some(sig) = &signal {
            sig.abort();
        }
        crate::plugin_info!(
            "session",
            "[Abort] 收到中止请求：中止信号{}（{}）",
            if registered {
                "已登记，已置位"
            } else {
                "未登记，走 3s 兜底强制收敛"
            },
            if registered {
                "等待 chat_loop 自行退出"
            } else {
                "is_working 将被直接复位"
            }
        );

        if registered {
            // 轮询等待中止信号被主动注销（最迟 3s 兜底，避免无限等待）
            //
            // 历史：原代码用 20×100ms=2s 硬编码 sleep 等候 AI 子任务退出；
            // 现在轮询登记是否被清空（见 `AbortGuard::disarm`），更准确反映
            // 子任务结束时机。3s 是兜底上限：若子任务未收敛，仍能 3s 内强制收敛。
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            loop {
                if state.inner.read().await.abort_signal.is_none() {
                    break;
                }
                if std::time::Instant::now() >= deadline {
                    crate::plugin_warn!("session", "[Abort] 中止信号未在 3s 内注销，强制收敛");
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }

        {
            let mut inner = state.inner.write().await;
            inner.is_working = false;
            inner.abort_signal = None;
        }

        // 节点收口——**中止路径自己的责任**，不能等 chat_loop。
        //
        // 上面的 3s 兜底一旦超时，说明 chat_loop 卡在某个不轮询中止信号的 await 里
        // （非流式工具执行是典型：`route_fut` 是单次 await，最长硬超时 600s）。
        // 此时 `abort_flag` 永远不置位，chat_loop 也就不会冒泡 `Err(Aborted)`；
        // 而本函数在下一帧醒来时因 `!is_working` 直接跳过 `persist_failure`。
        // 于是工具节点停在 `Streaming` 并永远转下去——「终止后还显示运行中」的根因。
        //
        // 无论 chat_loop 是否已收口，这里都跑一次：正常收口路径（Turn 任务的
        // ABORTED 分支）已把节点定稿，本调用因此是幂等的空操作。
        let converged = self
            .converge_inflight(state, &state.request_id_str(), "用户中止")
            .await;

        // 运行态收敛为「用户中止」。中止**不是**失败：会话节点的 `outcome` 记
        // `aborted`（提示音据此选音色），而 `status` 回到空闲——用户知道自己按了停止，
        // 再给一个失败角标只是噪音。
        //
        // 顺序有意：节点补丁先于会话状态。前端 `applySessionNode` 在
        // `working → 非 working` 迁移时会清掉"等待审批"角标与活动文字，因此
        // 消息节点必须先落地，否则会出现"会话已空闲、节点还在跑"的一帧。
        crate::plugin_info!(
            "session",
            "[Abort] 收口完成：{} 个在途节点定稿为 Completed",
            converged
        );
        self.emit_session_state(state, SessionStateChange::aborted())
            .await;
    }

    /// **中止某会话正在跑的那一轮**——`action(<A>, "abort")` 与 `chat/abort` 共用的
    /// 唯一实现。
    ///
    /// 返回**是否真的中止了一个在途轮次**。`false` = 这个会话此刻没在跑（含「本
    /// 实例从未跑过它」）：调用方把它翻成一句明确的话，而不是报成功——`chat/abort`
    /// 最坏的一面就是「无论停没停都回 `aborted`」。
    pub(crate) async fn abort_turn(&self, session_id: &str) -> bool {
        // **只查不建**：`get_or_create` 在这里会为一个从未跑过的会话（典型是
        // 子智能体空间的会话被错投到根实例）造一个永不释放的幽灵状态，而中止
        // 本身是空操作——调用方拿到"成功"，什么都没停。
        let Some(state) = self.active_mgr.get(session_id).await else {
            return false;
        };
        if !state.inner.read().await.is_working {
            return false;
        }
        self.handle_abort(&state).await;
        true
    }
}
