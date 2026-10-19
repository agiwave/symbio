//! loop→session 的内部动作面：`session/<sid>/_run/<op>`（[ADR-086](../../../../../docs/010-adr/086.md "ADR-086") §4）。
//!
//! ## 为什么需要这条面
//!
//! 驱动器切成独立插件后，它执行一轮仍要读写**本会话**的转写、事实与压缩状态。
//! 那些状态的全部 owner 都是 session（转写唯一写入点、事实写者令牌、活跃名册、
//! 配置当前值），而 E-009 禁止 loop 直接 `use crate::plugins::session`。于是这一面
//! 把「驱动器需要的那几个动作」按地址暴露出来——loop 经 `parent.get_vfs_provider()`
//! 派发，session 在这里执行，**行为与搬家前逐字一致**。
//!
//! ## 授权：三条判据，缺一即 `Forbidden`
//!
//! 地址面对前端、LLM 工具、心跳全部可达。内部动作若只看地址，就等于给「改任意
//! 会话的写路径」开了第二个公开面。三条判据：
//!
//! 1. 宿主 ctx 上有 [`TURN_DISPATCH`]（`WIRE=false` 的进程内键，线上不可伪造）；
//! 2. `dispatch.spec.session_id == <sid>`（令牌只对**本轮会话**有效）；
//! 3. 动作名在 [`run_ops::ALL`] 闭集内，且与地址末段**字面相等**。
//!
//! ## 写动作自己发帧
//!
//! 落库与发帧是**一个**出口（`transcript::publish::append_and_publish`，见
//! `session/docs/022-vdfs-session-messages.md` §3.4）：动作在 session 侧调用它，
//! 把权威副本直接投进本会话转写；loop 拿回权威副本只为维护自己的内存镜像。
//! 不把「落库」与「发帧」分居两插件——那正是「落库的忘了发、发帧的拿错号」的
//! 温床。
//!
//! ## 会话句柄为什么优先取 `SESSION_HANDLE`
//!
//! 临时会话（`_t_`）的引擎持有**实例级**内存存储，按 id 重建会得到另一份空列表。
//! 编排入口在派发前已把本轮会话句柄挂进 `chat_ctx`，动作面原样复用；只有持久
//! 会话在缺句柄时才经 `Convergence` 重建（引擎无实例状态，重建等价）。
//! 「缺句柄 ⇒ 回退临时会话」的驱动器侧豁免已随本刀废除（ADR-082 裁决五）。

use super::super::chat_session::{
    Carrier, ChatSessionHandle, MessageChange, PersistentChatSession, SESSION_HANDLE,
};
use super::super::transcript::append_and_publish;
use super::super::transcript::compression::CompressionEmitter;
use super::super::transcript::supplements::SupplementDrain;
use super::*;
use crate::symbio_core::{
    llm_message_frame, llm_removed_frame, llm_state_frame, run_ops, vdfs_from_plugin_error,
    vdfs_host_ctx, AddNodesRequest, CompressionBeginReply, CompressionBeginRequest,
    CompressionFinishRequest, CompressionOutcome, ConsolidateRequest, EditFrame, EditNodesRequest,
    PluginInvokeRequestExt, RemoveNodesRequest, ReplaceAllRequest, TurnDispatch, VdfsActionResult,
    VdfsContext, VdfsError, VdfsRequest, VdfsResponse, VdfsResult, TURN_DISPATCH,
};
use serde::de::DeserializeOwned;
use tokio::runtime::RuntimeFlavor;

