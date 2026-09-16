//! Session 插件实现
//!
//! 提供会话历史和上下文管理。
//!
//! 存储路径：`<本插件目录>/`（从 [`HomedirRegistry`] 直接派生）。
//! 不再使用 `storage_dir` 配置项，session 存储始终跟随系统目录。
//! 会话本身携带 `metadata.workdir` 用于 MODEL 工具调用上下文。
//!
//! ## 系统目录 (homedir)
//!
//! Session 存储目录由 [`HomedirRegistry::get()`] 派生：`<本插件目录>`。
//! 切换 homedir 后，新会话将写入新 homedir；存量数据**不会**自动迁移。
//!
//! ## 子模块分工（拆文件不拆行为）
//!
//! - [`nodes`]           VDFS 节点构造 / 路径模型 / 消息投影（纯函数，不持有 `self`）
//! - [`vdfs_provider`]   `impl VdfsProvider` 及其私有辅助
//!
//! 本文件保留：插件结构体与构造、内部能力（`impl SessionPlugin`）、协议路由与
//! 能力注册（`impl Plugin`）、配置定义，以及各子模块的**共享面重导出**。

use super::chat_session::ChatSession;
pub use super::config::SessionConfig;
use super::types::{Session, SessionSummary};
use crate::symbio_core::schemas::detail::{DetailDefinition, DetailField};
use crate::symbio_core::schemas::options::OPTIONS_LIST;
use crate::symbio_core::schemas::session::chat_message as cm;
use crate::symbio_core::schemas::session::session_chat_response;
use crate::symbio_core::vdfs;
use crate::symbio_core::vdfs_provider::{VDFS_PARAM_BEFORE, VDFS_PARAM_LIMIT};
use crate::symbio_core::{
    dir_from_ctx, ConfigFile, InvokeRequest, InvokeRequestExt, InvokeResponse, MemoryFile, Plugin,
    PluginDir, PluginError, PluginFrame, PluginMeta, PluginPayload, PLUGIN_FILE, PLUGIN_SESSION,
    SESSION_ID,
};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Weak};
use tokio::sync::{OnceCell, RwLock};

/// Session 插件
pub struct SessionPlugin {
    pub(crate) config: Arc<RwLock<SessionConfig>>,
    /// 配置文件的呈现与校验（`.vdfs/session/PLUGIN.yml`）——落盘写自己目录里的文件
    pub(crate) config_file: ConfigFile,
    /// 父插件引用（用于获取工具列表等）
    pub(crate) parent: Option<Weak<dyn Plugin>>,
    /// 活跃会话管理器 (V2 整合版：处理长连接与广播)
    pub(crate) active_mgr: Arc<super::active::ActiveSessionManager>,
    /// 存储单例（全局共享，初始化一次后跨 workdir / 跨会话复用）
    pub(crate) store: OnceCell<Arc<SessionStore>>,
    /// 心跳任务运行时状态：会话 id -> 最近一次"有效活动"时间戳（毫秒）。
    /// 调度器据此判断会话是否已空闲足够久。
    pub(crate) heartbeat_state: Arc<RwLock<HashMap<String, i64>>>,
    /// 工作目录监听管理器（目录树场景的实时数据变更通知）
    pub(crate) workdir_watches: super::workdir::WorkdirWatchManager,
    /// VDFS 实时：变更订阅表（引用计数 + 恰好一次投递）。
    ///
    /// provider 是**变更源的持有者**：会话的任何写入 / 删除都经这张表同步投给
    /// 当前订阅者，[`vdfs::VdfsProvider::watch`] 只往表里登记——变更因此无需
    /// 轮询即可到达 VDFS 事件总线，且**重叠订阅不会重复投递**（见该类型的文档）。
    ///
    /// 用 `Arc` 而非内联值：工作目录监听器（后台任务）也要投递进同一批订阅者，
    /// 需要共享所有权。
    pub(crate) change_subs: Arc<vdfs::ChangeSubscriptions>,
}

