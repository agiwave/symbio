//! 会话编排器：`handle_chat_message` 主链路 + 轮次收尾 + 崩溃兜底。
//!
//! 职责：校验请求 → 预算限流（`rate_limit`）→ 构造 `ChatOrchestrator`（模型配置 /
//! 父插件钩子 / 协议适配器的 session 侧确定性持有）→ 交付 chat_ctx 并把后续轮次
//! 委托给 `chat_loop`。此外承载：
//! - `WorkingGuard`：`is_working` 工作态守卫，任何退出路径（含 panic）都收敛状态；
//! - `merge_message_patch`：流式 Update 补丁合并（对齐前端语义）；
//! - `finalize_assistant_turn`：Turn 收尾状态机；
//! - `persist_failure`：失败降级持久化（作用域收窄到失败 Turn 及其后代）。

use super::active::{ActiveSessionState, REQUEST_ID_COUNTER};
use super::chat_loop::StopSignal;
use super::chat_pipeline::{attach_capabilities, collect_capabilities};
use super::model_chat;
use super::plugin::SessionPlugin;
use crate::plugin_debug;
use crate::symbio_core::event_bus::EventBus;
use crate::symbio_core::schemas::{
    session::chat_message as cm,
    session::{session_append, session_chat, session_chat_response},
};
use crate::symbio_core::vdfs;
use crate::symbio_core::{
    take_errors, InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginChannel,
    PluginError, PluginFrame, PluginPayload, MODE, PROVIDER_ID, RISK_LEVEL, SESSION_ID, WORKDIR,
};
use serde_json::json;
use std::sync::atomic::Ordering;

use std::sync::Arc;
use std::time::Duration;

/// 解析**必填**会话 id：头 → 请求体 → 报错。
///
/// 统一此前散落在 send/resume/abort/heartbeat_trigger 各入口的重复特判。三档取值：
/// - `ctx` 头里的 `SESSION_ID`（前端 WS 帧注入，优先级最高）；
/// - `fallback`（请求体 `req.session_id`，RPC 直连调用方）；
/// - 空串或历史哨兵 `"default"` 一律视为缺失 → `ValidationError`。
///
/// **修复的行为**：abort / heartbeat_trigger 两个 one-off 入口此前不看请求体，
/// 只靠 `unwrap_or("default")` 再自我判定为非法，使 `req.session_id` 永远不可达。
/// `"default"` 作为非法值的理由并非"它是保留 id"，而是它曾是缺省占位符——
/// 拿它当真实会话去 abort/触发会静默作用于不存在的会话，宁可显式报错。
pub(crate) fn resolve_required_session_id(
    ctx: &Arc<dyn InvokeRequest>,
    fallback: Option<&str>,
) -> Result<String, PluginError> {
    let raw = ctx
        .get(SESSION_ID)
        .or_else(|| fallback.map(|s| s.to_string()));
    match raw {
        Some(id) if !id.is_empty() && id != "default" => Ok(id),
        _ => Err(PluginError::ValidationError("session_id 不能为空".into())),
    }
}

/// `ai_control_tx` 登记守卫：保证消费循环的**任何**出口都会清走登记的控制通道
/// sender——正常路径显式 `disarm`，panic unwind 由 `Drop` 兜底。
///
/// ## 为什么需要它
///
/// `handle_abort` 以「`ai_control_tx` 是否为 `None`」作为 chat_loop 子任务是否
/// 仍在运行的**唯一**判据。历史上消费循环有两处提前出口（业务 Error 帧、
/// 1800s 消费超时）写作 `return`，直接跳过了循环之后的清理块，留下指向已关闭
/// 通道的陈旧 sender——判据从此永久为假：abort 必然空等 3s 才走兜底复位，
/// 且 Abort 帧投进死通道被静默丢弃（前端收不到中止确认）。
///
/// 登记与注销收拢进同一对象后，"新增出口忘记清理"不再能静默通过：`Drop` 保证
/// 至少有一次清理必然发生，也不依赖后续维护者记住"新增出口必须穿过清理块"这条
/// 隐性契约。与 [`WorkingGuard`] 同型（后者兜 `is_working`）。
struct AiControlGuard {
    state: Arc<ActiveSessionState>,
    /// `false` 表示已清理，Drop 成为 no-op（正常路径走 `disarm` 同步清理，
    /// 避免多一次 spawn 调度延迟）。
    armed: bool,
}