impl SessionPlugin {
    /// `_run` 分域的路由入口（`vdfs_provider::dispatch` 的唯一调用点）。
    pub(crate) async fn run_at(
        &self,
        ctx: &VdfsContext,
        path: &str,
        id: &str,
        op: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        // 只接受动作：`_run` 不是资源，读 / 写 / 列 / 删都没有语义。
        let VdfsRequest::Action { action, payload } = req else {
            return Err(VdfsError::invalid(format!(
                "内部动作段只接受 action：{path}"
            )));
        };
        // 地址末段与动作名必须相等：否则同一个地址可以借任意动作名绕过
        // 「哪个 op 落在哪一行」的可读性（地址与行为两套命名）。
        if action != op || !run_ops::ALL.contains(&op) {
            return Err(VdfsError::Forbidden(format!(
                "内部动作未授权（不在闭集内或与地址不符）：{path}"
            )));
        }
        // 令牌：宿主 ctx 上的本轮交付包。没有它 = 不是驱动器发起的调用。
        let host = vdfs_host_ctx(ctx)?;
        let Some(dispatch) = host.get(TURN_DISPATCH) else {
            return Err(VdfsError::Forbidden(format!(
                "内部动作只接受本轮驱动器调用（缺少 TURN_DISPATCH）：{path}"
            )));
        };
        if dispatch.spec.session_id != id {
            return Err(VdfsError::Forbidden(format!(
                "内部动作令牌不属于会话 {id}：{path}"
            )));
        }

        self.dispatch_run_op(&host, &dispatch, op, payload).await
    }