use super::store::SessionStore;

impl SessionPlugin {
    /// 主构造函数（Factory 机制使用）
    pub fn new(parent: Option<Weak<dyn Plugin>>, config: SessionConfig, dir: PluginDir) -> Self {
        // 变更订阅表：provider 自持一份，工作目录监听器共享同一份（见下方注入）
        let change_subs = Arc::new(vdfs::ChangeSubscriptions::default());
        // 目录树场景同时服务 VDFS：文件变化经**同一张订阅表**转发给 `.vdfs`
        // 订阅方，VDFS 侧不必另开一套监听（实时链路在机制层合流）。
        let workdir_watches = super::workdir::WorkdirWatchManager::default();
        workdir_watches.set_vdfs_subs(change_subs.clone());
        Self {
            config: Arc::new(RwLock::new(config)),
            config_file: ConfigFile::new(dir, "会话设置", config_definition()),
            parent,
            active_mgr: Arc::new(super::active::ActiveSessionManager::new()),
            store: OnceCell::new(),
            heartbeat_state: Arc::new(RwLock::new(HashMap::new())),
            workdir_watches,
            change_subs,
        }
    }

    /// 广播一次会话变更（VDFS 实时链路的数据源）。
    ///
    /// 无订阅者时直接返回；`path` 是 provider 子树内的相对路径（= 会话 id）。
    pub(crate) fn notify_change(&self, id: &str, change: &str) {
        self.change_subs.notify(&vdfs::VdfsChange::new(id, change));
    }

    // ==================== 消息级变更（不经前端补丁通道的那三条路由）====================
    //
    // `chat/clear_messages` / `chat/delete_message` / `chat/update_message` 是
    // **前端自己发起、自己已在本地收敛**的路由：补丁不需要再下发一遍，但 VDFS
    // 视图必须同步——否则 `.vdfs/session/<sid>/消息` 会停在旧内容上，而且没有任何
    // 机制会纠正它（两条链路互不校验）。
    //
    // 因此这三条路由各有一个「只发变更」的入口：与 `emit_message_patch` 的区别
    // 只在**不发前端帧**，发射规则（载荷宽度、地址拼法）完全同源。

    /// 删除单条消息 → `deleted`
    pub(crate) fn emit_message_deleted(&self, session_id: &str, mid: &str) {
        self.change_subs.notify(&vdfs::VdfsChange::new(
            message_path(session_id, mid),
            vdfs::VDFS_CHANGE_DELETED,
        ));
    }

    /// 清空整份转写 → 落在 `消息` 目录本身上的 `deleted`（消费者清空列表）
    pub(crate) fn emit_transcript_cleared(&self, session_id: &str) {
        self.change_subs.notify(&vdfs::VdfsChange::new(
            message_dir_path(session_id),
            vdfs::VDFS_CHANGE_DELETED,
        ));
    }

    /// 就地改写单条消息 → `updated`（带节点视图 + 内容快照）
    pub(crate) fn emit_message_updated(&self, session_id: &str, msg: &cm::ChatMessage) {
        self.change_subs.notify(&message_payload(
            vdfs::VdfsChange::new(message_path(session_id, &msg.id), vdfs::VDFS_CHANGE_UPDATED),
            session_id,
            msg,
        ));
    }

