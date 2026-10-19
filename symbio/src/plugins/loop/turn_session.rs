//! 驱动器侧的**会话窄句柄**：把一次轮次的交付包（[`TurnDispatch`]）收成
//! 驱动器实际会用的那几个读法。
//!
//! ## 为什么要有这一格
//!
//! 搬家前驱动器直接持 `Arc<PersistentChatSession>`（会话引擎）。切插件之后
//! 那份引擎留在 session 墙内，驱动器手里只有三样东西：纯值 [`TurnSpec`]、
//! 本会话事实轨（`Option<Arc<dyn Store>>`）、动作面端口（[`SessionPort`]）。
//! 本结构把这三样收成一个对象，**方法名与旧句柄对齐**——于是执行链上的调用点
//! 只需换类型、不必重写语义。
//!
//! ## 它不持有存储实现
//!
//! `facts` 的类型是 core 的 [`Store`] trait object（`TurnDispatch` 已构造好），
//! 不是 `EventWalStore`/`SessionStore`——loop 不命名会话存储实现（E-009 的
//! 同一纪律）。「读完整史」在这条 trait 上就是 `range(Seq::new(0))`。
//!
//! ## 写方法一律委托端口
//!
//! 转写写入的唯一落点是 session（转写唯一写入点 + 事实写者令牌都在墙内）。
//! 本结构上的 `add_by_runner` / `edit_nodes` / `remove_nodes` / `replace_all`
//! 都不自己落库，只把动作递过去——这正是「主动发起流程走统一端口/协议」的形态。

use std::path::Path;
use std::sync::Arc;

use crate::symbio_core::chat_node::ChatNode;
use crate::symbio_core::{
    CompressionBeginReply, CompressionFinishRequest, DrainedSupplement, EditFrame, Event,
    FleetSnapshot, PluginError, Scope, Seq, Store, TurnSettings, TurnSpec, WorkerProgress,
};

use super::session_port::SessionPort;

/// 驱动器侧的会话句柄（构造点唯一：loop 的 `provider.rs`）。
pub(crate) struct LoopSession {
    spec: TurnSpec,
    facts: Option<Arc<dyn Store<Event = Event>>>,
    port: Arc<dyn SessionPort>,
}

impl LoopSession {
    pub(crate) fn new(
        spec: TurnSpec,
        facts: Option<Arc<dyn Store<Event = Event>>>,
        port: Arc<dyn SessionPort>,
    ) -> Self {
        Self { spec, facts, port }
    }

    pub(crate) fn session_id(&self) -> &str {
        &self.spec.session_id
    }

    /// 作用域（子会话 ⇒ `Child(父主体)`，顶层 ⇒ `Root`；session 从元数据预计算）。
    pub(crate) fn scope(&self) -> Scope {
        self.spec.scope.clone()
    }

    pub(crate) fn context_window(&self) -> u64 {
        self.spec.context_window
    }

    pub(crate) fn is_persistent(&self) -> bool {
        self.spec.persistent
    }

    pub(crate) fn settings(&self) -> &TurnSettings {
        &self.spec.settings
    }

    pub(crate) fn skill_compile_enabled(&self) -> bool {
        self.spec.settings.skill_compile_enabled
    }

    pub(crate) fn skill_fast_path(&self) -> bool {
        self.spec.settings.skill_fast_path
    }

    /// 工具结果存档目录（session 在派发前按会话子目录算好）。
    pub(crate) fn tool_archive_dir(&self) -> &Path {
        &self.spec.tool_archive_dir
    }

    /// 压缩前历史转存目录（同上）。
    pub(crate) fn transcript_archive_dir(&self) -> &Path {
        &self.spec.transcript_archive_dir
    }

    /// 本会话事实轨（`None` = 临时会话：不落派生事实）。
    pub(crate) fn facts(&self) -> Option<Arc<dyn Store<Event = Event>>> {
        self.facts.clone()
    }

    /// 完整事实史（`facts = None` ⇒ 空）。
    ///
    /// 与「读会话消息」不是同一件事：这份是**事件网格**，派生事实与重建都读它。
    pub(crate) fn facts_snapshot(&self) -> Vec<Event> {
        self.facts
            .as_ref()
            .map(|s| s.range(Seq::new(0)))
            .unwrap_or_default()
    }

    pub(crate) async fn get_messages(&self) -> Result<Vec<ChatNode>, PluginError> {
        self.port.messages().await
    }

    /// 上下文候选（会话侧按轮数窗口与协议投影裁剪；窗口由 session 的配置决定）。
    pub(crate) async fn get_context_messages(&self) -> Result<Vec<ChatNode>, PluginError> {
        self.port.context_messages().await
    }

    pub(crate) async fn add_by_runner(
        &self,
        messages: Vec<ChatNode>,
    ) -> Result<Vec<ChatNode>, PluginError> {
        self.port.add_by_runner(messages).await
    }

    pub(crate) async fn edit_nodes(
        &self,
        nodes: Vec<ChatNode>,
        frame: EditFrame,
    ) -> Result<Vec<ChatNode>, PluginError> {
        self.port.edit_nodes(nodes, frame).await
    }

    pub(crate) async fn remove_nodes(&self, ids: Vec<String>) -> Result<(), PluginError> {
        self.port.remove_nodes(ids).await
    }

    pub(crate) async fn replace_all(
        &self,
        nodes: Vec<ChatNode>,
        dropped: Vec<String>,
        head: ChatNode,
    ) -> Result<(), PluginError> {
        self.port.replace_all(nodes, dropped, head).await
    }

    pub(crate) async fn drain_parked(&self) -> Result<(), PluginError> {
        self.port.drain_parked().await
    }

    pub(crate) async fn consolidate(&self, messages: Vec<ChatNode>) -> Result<(), PluginError> {
        self.port.consolidate(messages).await
    }

    pub(crate) async fn take_supplements(&self) -> Result<Option<DrainedSupplement>, PluginError> {
        self.port.take_supplements().await
    }

    pub(crate) async fn worker_progress(&self) -> Result<Vec<WorkerProgress>, PluginError> {
        self.port.worker_progress().await
    }

    pub(crate) async fn recall_snapshots(&self) -> Result<Vec<FleetSnapshot>, PluginError> {
        self.port.recall_snapshots().await
    }

    pub(crate) async fn elimination_revoked(&self) -> Result<bool, PluginError> {
        self.port.elimination_revoked().await
    }

    pub(crate) async fn compression_begin(
        &self,
        node_id: &str,
        force: bool,
    ) -> Result<CompressionBeginReply, PluginError> {
        self.port.compression_begin(node_id, force).await
    }

    pub(crate) async fn compression_finish(
        &self,
        req: CompressionFinishRequest,
    ) -> Result<ChatNode, PluginError> {
        self.port.compression_finish(req).await
    }
}