    async fn dispatch_run_op(
        &self,
        host: &Arc<dyn crate::symbio_core::PluginInvokeRequest>,
        dispatch: &TurnDispatch,
        op: &str,
        payload: Option<Value>,
    ) -> VdfsResult<VdfsResponse> {
        match op {
            run_ops::MESSAGES => {
                let session = self.run_session(host, dispatch).await?;
                let messages = session
                    .get_messages()
                    .await
                    .map_err(vdfs_from_plugin_error)?;
                action_data(op, format!("已读取 {} 条消息", messages.len()), messages)
            }
            run_ops::CONTEXT_MESSAGES => {
                let session = self.run_session(host, dispatch).await?;
                let messages = session
                    .get_context_messages(None)
                    .await
                    .map_err(vdfs_from_plugin_error)?;
                action_data(
                    op,
                    format!("已读取 {} 条上下文候选", messages.len()),
                    messages,
                )
            }
            run_ops::ADD_BY_RUNNER | run_ops::ADD_HERE => {
                let AddNodesRequest { messages } = parse_payload(payload)?;
                let session = self.run_session(host, dispatch).await?;
                let carrier = if op == run_ops::ADD_BY_RUNNER {
                    Carrier::Runner
                } else {
                    Carrier::Here
                };
                let persisted = append_and_publish(
                    &session,
                    messages,
                    carrier,
                    self.active_mgr.as_ref(),
                    session.session_id(),
                    &dispatch.spec.principal,
                )
                .await
                .map_err(vdfs_from_plugin_error)?;
                action_data(op, format!("已追加 {} 条消息", persisted.len()), persisted)
            }
            run_ops::EDIT_NODES => {
                let EditNodesRequest { nodes, frame } = parse_payload(payload)?;
                let session = self.run_session(host, dispatch).await?;
                let persisted = session
                    .apply(MessageChange::Edited(nodes))
                    .await
                    .map_err(vdfs_from_plugin_error)?;
                // 帧形由调用方指定（ADR-086 §4）：状态帧只更新身份 + 状态，
                // 完整消息帧替换整条正文。压成一个形状会让其中一处多发一次全文
                // 或让另一处前端停在旧状态。
                for message in &persisted {
                    let frame = match frame {
                        EditFrame::State => llm_state_frame(message),
                        EditFrame::Message => llm_message_frame(message),
                    };
                    self.active_mgr
                        .transcript_apply(session.session_id(), frame)
                        .await;
                }
                action_data(op, format!("已改写 {} 条消息", persisted.len()), persisted)
            }
            run_ops::REMOVE_NODES => {
                let RemoveNodesRequest { ids } = parse_payload(payload)?;
                let session = self.run_session(host, dispatch).await?;
                session
                    .apply(MessageChange::Removed(ids.clone()))
                    .await
                    .map_err(vdfs_from_plugin_error)?;
                for mid in &ids {
                    self.active_mgr
                        .transcript_apply(session.session_id(), llm_removed_frame(mid))
                        .await;
                }
                action_unit(op, format!("已删除 {} 条消息", ids.len()))
            }
            run_ops::REPLACE_ALL => {
                let ReplaceAllRequest {
                    nodes,
                    dropped,
                    head,
                } = parse_payload(payload)?;
                let session = self.run_session(host, dispatch).await?;
                // 失败口径与搬家前一致（`compress_snapshot_inner`）：缓存写失败
                // 不升级成整轮失败——压缩在内存里已经生效，报错等于用「记不下来」
                // 换「答不出来」；但也不吞，warn 一条。整表重写广播照发，否则
                // 前端永久停在压缩前的列表上。
                // list-write-fact-allow LWF-002: 地址面中继无锚可持——取代事实的寄存发生在逻辑调用方（loop 压缩现场 `context/pipeline.rs` 的 `context.consolidated = Some(...)`），本层只执行跨插件转发。
                if let Err(e) = session.replace_all(nodes).await {
                    crate::plugin_warn!(
                        "session",
                        "压缩快照缓存落库失败（内存已生效，重启后回退到压缩前）: {e}"
                    );
                }
                self.active_mgr
                    .emit_transcript_rewritten(session.session_id(), &dropped, &head)
                    .await;
                action_unit(op, format!("已整表重写（丢弃 {} 条）", dropped.len()))
            }
            run_ops::DRAIN_PARKED => {
                let session = self.run_session(host, dispatch).await?;
                self.drain_parked_of(&session);
                action_unit(op, "已补落寄存的变更".to_string())
            }
            run_ops::CONSOLIDATE => {
                let ConsolidateRequest { messages } = parse_payload(payload)?;
                let session = self.run_session(host, dispatch).await?;
                // 失败一律不致命（`consolidate_outside_turn` 的既有口径）：
                // 压缩已经发生，把它升级成整轮失败等于用「记不下来」换「答不出来」。
                super::super::orchestrator::consolidate::consolidate_outside_turn(
                    &session,
                    &messages,
                    &dispatch.spec.principal,
                );
                action_unit(op, "压缩事实已写入".to_string())
            }
            run_ops::TAKE_SUPPLEMENTS => {
                let session = self.run_session(host, dispatch).await?;
                let state = self.active_mgr.get_or_create(session.session_id()).await;
                let drain =
                    SupplementDrain::new(self.config.clone(), self.change_subs.clone(), state);
                // 取批内部用 `blocking_read`（同步孪生，见 `SupplementDrain` 的文档）：
                // 多线程 runtime 里必须 `block_in_place`，否则会阻塞工作线程。
                let batch = match tokio::runtime::Handle::try_current() {
                    Ok(h) if h.runtime_flavor() == RuntimeFlavor::MultiThread => {
                        tokio::task::block_in_place(|| drain.take_inbox_batch_sync())
                    }
                    _ => drain.take_inbox_batch_sync(),
                };
                action_data(op, "已取一批补充".to_string(), batch)
            }
            run_ops::WORKER_PROGRESS => {
                let session = self.run_session(host, dispatch).await?;
                let workers = super::super::worker_progress::for_session(&session)
                    .await
                    .map_err(vdfs_from_plugin_error)?;
                action_data(op, format!("{} 个子会话进展", workers.len()), workers)
            }
            run_ops::RECALL_SNAPSHOTS => {
                let session = self.run_session(host, dispatch).await?;
                let snapshots = session
                    .store_handle()
                    .recall_snapshots(session.session_id());
                action_data(
                    op,
                    format!("已召回 {} 份事实源", snapshots.len()),
                    snapshots,
                )
            }
            run_ops::ELIMINATION_REVOKED => {
                let session = self.run_session(host, dispatch).await?;
                // 存储布局与遍历留在 session，折叠判据仍在 core；loop 只收结论值
                // （ADR-086 §4：全量事件过墙是 P6「值注入」的反面）。
                let fleet = session.store_handle().deep_snapshots();
                let events: Vec<_> = fleet.into_iter().flatten().collect();
                let overlay = crate::symbio_core::GrantsOverlay::fold(
                    crate::symbio_core::EliminationDecider::verdict_grants_changes(&events),
                );
                let revoked = overlay.is_revoked(
                    &dispatch.spec.principal,
                    crate::symbio_core::authz::CAP_ASSIGN_WORK,
                );
                action_data(op, format!("assign.work 已收回：{revoked}"), revoked)
            }
            run_ops::COMPRESSION_BEGIN => {
                let CompressionBeginRequest { node_id, force } = parse_payload(payload)?;
                let session = self.run_session(host, dispatch).await?;
                let emitter = self.compression_emitter(session.session_id()).await;
                // 熔断判定随 begin 落在这里（ADR-086 §6）：计数器是会话级
                // 节奏状态，loop 只收「这次要不要跑」。`force`（用户手动压缩 /
                // 主动压缩工具）绕过熔断——用户已经表达了意图，不必再替他省。
                let skipped = !force && emitter.should_skip().await;
                if !skipped {
                    emitter.begin(&node_id).await;
                }
                action_data(
                    op,
                    format!("压缩占位节点{}", if skipped { "已跳过" } else { "已发布" }),
                    CompressionBeginReply { skipped },
                )
            }
            run_ops::COMPRESSION_FINISH => {
                let finish: CompressionFinishRequest = parse_payload(payload)?;
                let session = self.run_session(host, dispatch).await?;
                let emitter = self.compression_emitter(session.session_id()).await;
                // 熔断记账与定稿同一次动作（[ADR-086] §6）：只有 session 知道
                // 会话级计数器；loop 报结局，不自己留第二份。
                match finish.outcome {
                    CompressionOutcome::Success => emitter.record_success().await,
                    CompressionOutcome::Failure => emitter.record_failure(false).await,
                    CompressionOutcome::PermanentFailure => emitter.record_failure(true).await,
                    CompressionOutcome::None => {}
                }
                let node = emitter
                    .finish(
                        &finish.node_id,
                        finish.status,
                        &finish.text,
                        finish.failure_kind.as_deref(),
                        finish.stats,
                    )
                    .await;
                // 记录节点落库 + 回包：压缩是会话里真实发生的一步，必须留下记录
                // （否则刷新后只看到「历史突然变短」）。失败只 warn——前端已收到终态，
                // 与搬家前 `compress_with_snapshot_core` 同一口径。
                let persisted = append_and_publish(
                    &session,
                    vec![node.clone()],
                    Carrier::Here,
                    self.active_mgr.as_ref(),
                    session.session_id(),
                    &dispatch.spec.principal,
                )
                .await;
                match persisted {
                    Ok(mut nodes) if nodes.len() == 1 => {
                        action_data(op, "压缩节点已定稿".to_string(), nodes.remove(0))
                    }
                    Ok(_) => action_data(op, "压缩节点已定稿".to_string(), node),
                    Err(err) => {
                        crate::plugin_warn!(
                            "session",
                            "[Compress] 压缩节点落库失败（前端已收到终态）: {err}"
                        );
                        action_data(op, "压缩节点已定稿（落库失败）".to_string(), node)
                    }
                }
            }
            // 走到这里 = `op` 通过了 `run_ops::ALL` 的授权判据（第 1 步），但本函数
            // 没有对应的臂。**这是实现缺口，不是授权失败**，所以是 `Internal` 而非
            // `Forbidden`——把它报成「未授权」会让排查方向整个错掉（去看权限配置，
            // 而真正缺的是一个 `match` 臂）。
            //
            // ⚠️ 这里**没有编译期保护**：臂的模式是字符串常量，字符串不参与 `match`
            // 的穷尽性检查，新增 `run_ops` 常量而漏写臂照样编译通过。此前本行的注释
            // 写着「由编译器在这里报错」——那句话是错的，别信它。人工对账的办法：
            // `run_ops::ALL`（15 个）与下面的臂（14 个，`ADD_BY_RUNNER | ADD_HERE`
            // 合一臂）应当逐一对上；把这条对账做成可执行判据见
            // [T-172](../../../../../docs/009-task/172-内部动作段注册化.md "T-172") 的出口条件。
            _ => Err(VdfsError::internal(format!("内部动作未实现：{op}"))),
        }
    }