    /// 广播一条消息补丁到前端，并**同步发射**对应的 VDFS 变更。
    ///
    /// ## 为什么是一个函数而不是两处调用
    ///
    /// 消息补丁到达前端有两条入口：流式消费循环（模型 / 工具 / 子会话 / 审批
    /// 节点的逐帧补丁）与 `persist_failure`（错误或崩溃后由服务端定稿的终态）。
    /// 它们是两个真实时刻，无法也不该合并成一处调用；但「补丁到前端」与
    /// 「变更进 VDFS」必须**同生共死**——漏发一次变更，VDFS 列表就与前端
    /// 流式视图永久不一致（且没有任何机制会纠正它，因为两条链路互不校验）。
    ///
    /// 因此把两件事绑进同一个函数：入口有两个，出口只有一个。
    /// `existed` / `appended` 由实际合并结果给出，本函数不做二次判断。
    ///
    /// ## `patch` 与 `view` 的分工（不可合并成一个参数）
    ///
    /// - `patch`：**下发给前端的帧**，保持既有增量语义（前端按 `apply_patch`
    ///   合并）——换成合并后的完整消息会让前端把整条正文当增量再拼一遍；
    /// - `view`：**变更载荷的来源**，必须是合并后的完整消息，否则 `created` /
    ///   `updated` 附带的节点视图与内容快照是半成品，消费者拿到残缺结构。
    ///
    /// 两者对 `appended` 而言是同一份内容的不同表述（`patch` 是增量、`view`
    /// 是合并结果），因此不能用一个参数兼任。
    pub(crate) async fn emit_message_patch(
        &self,
        state: &Arc<super::active::ActiveSessionState>,
        session_id: &str,
        patch: cm::ChatMessage,
        view: &cm::ChatMessage,
        existed: bool,
        appended: Option<String>,
    ) {
        // 无订阅者时静默丢弃（订阅表语义），因此这条发射对不关心 VDFS 的
        // 调用方零成本。
        self.change_subs
            .notify(&message_change(session_id, view, existed, appended));
        self.broadcast_frame(
            state,
            PluginFrame::Data(json!(session_chat_response::StreamEvent::Update {
                message: patch,
            })),
        )
        .await;
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new("session", "会话管理")
            .with_description("提供会话历史和上下文管理")
            .with_version("0.3.0")
    }

    /// 静态工厂：从 InvokeRequest 构造 Plugin 实例
    pub fn build(ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Plugin> {
        // 自己的目录由容器经 `PLUGIN_DIR` 告知；配置就存在那里的 PLUGIN.yml
        // （反序列化使用 #[serde(default)]，自动忽略 storage_dir 等已废弃字段）
        let dir = dir_from_ctx(&*ctx, PLUGIN_SESSION);
        let config: SessionConfig = match dir.load::<SessionConfig>() {
            Ok(Some(c)) => c,
            Ok(None) => SessionConfig::default(),
            Err(e) => {
                crate::plugin_warn!("session", "读取自身配置失败，改用默认值：{e}");
                SessionConfig::default()
            }
        };

        let parent = ctx.parent();

        let plugin = Arc::new(SessionPlugin::new(parent, config, dir));

        // 启动心跳任务调度器（后台常驻）。仅在存在 Tokio runtime 时启动，
        // 避免单元测试（无 runtime）中 `tokio::spawn` 触发 panic。
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let scheduler = plugin.clone();
            handle.spawn(async move {
                scheduler.run_heartbeat_loop().await;
            });

            // 启动清理：把上次崩溃中断的 Streaming 消息标 Failed。
            // WaitingUserAction 节点不清理（合法的待恢复状态，重启后用户仍可 resume）。
            let cleaner = plugin.clone();
            handle.spawn(async move {
                cleaner.cleanup_crashed_sessions().await;
            });
        }

        plugin
    }

