//! loop→session 的内部动作面：`session/<sid>/_run/<op>`（[D-086](../../../../../docs/003-design/014-单环一表.md#d-086-loopsession-切片契约turndispatch-进程内交付turnspec-纯值面内部动作走地址面 "DE-014-单环一表") §4；
//! 行表化见 [D-094](../../../../../docs/003-design/014-单环一表.md#d-094-内部动作段注册化行表即闭集run_opsall-删除sessionport-收成一个-call "DE-014-单环一表")）。
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
//! 3. 动作名在下方的行表 [`RUN_OPS`] 闭集内，且与地址末段**字面相等**。
//!
//! ## 行表即闭集（D-094）
//!
//! 闭集不是另一份手写清单，而是**行表自身**：授权判据第 3 条读它，派发也读它。
//! 「在表内」与「有臂」因此是同一件事——清单与实现分居两地要靠人工对账，
//! 而漏一份是静默的授权缺口（字符串不参与 `match` 穷尽性检查，编译不拦）。
//! 加一个动作只改两处：**一行 + 一个处理函数**。
//!
//! ## 写动作自己发帧
//!
//! 落库与发帧是**一个**出口（`transcript::publish::append_and_publish`，见
//! `../docs/109-vdfs-session-messages.md` §3.4）：动作在 session 侧调用它，
//! 把权威副本直接投进本会话转写；loop 拿回权威副本只为维护自己的内存镜像。
//! 不把「落库」与「发帧」分居两插件——那正是「落库的忘了发、发帧的拿错号」的
//! 温床。
//!
//! ## 会话句柄为什么优先取 `SESSION_HANDLE`
//!
//! 临时会话（`_t_`）的引擎持有**实例级**内存存储，按 id 重建会得到另一份空列表。
//! 编排入口在派发前已把本轮会话句柄挂进 `chat_ctx`，动作面原样复用；只有持久
//! 会话在缺句柄时才经 `Convergence` 重建（引擎无实例状态，重建等价）。
//! 「缺句柄 ⇒ 回退临时会话」的驱动器侧豁免已随本刀废除（D-082 裁决五）。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

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
    PluginInvokeRequest, RemoveNodesRequest, ReplaceAllRequest, TurnDispatch, VdfsActionResult,
    VdfsContext, VdfsError, VdfsRequest, VdfsResponse, VdfsResult, TURN_DISPATCH,
};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::runtime::RuntimeFlavor;

/// 内部动作的处理函数：收 session 插件（状态的所有者）、宿主 ctx、本轮交付包、
/// **行上登记的 op 名**与动作载荷，回动作响应。
///
/// 签名统一成这一个形状是行表的前提：三个动作的入参与返回本来就不同
/// （`messages` / `consolidate` / `compression-begin` 各长各的），用泛型或
/// trait object 统一都会撞 CS-001——而行表面向的是**闭集**（装配期冻结、
/// 运行期不增不减），fn 指针就够，不需要第三种分发机制。
///
/// `op` 取行上的 `&'static str` 而不是线上传来的那个 `&str`：回执的 `action`
/// 字段回登记名，不回路过的字符串（两者由行表查找的相等性保证一致）。
type RunOpFn = for<'a> fn(
    &'a SessionPlugin,
    &'a Arc<dyn PluginInvokeRequest>,
    &'a TurnDispatch,
    &'static str,
    Option<Value>,
) -> Pin<Box<dyn Future<Output = VdfsResult<VdfsResponse>> + Send + 'a>>;

/// 行表的一行：op 名（[`run_ops`] 词表常量）与它的处理函数。
///
/// **一行一 fn**：「在表内」与「有臂」由构造保证相等，这是本面唯一的
/// 不变量，也是它敢于删掉 core 手写 `ALL` 的原因（[D-094](../../../../../docs/003-design/014-单环一表.md#d-094-内部动作段注册化行表即闭集run_opsall-删除sessionport-收成一个-call "DE-014-单环一表")）。
struct RunOp {
    op: &'static str,
    run: RunOpFn,
}