    /// 本轮会话引擎：优先复用编排入口交付的句柄（临时会话的唯一正确来源）。
    ///
    /// 临时会话的存储是**实例级**的（`PersistentChatSession::ephemeral` 里的
    /// `SessionStore::ephemeral()`）——按 id 重建会得到一份空列表，所有在第 1 步
    /// 之后落进内存的消息都会消失。因此缺句柄 = `Internal`，不是「再建一个」。
    async fn run_session(
        &self,
        host: &Arc<dyn crate::symbio_core::PluginInvokeRequest>,
        dispatch: &TurnDispatch,
    ) -> VdfsResult<Arc<PersistentChatSession>> {
        let id = dispatch.spec.session_id.as_str();
        if let Some(handle) = host.get(SESSION_HANDLE) {
            let ChatSessionHandle(session) = &*handle;
            if session.session_id() == id {
                return Ok(session.clone());
            }
        }
        if !dispatch.spec.persistent {
            return Err(VdfsError::internal(
                "临时会话缺少本轮会话句柄，内部动作无法执行".to_string(),
            ));
        }
        self.convergence
            .open_chat_session(id)
            .await
            .map_err(vdfs_from_plugin_error)
    }

    /// 压缩节点发射器：名册上的活跃状态 + 本插件持有的名册 / 配置当前值。
    async fn compression_emitter(&self, session_id: &str) -> CompressionEmitter {
        let state = self.active_mgr.get_or_create(session_id).await;
        CompressionEmitter::new(state)
    }