    /// 启动清理：扫描所有 session，把 `Streaming` 状态的消息标 `Failed`。
    ///
    /// 触发场景：后端崩溃/重启后，上次未完成的 chat_loop 留下了 Streaming 状态的消息。
    /// 这些消息需要收敛为 Failed 终态，使切回会话时能看到上次中断的错误。
    ///
    /// **不清理** `WaitingUserAction` 节点——它们是合法的待恢复状态（工具需审批/补充），
    /// 重启后用户仍可 resume。
    pub(crate) async fn cleanup_crashed_sessions(&self) {
        let sessions = match self.list_sessions().await {
            Ok(s) => s,
            Err(e) => {
                crate::plugin_warn!(
                    "session",
                    "cleanup_crashed_sessions: list_sessions 失败: {}",
                    e
                );
                return;
            }
        };

        for session in sessions {
            let chat_session = match self.open_chat_session(&session.id).await {
                Ok(cs) => cs,
                Err(_) => continue,
            };
            let mut msgs = match chat_session.get_messages().await {
                Ok(m) => m,
                Err(_) => continue,
            };

            let mut updates = Vec::new();
            for m in msgs.iter_mut() {
                if m.status == Some(cm::MessageStatus::Streaming) {
                    m.status = Some(cm::MessageStatus::Failed);
                    m.error = Some("会话因重启中断".to_string());
                    updates.push(m.clone());
                }
            }

            if !updates.is_empty() {
                crate::plugin_info!(
                    "session",
                    "cleanup_crashed_sessions: 会话 {} 清理 {} 条 Streaming 消息",
                    session.id,
                    updates.len()
                );
                if let Err(e) = chat_session.update_messages(updates).await {
                    crate::plugin_warn!(
                        "session",
                        "cleanup_crashed_sessions: update_messages 失败: {}",
                        e
                    );
                }
            }
        }
    }