impl AiControlGuard {
    async fn disarm(&mut self) {
        self.state.inner.write().await.ai_control_tx = None;
        self.armed = false;
    }
}

impl Drop for AiControlGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // Drop 中不能 await：优先 `try_write` 就地同步清理（写锁持有期极短，
        // 几乎总能拿到）；确实取不到才退回 detached spawn（与 WorkingGuard 同型）。
        if let Ok(mut inner) = self.state.inner.try_write() {
            inner.ai_control_tx = None;
            return;
        }
        let state = self.state.clone();
        tokio::spawn(async move {
            state.inner.write().await.ai_control_tx = None;
        });
    }
}

/// 工作态守卫：保障 `is_working` 在任何退出路径（包括 panic 崩溃）下都会收敛。
///
/// ## 背景
///
/// spawn 任务若发生 **panic**，该任务会被 tokio 静默终止，`is_working`
/// 将永远停留在 `true`，导致前端永久显示"AI 处理中"且无法恢复。
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
    /// 本次请求的 Stop 触发器。
    ///
    /// 正常路径下 `run_chat_loop` 的出口已显式 fire 过，此处 Drop 是 no-op；
    /// panic 路径下它是**唯一**能保证 Stop 送达的机制（chat_loop 的出口代码
    /// 根本不会执行）。字段声明顺序决定析构顺序：本字段必须在 `done` 之前
    /// 落地，故放在结构体末尾即可（Drop 中显式调用，不依赖字段析构次序）。
    stop: Arc<StopSignal>,
    done: bool,
}