    /// 补落寄存的变更（持写者令牌的一方执行，见 `message_events::drain_parked`）。
    ///
    /// 无事实源（临时会话）⇒ 无事可落；事实源打不开 ⇒ 出声（与
    /// `consolidate_outside_turn` 的两种处置同口径）。
    fn drain_parked_of(&self, session: &PersistentChatSession) {
        match session.facts() {
            Ok(Some(store)) => {
                super::super::message_events::drain_parked(&store, session.session_id())
            }
            Ok(None) => {}
            Err(why) => crate::plugin_warn!(
                "session",
                "[Session] 寄存变更补落：事实源打不开（该次变更仍寄存）: {why}"
            ),
        }
    }
}

fn parse_payload<T: DeserializeOwned>(payload: Option<Value>) -> VdfsResult<T> {
    let value = payload.ok_or_else(|| VdfsError::invalid("内部动作缺少载荷"))?;
    serde_json::from_value(value)
        .map_err(|e| VdfsError::invalid(format!("内部动作载荷不合法：{e}")))
}

fn action_unit(op: &str, message: String) -> VdfsResult<VdfsResponse> {
    Ok(VdfsResponse::Action(VdfsActionResult {
        action: op.to_string(),
        ok: true,
        message,
        data: None,
    }))
}

fn action_data<T: serde::Serialize>(
    op: &str,
    message: String,
    value: T,
) -> VdfsResult<VdfsResponse> {
    let data = serde_json::to_value(value)
        .map_err(|e| VdfsError::internal(format!("内部动作结果序列化失败：{e}")))?;
    Ok(VdfsResponse::Action(VdfsActionResult {
        action: op.to_string(),
        ok: true,
        message,
        data: Some(data),
    }))
}

#[cfg(test)]
#[path = "run_actions.test.rs"]
mod tests;