    /// 获取父插件引用
    pub(crate) fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        self.parent.as_ref().and_then(|w| w.upgrade())
    }

    /// Session 存储目录：**本插件自己的目录**
    ///
    /// 根不在本插件里推导——来自构造时父插件经 `PLUGIN_DIR` 告知的 `PluginDir`。
    /// 插件不知道、也不该知道自己被放在哪。
    ///
    /// 按 id 派生路径的自由函数（`paths::session_dir` / `memory_path` /
    /// `resolve_archive_dir` / `save_transcript_archive`）一概由**调用方把根传进去**，
    /// 不在内部反查全局布局。
    ///
    /// 这是 session 存储目录的**唯一权威位置**，不依赖任何 config 字段。
    /// 切换 homedir 后 worker composite 会整体重建（`home/reload`），
    /// 新插件实例的下一次 `get_store` 因此用新 homedir 下的目录。
    pub fn storage_dir(&self) -> PathBuf {
        self.config_file.dir().dir().to_path_buf()
    }

    /// **仅测试用**的回退：没有插件实例时（自由函数 / 单测）拿不到自己的目录，
    /// 只能按插件名取常规落位。生产路径一律走 [`Self::storage_dir`]。
    #[cfg(test)]
    pub fn session_storage_dir() -> PathBuf {
        crate::providers::vdfs_service::entry::category_dir(PLUGIN_SESSION)
    }

    /// 获取（或初始化）存储后端。全局单例，首次调用时创建。
    pub(crate) async fn get_store(&self) -> Result<Arc<SessionStore>, PluginError> {
        if let Some(store) = self.store.get() {
            return Ok(Arc::clone(store));
        }

        let store = Arc::new(SessionStore::new(self.storage_dir()));

        // 一次性迁移：旧布局（消息内联）→ 元数据 / 消息两个文件，并补写清单投影。
        // 放在 store 构造之后、首次发布之前——每个进程只跑一次；读路径本来就有
        // 兜底，迁移失败也不影响可用性。
        if let Err(e) = store.migrate_split_messages().await {
            crate::plugin_warn!("session", "会话存储迁移失败（读路径有兜底）: {}", e);
        }

        // OnceCell::set 在多 writer 竞争时可能失败，但失败时另一线程已成功，直接拿
        let _ = self.store.set(Arc::clone(&store));
        Ok(self
            .store
            .get()
            .cloned()
            .unwrap_or_else(|| Arc::clone(&store)))
    }

    pub(crate) async fn get_or_create_session(
        &self,
        session_id: &str,
    ) -> Result<Session, PluginError> {
        self.get_store().await?.load_session(session_id).await
    }

    pub(crate) async fn save_session(&self, session: &Session) -> Result<(), PluginError> {
        self.get_store().await?.save_session(session).await
    }

    /// 会话清单（**摘要**，不含消息）
    pub(crate) async fn list_sessions(&self) -> Result<Vec<SessionSummary>, PluginError> {
        self.get_store().await?.list_sessions().await
    }

    /// 有界会话清单：`limit` 条、游标 `before` 之后（VDFS 调用级参数袋传入）
    pub(crate) async fn list_sessions_window(
        &self,
        limit: Option<u32>,
        before: Option<&str>,
    ) -> Result<Vec<SessionSummary>, PluginError> {
        self.get_store()
            .await?
            .list_sessions_window(limit, before)
            .await
    }

    pub(crate) async fn open_chat_session(
        &self,
        session_id: &str,
    ) -> Result<Arc<dyn ChatSession>, PluginError> {
        let store = self.get_store().await?;
        Ok(Arc::new(super::chat_session::PersistentChatSession::new(
            session_id.to_string(),
            self.config.clone(),
            store,
        )))
    }

    // ==================== 会话记忆（`.vdfs/session/<id>/AGENTS.md`）====================
    //
    // 机制在 `symbio_core::memory`（三层记忆同一份实现），本插件只有「个性」：
    // 落位在会话目录、地址挂在会话节点下、两道闸门取自 [`SessionConfig`]。
    // 归属原则是「谁能读写它，谁负责注入它」——工作区记忆归 work，本层只认会话。

    /// 由会话 id + 生效配置构造记忆门面。
    ///
    /// **作用域闸门**（id 缺失 / 空 → 无作用域）与**两道容量闸门**都在这一处注入，
    /// 因此调用点不必各自判断。
    fn store_with(
        root: &std::path::Path,
        session_id: Option<&str>,
        cfg: &SessionConfig,
    ) -> MemoryFile {
        super::memory::store(
            root,
            session_id,
            cfg.effective_memory_max_bytes(),
            cfg.effective_memory_inject_bytes(),
        )
    }

    /// 依会话 id 构造记忆门面。
    ///
    /// 会话 id 有两个来源——收集期来自 `ctx[SESSION_ID]`，VDFS 侧来自**路径**——
    /// 两者都归到这一个构造点，作用域闸门与两道容量闸门因此只写一遍。
    pub(crate) async fn memory_store(&self, session_id: &str) -> MemoryFile {
        let cfg = self.config.read().await;
        let root = self.storage_dir();
        Self::store_with(&root, Some(session_id), &cfg)
    }

    /// 参与能力收集：交出**会话记忆**（系统提示词片段）。
    ///
    /// 无会话 id 时静默跳过——没有会话就没有「本会话的记忆」，这不是故障，
    /// 不该往收集期错误桶里塞东西。
    async fn contribute_memory(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
        visitor: &Arc<dyn crate::symbio_core::CapabilityVisitor>,
    ) {
        let sid = ctx.get(SESSION_ID).unwrap_or_default();
        let store = self.memory_store(&sid).await;
        if !store.has_scope() {
            return;
        }
        let address = super::memory::memory_address(&sid);
        match store.segment(&super::memory::segment_spec(&address)) {
            Ok(Some(segment)) => {
                visitor
                    .register_system_prompt(super::memory::SEGMENT_NAME, segment)
                    .await;
            }
            Ok(None) => {}
            Err(e) => crate::plugin_warn!("session", "读取会话记忆失败，本轮不注入：{e}"),
        }
    }
}