/// loop→session 内部动作的**行表**：op 与处理函数一一对应，装配期冻结。
///
/// 闭集就是本表自身（授权判据第 3 条与派发读同一张表）。`add-by-runner` /
/// `add-here` 是两个 op 一个行为、差异在数据（`Carrier`）——按两行登记，
/// 处理函数里没有 `op ==` 比较。
static RUN_OPS: &[RunOp] = &[
    RunOp {
        op: run_ops::MESSAGES,
        run: |p, h, d, o, v| Box::pin(run_messages(p, h, d, o, v)),
    },
    RunOp {
        op: run_ops::CONTEXT_MESSAGES,
        run: |p, h, d, o, v| Box::pin(run_context_messages(p, h, d, o, v)),
    },
    RunOp {
        op: run_ops::ADD_BY_RUNNER,
        run: |p, h, d, o, v| Box::pin(run_add(p, h, d, o, v, Carrier::Runner)),
    },
    RunOp {
        op: run_ops::ADD_HERE,
        run: |p, h, d, o, v| Box::pin(run_add(p, h, d, o, v, Carrier::Here)),
    },
    RunOp {
        op: run_ops::EDIT_NODES,
        run: |p, h, d, o, v| Box::pin(run_edit_nodes(p, h, d, o, v)),
    },
    RunOp {
        op: run_ops::REMOVE_NODES,
        run: |p, h, d, o, v| Box::pin(run_remove_nodes(p, h, d, o, v)),
    },
    RunOp {
        op: run_ops::REPLACE_ALL,
        run: |p, h, d, o, v| Box::pin(run_replace_all(p, h, d, o, v)),
    },
    RunOp {
        op: run_ops::DRAIN_PARKED,
        run: |p, h, d, o, v| Box::pin(run_drain_parked(p, h, d, o, v)),
    },
    RunOp {
        op: run_ops::CONSOLIDATE,
        run: |p, h, d, o, v| Box::pin(run_consolidate(p, h, d, o, v)),
    },
    RunOp {
        op: run_ops::TAKE_SUPPLEMENTS,
        run: |p, h, d, o, v| Box::pin(run_take_supplements(p, h, d, o, v)),
    },
    RunOp {
        op: run_ops::WORKER_PROGRESS,
        run: |p, h, d, o, v| Box::pin(run_worker_progress(p, h, d, o, v)),
    },
    RunOp {
        op: run_ops::RECALL_SNAPSHOTS,
        run: |p, h, d, o, v| Box::pin(run_recall_snapshots(p, h, d, o, v)),
    },
    RunOp {
        op: run_ops::ELIMINATION_REVOKED,
        run: |p, h, d, o, v| Box::pin(run_elimination_revoked(p, h, d, o, v)),
    },
    RunOp {
        op: run_ops::COMPRESSION_BEGIN,
        run: |p, h, d, o, v| Box::pin(run_compression_begin(p, h, d, o, v)),
    },
    RunOp {
        op: run_ops::COMPRESSION_FINISH,
        run: |p, h, d, o, v| Box::pin(run_compression_finish(p, h, d, o, v)),
    },
];

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
        if action != op {
            return Err(VdfsError::Forbidden(format!(
                "内部动作未授权（动作名与地址末段不符）：{path}"
            )));
        }
        // 闭集 = 行表自身（授权判据第 3 条）：不在表内 ⇒ Forbidden。
        // 「在表内」与「有臂」是同一件事——行就是 op 与处理函数的配对。
        let Some(row) = RUN_OPS.iter().find(|r| r.op == op) else {
            return Err(VdfsError::Forbidden(format!(
                "内部动作未授权（不在闭集内）：{path}"
            )));
        };
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

        (row.run)(self, &host, &dispatch, row.op, payload).await
    }

    /// 本轮会话引擎：优先复用编排入口交付的句柄（临时会话的唯一正确来源）。
    ///
    /// 临时会话的存储是**实例级**的（`PersistentChatSession::ephemeral` 里的
    /// `SessionStore::ephemeral()`）——按 id 重建会得到一份空列表，所有在第 1 步
    /// 之后落进内存的消息都会消失。因此缺句柄 = `Internal`，不是「再建一个」。
    async fn run_session(
        &self,
        host: &Arc<dyn PluginInvokeRequest>,
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

// ── 处理函数（一行一个；载荷解析各自进行，解析失败 = 调用方拼错了）──────

async fn run_messages(
    plugin: &SessionPlugin,
    host: &Arc<dyn PluginInvokeRequest>,
    dispatch: &TurnDispatch,
    op: &'static str,
    _payload: Option<Value>,
) -> VdfsResult<VdfsResponse> {
    let session = plugin.run_session(host, dispatch).await?;
    let messages = session
        .get_messages()
        .await
        .map_err(vdfs_from_plugin_error)?;
    action_data(op, format!("已读取 {} 条消息", messages.len()), messages)
}

async fn run_context_messages(
    plugin: &SessionPlugin,
    host: &Arc<dyn PluginInvokeRequest>,
    dispatch: &TurnDispatch,
    op: &'static str,
    _payload: Option<Value>,
) -> VdfsResult<VdfsResponse> {
    let session = plugin.run_session(host, dispatch).await?;
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

async fn run_add(
    plugin: &SessionPlugin,
    host: &Arc<dyn PluginInvokeRequest>,
    dispatch: &TurnDispatch,
    op: &'static str,
    payload: Option<Value>,
    carrier: Carrier,
) -> VdfsResult<VdfsResponse> {
    let AddNodesRequest { messages } = parse_payload(payload)?;
    let session = plugin.run_session(host, dispatch).await?;
    let persisted = append_and_publish(
        &session,
        messages,
        carrier,
        plugin.active_mgr.as_ref(),
        session.session_id(),
        &dispatch.spec.principal,
    )
    .await
    .map_err(vdfs_from_plugin_error)?;
    action_data(op, format!("已追加 {} 条消息", persisted.len()), persisted)
}

async fn run_edit_nodes(
    plugin: &SessionPlugin,
    host: &Arc<dyn PluginInvokeRequest>,
    dispatch: &TurnDispatch,
    op: &'static str,
    payload: Option<Value>,
) -> VdfsResult<VdfsResponse> {
    let EditNodesRequest { nodes, frame } = parse_payload(payload)?;
    let session = plugin.run_session(host, dispatch).await?;
    let persisted = session
        .apply(MessageChange::Edited(nodes))
        .await
        .map_err(vdfs_from_plugin_error)?;
    // 帧形由调用方指定（D-086 §4）：状态帧只更新身份 + 状态，
    // 完整消息帧替换整条正文。压成一个形状会让其中一处多发一次全文
    // 或让另一处前端停在旧状态。
    for message in &persisted {
        let frame = match frame {
            EditFrame::State => llm_state_frame(message),
            EditFrame::Message => llm_message_frame(message),
        };
        plugin
            .active_mgr
            .transcript_apply(session.session_id(), frame)
            .await;
    }
    action_data(op, format!("已改写 {} 条消息", persisted.len()), persisted)
}

async fn run_remove_nodes(
    plugin: &SessionPlugin,
    host: &Arc<dyn PluginInvokeRequest>,
    dispatch: &TurnDispatch,
    op: &'static str,
    payload: Option<Value>,
) -> VdfsResult<VdfsResponse> {
    let RemoveNodesRequest { ids } = parse_payload(payload)?;
    let session = plugin.run_session(host, dispatch).await?;
    session
        .apply(MessageChange::Removed(ids.clone()))
        .await
        .map_err(vdfs_from_plugin_error)?;
    for mid in &ids {
        plugin
            .active_mgr
            .transcript_apply(session.session_id(), llm_removed_frame(mid))
            .await;
    }
    action_unit(op, format!("已删除 {} 条消息", ids.len()))
}

async fn run_replace_all(
    plugin: &SessionPlugin,
    host: &Arc<dyn PluginInvokeRequest>,
    dispatch: &TurnDispatch,
    op: &'static str,
    payload: Option<Value>,
) -> VdfsResult<VdfsResponse> {
    let ReplaceAllRequest {
        nodes,
        dropped,
        head,
    } = parse_payload(payload)?;
    let session = plugin.run_session(host, dispatch).await?;
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
    plugin
        .active_mgr
        .emit_transcript_rewritten(session.session_id(), &dropped, &head)
        .await;
    action_unit(op, format!("已整表重写（丢弃 {} 条）", dropped.len()))
}

async fn run_drain_parked(
    plugin: &SessionPlugin,
    host: &Arc<dyn PluginInvokeRequest>,
    dispatch: &TurnDispatch,
    op: &'static str,
    _payload: Option<Value>,
) -> VdfsResult<VdfsResponse> {
    let session = plugin.run_session(host, dispatch).await?;
    plugin.drain_parked_of(&session);
    action_unit(op, "已补落寄存的变更".to_string())
}

async fn run_consolidate(
    plugin: &SessionPlugin,
    host: &Arc<dyn PluginInvokeRequest>,
    dispatch: &TurnDispatch,
    op: &'static str,
    payload: Option<Value>,
) -> VdfsResult<VdfsResponse> {
    let ConsolidateRequest { messages } = parse_payload(payload)?;
    let session = plugin.run_session(host, dispatch).await?;
    // 失败一律不致命（`consolidate_outside_turn` 的既有口径）：
    // 压缩已经发生，把它升级成整轮失败等于用「记不下来」换「答不出来」。
    super::super::orchestrator::consolidate::consolidate_outside_turn(
        &session,
        &messages,
        &dispatch.spec.principal,
    );
    action_unit(op, "压缩事实已写入".to_string())
}

async fn run_take_supplements(
    plugin: &SessionPlugin,
    host: &Arc<dyn PluginInvokeRequest>,
    dispatch: &TurnDispatch,
    op: &'static str,
    _payload: Option<Value>,
) -> VdfsResult<VdfsResponse> {
    let session = plugin.run_session(host, dispatch).await?;
    let state = plugin.active_mgr.get_or_create(session.session_id()).await;
    let drain = SupplementDrain::new(plugin.config.clone(), plugin.change_subs.clone(), state);
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

async fn run_worker_progress(
    plugin: &SessionPlugin,
    host: &Arc<dyn PluginInvokeRequest>,
    dispatch: &TurnDispatch,
    op: &'static str,
    _payload: Option<Value>,
) -> VdfsResult<VdfsResponse> {
    let session = plugin.run_session(host, dispatch).await?;
    let workers = super::super::worker_progress::for_session(&session)
        .await
        .map_err(vdfs_from_plugin_error)?;
    action_data(op, format!("{} 个子会话进展", workers.len()), workers)
}

async fn run_recall_snapshots(
    plugin: &SessionPlugin,
    host: &Arc<dyn PluginInvokeRequest>,
    dispatch: &TurnDispatch,
    op: &'static str,
    _payload: Option<Value>,
) -> VdfsResult<VdfsResponse> {
    let session = plugin.run_session(host, dispatch).await?;
    let snapshots = session
        .store_handle()
        .recall_snapshots(session.session_id());
    action_data(
        op,
        format!("已召回 {} 份事实源", snapshots.len()),
        snapshots,
    )
}

async fn run_elimination_revoked(
    plugin: &SessionPlugin,
    host: &Arc<dyn PluginInvokeRequest>,
    dispatch: &TurnDispatch,
    op: &'static str,
    _payload: Option<Value>,
) -> VdfsResult<VdfsResponse> {
    let session = plugin.run_session(host, dispatch).await?;
    // 存储布局与遍历留在 session，折叠判据仍在 core；loop 只收结论值
    // （D-086 §4：全量事件过墙是 P6「值注入」的反面）。
    let fleet = session.store_handle().deep_snapshots();
    let events: Vec<_> = fleet.into_iter().flatten().collect();
    let overlay = crate::symbio_core::GrantsOverlay::fold(
        crate::symbio_core::EliminationDecider::verdict_grants_changes(&events),
    );
    let revoked = overlay.is_revoked(
        &dispatch.spec.principal,
        crate::symbio_core::CAP_ASSIGN_WORK,
    );
    action_data(op, format!("assign.work 已收回：{revoked}"), revoked)
}

async fn run_compression_begin(
    plugin: &SessionPlugin,
    host: &Arc<dyn PluginInvokeRequest>,
    dispatch: &TurnDispatch,
    op: &'static str,
    payload: Option<Value>,
) -> VdfsResult<VdfsResponse> {
    let CompressionBeginRequest { node_id, force } = parse_payload(payload)?;
    let session = plugin.run_session(host, dispatch).await?;
    let emitter = plugin.compression_emitter(session.session_id()).await;
    // 熔断判定随 begin 落在这里（D-086 §6）：计数器是会话级
    // 节奏状态，loop 只收「这次要不要跑」。`force`（用户手动压缩 /
    // 主动压缩工具）绕过熔断——用户已经表达了意图，不必再替他省。
    let skipped = !force && emitter.should_skip().await;
    if !skipped {
        emitter
            .begin(
                &node_id,
                &crate::symbio_core::principal::Principal::from_legacy(&dispatch.spec.principal),
            )
            .await;
    }
    action_data(
        op,
        format!("压缩占位节点{}", if skipped { "已跳过" } else { "已发布" }),
        CompressionBeginReply { skipped },
    )
}

async fn run_compression_finish(
    plugin: &SessionPlugin,
    host: &Arc<dyn PluginInvokeRequest>,
    dispatch: &TurnDispatch,
    op: &'static str,
    payload: Option<Value>,
) -> VdfsResult<VdfsResponse> {
    let finish: CompressionFinishRequest = parse_payload(payload)?;
    let session = plugin.run_session(host, dispatch).await?;
    let emitter = plugin.compression_emitter(session.session_id()).await;
    // 熔断记账与定稿同一次动作（[D-086] §6）：只有 session 知道
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
            &crate::symbio_core::principal::Principal::from_legacy(&dispatch.spec.principal),
        )
        .await;
    // 记录节点落库 + 回包：压缩是会话里真实发生的一步，必须留下记录
    // （否则刷新后只看到「历史突然变短」）。失败只 warn——前端已收到终态，
    // 与搬家前 `compress_with_snapshot_core` 同一口径。
    let persisted = append_and_publish(
        &session,
        vec![node.clone()],
        Carrier::Here,
        plugin.active_mgr.as_ref(),
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