impl Drop for WorkingGuard {
    fn drop(&mut self) {
        // Stop 钩子兜底（幂等）：无论正常/panic/提前 return，本请求生命周期内
        // 恰好触发一次。放在 `done` 早退之前——正常结束路径同样依赖它兜住
        // "chat_loop 出口漏调"的情形。
        self.stop.fire_fallback();
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
///
/// # 返回值 = 被追加的那段文本
///
/// `Some(delta)` 表示本次合并是**尾部追加**（正是 `appended` 变更的语义），
/// 且 `delta` 就是追加进去的那一段；`None` 表示全量替换或未触及内容。
///
/// **为什么由本函数回报，而不是让调用方另行判断**：变更类型（`appended` vs
/// `updated`）必须与合并方式**逐字一致**——若这里按追加合并、那里判成全量，
/// 消费者按 `delta` 拼接就会得到错误内容。让唯一决定合并方式的地方顺带说出
/// 它是哪种方式，这种漂移在结构上就不可能发生。
fn merge_message_patch(existing: &mut cm::ChatMessage, patch: &cm::ChatMessage) -> Option<String> {
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

    // 本次合并追加进去的文本（`Some` = 追加型，正是 `appended` 变更的载荷）
    let mut appended: Option<String> = None;

    if let Some(new_content) = &patch.content {
        // 工具流式帧（role=Tool，如 shell 的增量输出）与前端 sessions.ts 的
        // 合并语义保持一致：全量替换，而非 SSE delta 追加。
        if matches!(&patch.role, Some(cm::MessageRole::Tool)) {
            existing.content = Some(new_content.clone());
        } else {
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
                                appended = Some(new_text.clone());
                            } else {
                                // Type mismatch (rare) — fall back to replacement
                                *existing_c = cm::MessageContent::Text(new_text.clone());
                            }
                        }
                        (None, cm::MessageContent::Text(new_text)) => {
                            existing.content = Some(cm::MessageContent::Text(new_text.clone()));
                            // 首次写入也是追加：节点此前无正文，尾部多出来的就是全部
                            appended = Some(new_text.clone());
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

    appended
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
        stop: &super::chat_loop::StopSignal,
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
    async fn run_chat_loop_task(
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

        let stop = Arc::new(super::chat_loop::StopSignal::new(
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
        super::rate_limit::RATE_LIMITER
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
        let orchestrator = super::chat_loop::ChatOrchestrator::new(
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
                super::chat_loop::run_chat_loop(&orchestrator, ctx_clone, plugin_chan).await
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
                                let _ = self.change_tx.send(vdfs::VdfsChange::new(
                                    super::plugin::message_path(&session_id, &message_id),
                                    vdfs::VFDS_CHANGE_DELETED,
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
            // 交付失败仅记日志：model 侧回退内存会话（`PersistentChatSession::detached`，
            // 同一引擎逻辑 + InMemorySessionStore，审计 B1）
            // （与原路由失败路径等价，不阻断会话）。
            if let Ok(session) = this_spawn
                .open_session_handle(Some(sid_spawn.clone()))
                .await
            {
                chat_ctx.set(
                    super::chat_session::SESSION_HANDLE,
                    std::sync::Arc::new(super::chat_session::ChatSessionHandle::new(session)),
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

            // ── 基础提示词（与智能体无关）：AGENTS.md 全局 / 工作区指令 ──
            // 智能体人格不在这里——由 agent_identity 工具说明承载。
            let base_prompt = super::prompt::build_system_prompt(Some(w_clone.as_str())).await;
            let system_prompt_opt = if base_prompt.trim().is_empty() {
                None
            } else {
                Some(base_prompt)
            };

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
                    system_prompt: system_prompt_opt,
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
                        m.prompt = Some(super::prompt::temporal_context(Some(w_clone.as_str())));
                    }
                    m
                });
                json!(model_chat::Request {
                    system_prompt: system_prompt_opt,
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

    /// 自动命名：会话尚无显式标题（metadata.title）时，从会话内容生成并持久化。
    ///
    /// 在首个用户消息落盘后调用；规则与 [`super::types::Session::display_title`]
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
        let Some(title) = super::types::derive_session_title(&session.messages) else {
            return;
        };
        if let Some(obj) = session.metadata.as_object_mut() {
            obj.insert("title".to_string(), json!(title));
        }
        session.updated_at =
            (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64;
        if self.save_session(&session).await.is_err() {
            return;
        }

        self.notify_change(session_id, vdfs::VFDS_CHANGE_UPDATED);
    }

    pub async fn broadcast_status(&self, state: &Arc<ActiveSessionState>, status: &str) {
        self.broadcast_frame(
            state,
            PluginFrame::Data(json!(session_chat_response::StreamEvent::Status {
                status: status.to_string()
            })),
        )
        .await;

        // 会话的忙/闲是**节点状态**，不是流式内容：以一次 VDFS 变更通知订阅方，
        // 由列表/详情按 path 重读 `vdfs/stat` 取最新 status（角标即时刷新，
        // 初始态仍由清单接口兜底）。
        let id = state.request_id_str();
        match status {
            "busy" | "idle" => {
                self.notify_change(&id, vdfs::VFDS_CHANGE_UPDATED);
            }
            _ => {}
        }
    }

    /// 把"仍在进行中"的 AI 响应消息持久化为 `Failed` + 错误原因。
    ///
    /// ## 触发场景
    ///
    /// - 业务错误路径（Model 插件返回 Err / 透传 PluginFrame::Error）
    /// - 后台任务 **panic 崩溃**（由 `WorkingGuard::drop` 调用）
    ///
    /// ## 作用域：只有「真正在飞行中的那一个 Turn」会被降级（M-002）
    ///
    /// `collected` 是**整个 `run_chat_loop` 调用期间**累积的全部流式消息快照。
    /// 一次调用最多可跑 `max_tool_rounds` 轮（默认 0 = 不限制），因此它往往包含几十上百个
    /// **早已成功定稿**的历史 Turn——那些轮次的工具早已执行完毕、结果也早已由
    /// Model 插件在每轮结束时落库。
    ///
    /// 若不加区分地把 `collected` 里每一个根级 Turn 都回滚成 `Failed`，那么末尾一次
    /// 断流就会把**此前全部成功轮次追溯标记为失败**，并把同一条错误写进每一轮。
    /// 实测：一次上游 502 断流 → 189 个 Turn 全部 `Failed` + 同一条错误刷屏 189 次。
    ///
    /// 因此这里先定位失败 Turn（`collected` 中**最后一个**根级 Turn，即错误发生时
    /// 正在进行的那一个，因为 `collected` 按到达顺序累积），再只对**它及其子树**
    /// 做收尾；历史 Turn 一律保持存储中已有的终态，不动、不广播。
    ///
    /// ## 持久化策略
    ///
    /// 直接 `replace_messages(collected)` 会**覆盖掉整段历史**（用户之前的消息也会被清掉），
    /// 所以这里走"载入整会话 → 按 id 合并 → 整体替换"：
    /// - 失败 Turn（根级）：标 `Failed` + `error`，这是前端唯一渲染 ⚠ 错误条 + 重试入口的节点
    /// - 失败 Turn 的子节点（text / reasoning / tool_call / 工具结果）：仅把仍在进行中的
    ///   (None / Streaming / Pending) 定稿为 Completed，**绝不挂 error**
    /// - 尚未落库的子节点：补写，前提是其父节点存在（避免写入悬空孤儿节点）
    ///
    /// 这样切回会话时（`get_messages`）能看到上次的失败终态与原因（CHAT_FLOW_ANALYSIS 目标 3）。
    async fn persist_failure(
        &self,
        state: &Arc<ActiveSessionState>,
        session_id: &str,
        collected: &Arc<tokio::sync::Mutex<Vec<cm::ChatMessage>>>,
        error: &str,
    ) {
        // 1. 定位失败 Turn。
        //    `collected` 为空表示错误发生在任何 Turn 节点创建之前（例如能力收集失败、
        //    Model 插件路由直接报错）——此时没有任何节点可降级，错误属于会话级，
        //    由调用方广播的 Error 事件承载，这里直接返回。
        let failing_turn_id = {
            let c = collected.lock().await;
            c.iter()
                .rev()
                .find(|m| m.msg_type == Some(cm::MessageType::Turn) && m.parent_id.is_none())
                .map(|m| m.id.clone())
        };
        let failing_turn_id = match failing_turn_id {
            Some(id) => id,
            None => {
                crate::plugin_info!(
                    "session",
                    "persist_failure: 无在途 Turn（错误发生在 Turn 创建之前），仅广播会话级错误"
                );
                return;
            }
        };

        // 2. 收窄作用域：只取失败 Turn 及其**传递闭包**子树
        //    （Turn → text/reasoning/tool_call → 工具结果）。
        //    历史轮次的消息不在此列：它们已由 Model 插件按自己的规则落库，session 层
        //    不应再拿流式快照去"补写"一份——否则同一个 Turn 下会出现两份内容相同的
        //    文本节点（流式 id 与落库 id 不一致时的典型表现）。
        let subtree_ids = {
            let c = collected.lock().await;
            subtree_of(&c, &failing_turn_id)
        };

        // 3. 在本任务的内存镜像里定稿失败 Turn 的子树。
        //
        //    关键修复（原始 Bug：错误刷屏 + 工具节点出现服务器错误）：
        //    - 失败 Turn（根级）：Failed + error，这是前端唯一渲染 ⚠ 错误条 + 重试入口的节点。
        //    - 其余子节点（text / reasoning / tool_call）：仅把仍在进行中的
        //      (None / Streaming / Pending) 定稿为 Completed，**绝不挂 error**。
        //      理由：工具调用在本地执行、不请求 LLM 服务器，不可能产生「API 错误」；
        //      把同一个 429 刷到每条文本/工具节点既不符合逻辑，又造成错误刷屏。
        {
            let mut c = collected.lock().await;
            for m in c.iter_mut() {
                if !subtree_ids.contains(&m.id) {
                    continue;
                }
                if m.id == failing_turn_id {
                    m.status = Some(cm::MessageStatus::Failed);
                    if m.error.is_none() {
                        m.error = Some(error.to_string());
                    }
                } else if matches!(
                    m.status,
                    None | Some(cm::MessageStatus::Streaming) | Some(cm::MessageStatus::Pending)
                ) {
                    m.status = Some(cm::MessageStatus::Completed);
                    m.error = None;
                }
            }
        }

        // 4. 载入整会话并合并（合并语义，不覆盖历史）。
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

        // 仅记录本次真正发生状态变化的消息，最后只广播这些，
        // 避免对未变化的消息重复推送（也避免把错误刷屏到每条文本）。
        let mut changed: Vec<cm::ChatMessage> = Vec::new();

        // 5. 合并失败 Turn 的子树（作用域外的一律跳过：保持存储中已有的终态）。
        let c = collected.lock().await;
        for cm_msg in c.iter() {
            if !subtree_ids.contains(&cm_msg.id) {
                continue;
            }
            // 失败 Turn 本身必须回滚为 Failed：即便它已被 finalize 为 Completed
            //（例如 turn 循环在 finalize 之后、工具阶段才抛出 PluginFrame::Error，
            // 或 panic 发生在 finalize 之后），也应强制标 Failed 并持久化。否则前端实时
            // 看到的「失败 + 重试」在会话重载后变回 Completed，错误显示与重试入口消失。
            let is_failing_turn = cm_msg.id == failing_turn_id;
            match all.iter_mut().find(|m| m.id == cm_msg.id) {
                Some(existing) => {
                    if is_failing_turn {
                        // 仅失败 Turn 承载错误 + 重试入口。
                        if existing.status != Some(cm::MessageStatus::Failed)
                            || existing.error.is_none()
                        {
                            existing.status = Some(cm::MessageStatus::Failed);
                            if existing.error.is_none() {
                                existing.error = Some(error.to_string());
                            }
                            changed.push(existing.clone());
                        }
                    } else if matches!(
                        existing.status,
                        None | Some(cm::MessageStatus::Streaming)
                            | Some(cm::MessageStatus::Pending)
                    ) {
                        // 进行中的子节点定稿为 Completed（结束前端流式动画），不挂 error。
                        existing.status = Some(cm::MessageStatus::Completed);
                        existing.error = None;
                        changed.push(existing.clone());
                    }
                    // 已 Completed/Failed 的节点：原样保留，不改动、不广播。
                }
                None => {
                    // collected 中存在但存储里没有的消息（崩溃时刚流式出来、尚未落库）。
                    // - 失败 Turn 本身：补写为 Failed + error（承载重试入口）；
                    // - 子节点：定稿为 Completed 后补写，**丢弃父节点缺失的孤儿**
                    //   （避免写入无父的悬空节点污染前端渲染）。
                    let parent_exists = cm_msg
                        .parent_id
                        .as_ref()
                        .map(|pid| all.iter().any(|m| m.id == *pid))
                        .unwrap_or(true);
                    let mut new = cm_msg.clone();
                    if is_failing_turn {
                        new.status = Some(cm::MessageStatus::Failed);
                        if new.error.is_none() {
                            new.error = Some(error.to_string());
                        }
                        all.push(new.clone());
                        changed.push(new);
                    } else if parent_exists {
                        new.status = Some(cm::MessageStatus::Completed);
                        new.error = None;
                        all.push(new.clone());
                        changed.push(new);
                    }
                    // 孤儿子节点：直接丢弃，不写入存储。
                }
            }
        }
        drop(c);

        if let Err(e) = chat_session.replace_messages(all).await {
            crate::plugin_error!("session", "persist_failure: replace_messages failed: {}", e);
        }

        // 6. 仅广播本次状态真正变化的消息（服务端权威终态，前端只信服务端）：
        //    - 根 Turn 的 Failed + error → 错误条 + 重试入口；
        //    - 子节点的 Completed → 结束前端流式动画（不再渲染 ⚠）。
        //    推送发生在 Error 事件之前（调用方先 persist_failure 再 broadcast_error_with_idle），
        //    Error 事件仅承担 transport 级兜底语义。
        //
        //    走 `emit_message_patch`（而非直接 `broadcast_frame`）：这几条终态同样是
        //    **消息补丁**，VDFS 列表必须同步收敛，否则一次失败之后 VDFS 视图会永远
        //    停在 streaming——`replace_messages` 已把终态写进存储，而列表读的是存储。
        //    这些消息都已在存储中（`existed = true`），且这里是全量替换终态，
        //    故一律 `updated`——`patch` 与 `view` 同一条（全量帧本身就是合并结果）。
        for m in changed {
            self.emit_message_patch(state, session_id, m.clone(), &m, true, None)
                .await;
        }

        // 7. 本轮在途缓冲随之作废：权威副本已由上面的 `replace_messages` 回到存储，
        //    继续叠加只会让同一条消息在 VDFS 列表里出现两次。
        collected.lock().await.clear();
    }
}

/// 计算 `root` 在 `messages` 中的传递闭包子树（含 `root` 自身）。
///
/// `persist_failure` 用它把「错误降级」的作用域收窄到失败 Turn 及其后代：
/// 历史轮次的消息不参与合并，避免 session 层拿流式快照去"补写"一份
/// Model 插件已落库的节点（同一 Turn 下出现两份同内容文本节点的根源）。
fn subtree_of(messages: &[cm::ChatMessage], root: &str) -> std::collections::HashSet<String> {
    let mut ids = std::collections::HashSet::new();
    ids.insert(root.to_string());
    let mut queue = vec![root.to_string()];
    while let Some(pid) = queue.pop() {
        for m in messages {
            if m.parent_id.as_deref() == Some(pid.as_str()) && ids.insert(m.id.clone()) {
                queue.push(m.id.clone());
            }
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::PluginChannel;

    /// 造一个已登记 `ai_control_tx` 的会话状态（模拟消费循环入口的登记）。
    async fn armed_state() -> Arc<ActiveSessionState> {
        let state = Arc::new(ActiveSessionState::with_session_id("s1".into()));
        let (host_chan, _ai_chan) = PluginChannel::pair(4);
        state.inner.write().await.ai_control_tx = Some(host_chan.tx.clone());
        state
    }

    async fn is_registered(state: &Arc<ActiveSessionState>) -> bool {
        state.inner.read().await.ai_control_tx.is_some()
    }

    /// 回归（watchdog 与 `stop_session` 竞态）：消费循环**提前 return** 时，
    /// 控制通道登记必须随之注销。
    ///
    /// 历史缺陷：业务 Error 帧与 1800s 消费超时两处出口写作 `return`，跳过了
    /// 循环之后的 `ai_control_tx = None` 清理，留下指向已关闭通道的陈旧 sender。
    /// `handle_abort` 以「登记是否为 `None`」作为子任务是否仍在运行的唯一判据，
    /// 判据从此永久为假 → abort 必然空等 3s 兜底，且 Abort 帧投进死通道被丢弃。
    #[tokio::test]
    async fn early_return_still_unregisters_ai_control_tx() {
        let state = armed_state().await;
        assert!(is_registered(&state).await, "前置：入口已登记控制通道");

        // 模拟提前出口：守卫存活期间函数直接 return（正常路径先显式 disarm）
        async fn early_return(mut guard: AiControlGuard) {
            guard.disarm().await;
        }
        early_return(AiControlGuard {
            state: state.clone(),
            armed: true,
        })
        .await;

        assert!(
            !is_registered(&state).await,
            "提前 return 后 ai_control_tx 应已注销，否则 handle_abort 会空等 3s 兜底"
        );
    }

    /// 回归（同上，panic / 裸 return 路径）：即使出口既没 `disarm` 也没走到
    /// 清理块，`Drop` 也必须兜住清理——这是"新增出口忘记清理"不再静默通过的保证。
    #[tokio::test]
    async fn guard_drop_unregisters_even_without_explicit_disarm() {
        let state = armed_state().await;
        {
            let _guard = AiControlGuard {
                state: state.clone(),
                armed: true,
            };
            // 故意不调用 disarm：离开作用域应由 Drop 清理
        }
        assert!(
            !is_registered(&state).await,
            "Drop 兜底失效：登记会永久残留，abort 判据再次退化为假"
        );
    }

    /// 幂等性：`disarm` 之后 Drop 不得再做二次清理——否则会误伤后续轮次
    /// 新登记的控制通道（消费循环与 resume 复用同一 `ActiveSessionState`）。
    #[tokio::test]
    async fn disarmed_guard_does_not_clobber_next_turn_registration() {
        let state = armed_state().await;
        let mut guard = AiControlGuard {
            state: state.clone(),
            armed: true,
        };
        guard.disarm().await;

        // 模拟下一轮：新的控制通道登记进来
        let (host_chan, _ai_chan) = PluginChannel::pair(4);
        state.inner.write().await.ai_control_tx = Some(host_chan.tx.clone());

        drop(guard); // 已 disarm 的旧守卫离开作用域

        assert!(
            is_registered(&state).await,
            "旧守卫的 Drop 误清了新一轮的登记：新一轮 abort 将失去中止能力"
        );
    }

    // ==================== 合并语义 ↔ 变更语义（不得漂移） ====================

    fn text_msg(id: &str, content: &str, status: cm::MessageStatus) -> cm::ChatMessage {
        cm::ChatMessage {
            id: id.into(),
            role: Some(cm::MessageRole::Assistant),
            msg_type: Some(cm::MessageType::Text),
            content: Some(cm::MessageContent::Text(content.into())),
            status: Some(status),
            ..Default::default()
        }
    }

    /// 流式文本：合并**回报**被追加的那段，VDFS 变更才能据此产出 `appended`。
    ///
    /// 这是「变更语义与合并语义不漂移」的结构保证——判据不是另算一遍，
    /// 而是合并函数自己说出来的。若有人日后把 `appended` 的判据改成独立实现，
    /// 这条测试仍会通过（它测的是回报值），所以另有一条测试直接断言两者一致
    /// （见 `plugin.rs::message_change_maps_patch_to_change_kind`）。
    #[test]
    fn merge_reports_appended_delta_for_streamed_text() {
        let mut existing = text_msg("m1", "你好", cm::MessageStatus::Streaming);
        let delta = merge_message_patch(
            &mut existing,
            &text_msg("m1", "，世界", cm::MessageStatus::Streaming),
        );
        assert_eq!(delta.as_deref(), Some("，世界"));
        assert_eq!(
            existing.content.as_ref().map(|c| c.to_text()),
            Some("你好，世界".to_string()),
            "回报的 delta 必须正是被拼进去的那一段"
        );

        // 首次写入同样是追加：节点此前无正文，尾部多出来的就是全部
        let mut empty = text_msg("m2", "", cm::MessageStatus::Streaming);
        empty.content = None;
        let delta = merge_message_patch(
            &mut empty,
            &text_msg("m2", "开头", cm::MessageStatus::Streaming),
        );
        assert_eq!(delta.as_deref(), Some("开头"));
    }

    /// 全量替换类补丁**不得**回报增量——否则消费者会按 delta 拼接，
    /// 把「每帧都是完整参数」的工具调用拼成垃圾。
    #[test]
    fn merge_reports_no_delta_for_full_replacement() {
        // ToolCall：每帧全量 JSON
        let mut tc = cm::ChatMessage {
            id: "t1".into(),
            role: Some(cm::MessageRole::Assistant),
            msg_type: Some(cm::MessageType::ToolCall),
            content: Some(cm::MessageContent::Text("{\"a\":".into())),
            ..Default::default()
        };
        let patch = cm::ChatMessage {
            id: "t1".into(),
            msg_type: Some(cm::MessageType::ToolCall),
            content: Some(cm::MessageContent::Text("{\"a\":1}".into())),
            ..Default::default()
        };
        assert!(merge_message_patch(&mut tc, &patch).is_none());
        assert_eq!(
            tc.content.as_ref().map(|c| c.to_text()),
            Some("{\"a\":1}".to_string()),
            "ToolCall 是全量替换，不是追加"
        );

        // role=Tool 的工具流式帧：全量替换（与前端 sessions.ts 同语义）
        let mut tool = text_msg("r1", "第一行\n", cm::MessageStatus::Streaming);
        tool.role = Some(cm::MessageRole::Tool);
        let patch = cm::ChatMessage {
            id: "r1".into(),
            role: Some(cm::MessageRole::Tool),
            content: Some(cm::MessageContent::Text("第一行\n第二行\n".into())),
            ..Default::default()
        };
        assert!(merge_message_patch(&mut tool, &patch).is_none());

        // 纯状态补丁（只带 id + status）：不触及内容
        let mut m = text_msg("m3", "正文", cm::MessageStatus::Streaming);
        let status_only = cm::ChatMessage {
            id: "m3".into(),
            status: Some(cm::MessageStatus::Completed),
            ..Default::default()
        };
        assert!(merge_message_patch(&mut m, &status_only).is_none());
        assert_eq!(m.status, Some(cm::MessageStatus::Completed));
        assert_eq!(
            m.content.as_ref().map(|c| c.to_text()),
            Some("正文".to_string())
        );
    }
}