#[async_trait]
impl Plugin for SessionPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let path = ctx.get(crate::symbio_core::PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);

        // 会话存储已迁移到 <本插件目录>/ 全局目录，session/* 系列接口
        // 不再依赖 ctx.workdir；ctx.workdir 仅在 chat 路径和需要 Model 路由时使用。
        let data = match path {
            "chat/send" => return self.handle_chat_send_oneoff(ctx).await,
            "chat/abort" => return self.handle_chat_abort_oneoff(ctx).await,
            "get_messages" => self.invoke_get_messages(ctx.clone()).await?,
            "append" => self.invoke_append(ctx.clone()).await?,
            "open" => return self.invoke_open(ctx.clone()).await,
            "clear" => self.invoke_clear(ctx.clone()).await?,
            "chat/clear_messages" => self.invoke_clear_messages(ctx.clone()).await?,
            "chat/delete_message" => self.invoke_delete_message(ctx.clone()).await?,
            "chat/update_message" => self.invoke_update_message(ctx.clone()).await?,
            "update" => self.invoke_update(ctx.clone()).await?,
            // 级联选项机制：会话是选项宿主，根选项列表在全项目收集后一次下发
            // （子层经 payload.parent 懒加载，与 vdfs/list 的 parent 懒加载同构）
            OPTIONS_LIST => return super::options::handle_list_options(self.as_ref(), ctx).await,
            "heartbeat/trigger" => return self.handle_heartbeat_trigger_oneoff(ctx).await,
            _ => return Err(PluginError::NotFound(format!("未知路径: {path}"))),
        };

        Ok(PluginPayload::new(&data))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        // 选项收集：会话作为选项宿主贡献「自有选项」（工作目录 / 风险等级 /
        // 运行模式 / 心跳）。与其它插件同构——命中 available_options 时才注册，
        // 经统一的 OPTION_VISITOR 收集器汇流。
        if ctx.get(crate::symbio_core::PATH).unwrap_or_default()
            == crate::symbio_core::TRAVERSE_AVAILABLE_OPTIONS
        {
            if let Some(visitor) = ctx.get(crate::symbio_core::OPTION_VISITOR) {
                visitor
                    .register_batch(self.build_option_nodes(&ctx).await)
                    .await;
            }
        }
        // 能力收集（available_tools）：session 贡献一个内聚工具——心跳设置。
        // 心跳机制（配置存储 / 后台调度 / 触发执行）全部在本插件内闭环，
        // 其设置工具同样内聚于此：直接读写本插件会话存储，零跨模块路由。
        if ctx.get(crate::symbio_core::PATH).unwrap_or_default()
            == crate::symbio_core::TRAVERSE_AVAILABLE_TOOLS
        {
            if let Some(visitor) = ctx.get(crate::symbio_core::CAPABILITY_VISITOR) {
                visitor
                    .register(Arc::new(super::heartbeat_tool::HeartbeatTool::new(
                        Arc::downgrade(&self),
                    )))
                    .await;
            }
        }
        // VDFS 挂载点：与会话工具共用同一次能力广播，把自己注册为一份 VDFS 资源。
        // 挂载名由使用方（此处即本插件）选定——约定用插件名（`PLUGIN_SESSION`），
        // 插件名在宿主内唯一，天然就是合格的挂载名；provider 自身不含此概念。
        if let Some(visitor) = ctx.get(crate::symbio_core::CAPABILITY_VISITOR) {
            let me: vdfs::DynVdfsProvider = self.clone();
            visitor.register_vdfs_provider(PLUGIN_SESSION, me).await;

            // 全局指令（`{homedir}/AGENTS.md`）：**只读、无地址、无容量**——它不是记忆
            // （模型改不动它），因此不进 `symbio_core::memory` 那套内核，由会话侧直接
            // 注入。走注册通道而非 `req.system_prompt`：后者会顶掉模型插件的人格。
            //
            // 工作区 `AGENTS.md` **不在这里**——那是 work 插件的工作区记忆，
            // 「谁能读写它，谁负责注入它」。
            if let Some(text) = super::prompt::global_instruction().await {
                visitor
                    .register_system_prompt(super::prompt::GLOBAL_PROMPT_NAME, text)
                    .await;
            }

            // 会话记忆（`.vdfs/session/<id>/AGENTS.md`）：**本会话私有**，可读写、有地址、
            // 有两道容量闸门——因此它归内核那套机制，本插件只负责「落位 + 标题 + 地址」。
            // 作用域闸门在 `contribute_memory` 内一处收口（无 `ctx[SESSION_ID]` 即不注入）。
            self.contribute_memory(&ctx, &visitor).await;
        }
        // 顺带声明「本插件有一份配置文档」（设置页据此列出并指路）
        crate::symbio_core::announce_configurable(&ctx, &self.config_file).await;
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

crate::submit_object_creator!(PLUGIN_SESSION, SessionPlugin::build, dyn Plugin);

// ==================== 配置文档（`.vdfs/session/PLUGIN.yml`） ====================

/// 会话配置的定义 —— **定义由配置的拥有者产出**。
///
/// 字段默认值一律从 [`SessionConfig::default()`] 读出，不写第二份字面量：
/// 历史上定义寄居在 setting 插件里，schema 与 serde 各写一份默认值，出现过
/// `max_tool_rounds` schema=15 而 serde=65535 的漂移（面板显示的默认值与
/// 实际行为不符）。不变式由 `config_definition_defaults_come_from_session_config`
/// 锁定。
fn config_definition() -> DetailDefinition {
    let d = SessionConfig::default();
    DetailDefinition::form(
        "会话设置",
        vec![
            DetailField::number(
                "max_messages",
                "最大消息数",
                "每个会话保存的最大消息数量",
                10.0,
                1000.0,
                json!(d.max_messages),
            ),
            DetailField::toggle(
                "auto_compress",
                "自动压缩",
                "上下文 Token 用量达到有效上限 70% 时自动压缩历史（LLM 语义快照）",
                d.auto_compress,
            ),
            DetailField::toggle(
                "enable_compact_tool",
                "工具压缩",
                "向模型提供主动压缩工具（context_compact）与 55% 水位提醒；关闭后仅保留自动压缩兜底",
                d.enable_compact_tool,
            ),
            DetailField::number(
                "context_messages",
                "上下文消息数量",
                "Model 对话时包含的上下文消息数量（0 表示不限制，6 表示 3 轮对话）",
                0.0,
                200.0,
                json!(d.context_messages),
            ),
            DetailField::number(
                "memory_max_bytes",
                "记忆写入上限（字节）",
                "会话记忆（.vdfs/session/<id>/AGENTS.md）单次写入的字节上限，超出会被拒绝",
                1.0,
                1_048_576.0,
                json!(d.memory_max_bytes),
            ),
            DetailField::number(
                "memory_inject_max_bytes",
                "记忆注入上限（字节）",
                "每轮注入系统提示词的会话记忆正文字节上限，超出部分截断",
                1.0,
                1_048_576.0,
                json!(d.memory_inject_max_bytes),
            ),
        ],
    )
}

// ==================== VDFS 挂载点（/session） ====================
//
// 会话是 VDFS 的第二个原生 provider：
//
// - **根 = 会话清单**：`.vdfs/session` 的目录内容即全部会话；根下可新建「会话」
//   （`new_types`），**新建语义完全由本 provider 自持**——id 由 provider 生成、
//   路径名作标题、经 `create` 写意图区分「新建」与「覆盖」；
// - **节点 = 单个会话**：`ext = session` → 前端聊天工作区渲染器（同一份详情实现
//   只服务 VDFS 机制）；
// - **实时**：会话的任何变更经 [`SessionPlugin::notify_change`] 广播 → `watch`
//   的转发任务 → VDFS 事件总线，前端列表无需轮询即可收敛。
//
// 读写删都转发既有会话能力（SessionStore + 会话 metadata 合并），不新造协议。

mod nodes;
mod vdfs_provider;

// 模块内共享面：`nodes` / `vdfs_provider` 经 `use super::*;` 取用，测试（`plugin.test.rs`）亦同。
// 未被本文件引用的项由编译器 `unused_imports` 兜底。
pub(crate) use self::nodes::{
    internal_dirs, message_change, message_dir_path, message_node, message_of, message_path,
    message_payload, message_text, ordered, overlay_live, parse_session_path, session_content,
    session_node, title_from_new_path, transcript_window, window_params, VdfsSessionPath,
    SEG_MESSAGES,
};

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
