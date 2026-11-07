//! Session 插件实现
//!
//! 提供会话历史和上下文管理。
//!
//! 存储路径：`<本插件目录>/`（装配期由父插件经 `PLUGIN_DIR` 告知）。
//! 不再使用 `storage_dir` 配置项，session 存储始终跟随本实例的作用域。
//! 会话本身携带 `metadata.workdir` 用于 MODEL 工具调用上下文。
//!
//! ## 作用域
//!
//! Session 存储目录就是**本插件自己的目录**：顶层时它是系统级 `<homedir>/session`，
//! 挂在子智能体下时是 `<子树>/session`。本插件不读、也不需要知道 homedir。
//!
//! ## 子模块分工（拆文件不拆行为）
//!
//! - [`nodes`]           VDFS 节点构造 / 路径模型 / 消息投影（纯函数，不持有 `self`）
//! - [`vdfs_provider`]   `impl VdfsProvider` 的路由、订阅表与跨域共用读值
//! - [`vdfs_catalog`]    目录域：根清单 / 会话条目 / 记忆 / 子会话 / 工作目录
//! - [`vdfs_messages`]   转写域：`<id>/message[/mid]` 的读、改、截断、清空
//! - [`vdfs_inbox`]      收件箱域：`<id>/inbox[/iid]`（运行中会话的在途事件）
//!
//! 会话地址面按**域**分文件（判据：那一段错了红的是哪一面）；「路径 → 域」的映射
//! 只在 `vdfs_provider::dispatch` 一处。
//!
//! 本文件保留：插件结构体与构造、内部能力（`impl SessionPlugin`）、协议路由与
//! 能力注册（`impl Plugin`）、配置定义，以及各子模块的**共享面重导出**。

use super::chat_session::PersistentChatSession;
pub use super::config::{PresentationTree, SessionConfig};
use super::types::{Session, SessionSummary};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock, Weak};
use symbio_kernel::capability::CapabilityRow;
use symbio_kernel::plugin::{
    Plugin, PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse,
    PluginMeta, PluginPayload, PluginStopReason,
};
use symbio_kernel::present::{DetailDefinition, DetailField, DetailOption, DetailSection};
use symbio_kernel::vdfs::{VdfsAccess, VdfsChange};
use symbio_kernel::vdfs::{VDFS_PARAM_BEFORE, VDFS_PARAM_LIMIT};
use symbio_plugin_api::fact_source::{FactSource, FACT_SOURCE};
use symbio_plugin_api::PLUGIN_ID_SESSION;
use symbio_providers::MemoryFile;
use symbio_sdk::chat::session_chat;
use symbio_sdk::keys::SESSION_ID;
use symbio_sdk::plugin::{plugin_dir_from_ctx, PluginConfigFile, PluginDir, PLUGIN_FILE};
use symbio_sdk::vdfs::host::VdfsChangeSubscriptions;
use tokio::sync::{Mutex, OnceCell, RwLock};

/// Session 插件
pub struct SessionPlugin {
    pub(crate) config: Arc<RwLock<SessionConfig>>,
    /// 授权清单的读-改-写串行锁：`session/grant` 是「读整份配置 → 抬一格 → 落回」，
    /// 两张审批卡同时点「永久放行」时后写的会丢掉先写的那一格。`apply` 仍是唯一
    /// 落盘路径，本锁只把两次读-改-写排成队（不是第二套写口）。
    pub(crate) grant_lock: Mutex<()>,
    /// 配置文件的呈现与校验（`<根>/session/PLUGIN.yml`）——落盘写自己目录里的文件
    pub(crate) config_file: PluginConfigFile,
    /// 父插件引用（用于获取工具列表等）
    pub(crate) parent: Option<Weak<dyn Plugin>>,
    /// 活跃会话管理器 (V2 整合版：处理长连接与广播)
    pub(crate) active_mgr: Arc<super::active::ActiveSessionManager>,
    /// 会话收敛手柄：存储格子 / 存储目录 / 配置 / 活跃名册——把运行态与终态落到一个
    /// 会话上所需的那四样。
    ///
    /// 存储的 `OnceCell` **住在手柄上**而不是插件上：后台任务里的两个调用方
    /// （`TranscriptSink` / `WorkingGuard`）拿它而不是整个插件
    /// （[D-075](../../../docs/003-design/002-架构总览.md#d-075-卸载能力先于目录分离session-切分立三条前置会话记忆与存储留守 "DE-002-架构总览") 前置 P2），而「卸载后任务还在收尾」
    /// 恰恰需要这份存储比插件活得更久。
    pub(crate) convergence: Convergence,
    /// 工作目录监听管理器（目录树场景的实时数据变更通知）
    pub(crate) workdir_watches: super::workdir::WorkdirWatchManager,
    /// VDFS 实时：变更订阅表（引用计数 + 恰好一次投递）。
    ///
    /// provider 是**变更源的持有者**：会话的任何写入 / 删除都经这张表同步投给
    /// 当前订阅者，[`symbio_kernel::vdfs::request::VdfsRequest::Watch`] 只往表里登记——变更因此无需
    /// 轮询即可到达 VDFS 事件总线，且**重叠订阅不会重复投递**（见该类型的文档）。
    ///
    /// 用 `Arc` 而非内联值：工作目录监听器（后台任务）也要投递进同一批订阅者，
    /// 需要共享所有权。
    pub(crate) change_subs: Arc<VdfsChangeSubscriptions>,
    /// 本部署根的驱动实例（按根构造一次；父插件即根）。
    ///
    /// 懒建：构造时父句柄可能还是弱引用，等第一次开轮再 upgrade；之后整份插件
    /// 共享同一实例（按会话的重入位因此只有一份）。
    drive: OnceLock<Option<Arc<dyn symbio_plugin_api::DriveEntry>>>,
    /// 指向自身的弱引用：轮收尾的后台任务（[`SessionPlugin::spawn_turn_closer`]）
    /// 要活得比写臂久，必须持有一份 `'static` 的插件句柄，而 `dispatch` 只有 `&self`。
    ///
    /// 装配期在 [`Self::build`] 里登记一次（`Weak` 不延长生命周期，不构成泄漏）；
    /// 直接 `new` 出来的实例（测试）没有它，收尾任务因而只投影、不抽干。
    pub(crate) self_ref: OnceLock<Weak<SessionPlugin>>,
}

use super::convergence::Convergence;
use super::store::SessionStore;

impl SessionPlugin {
    /// 主构造函数（Factory 机制使用）
    pub fn new(parent: Option<Weak<dyn Plugin>>, config: SessionConfig, dir: PluginDir) -> Self {
        // 变更订阅表：provider 自持一份，工作目录监听器共享同一份（见下方注入）
        let change_subs = Arc::new(VdfsChangeSubscriptions::default());
        // 目录树场景同时服务 VDFS：文件变化经**同一张订阅表**转发给 `<根>`
        // 订阅方，VDFS 侧不必另开一套监听（实时链路在机制层合流）。
        let workdir_watches = super::workdir::WorkdirWatchManager::default();
        workdir_watches.set_vdfs_subs(change_subs.clone());
        let config_file = PluginConfigFile::new(dir, "会话设置", config_definition());
        // 存储根 = 本插件目录（`storage_dir` 的唯一来源，见 [`Self::storage_dir`]）
        let store_dir = config_file.dir().dir().to_path_buf();
        // 事实源开面按这个目录解析进程内唯一的 `SessionStore`（装配层 open 用）。
        super::store::remember_storage_dir(&store_dir);
        let store: Arc<tokio::sync::OnceCell<Arc<SessionStore>>> = Arc::new(OnceCell::new());
        let active_mgr = Arc::new(super::active::ActiveSessionManager::new(
            change_subs.as_ref().clone(),
        ));
        let config = Arc::new(RwLock::new(config));
        let convergence = Convergence::new(store, store_dir, config.clone(), active_mgr.clone());
        Self {
            config,
            grant_lock: Mutex::new(()),
            config_file,
            parent,
            convergence,
            active_mgr,
            workdir_watches,
            change_subs,
            drive: OnceLock::new(),
            self_ref: OnceLock::new(),
        }
    }

    /// 广播一次会话变更（VDFS 实时链路的数据源）。
    ///
    /// 无订阅者时直接返回；`path` 是 provider 子树内的相对路径（= 会话 id）。
    ///
    /// ## 它承载会话叶子的**粗粒度**变更
    ///
    /// 会话叶子 `<id>` 上的变更分两半，出口不同但**通道相同**（都是本表、都是
    /// `vdfs/watch`），差别只在粒度：
    ///
    /// | 一半 | 出口 | 粒度 |
    /// |---|---|---|
    /// | 运行态（本轮 `working` / 终态 / 警告 / 结局） | `orchestrator::emit_session_state` → `Transcript::emit_session_state` | `data` = 全量节点视图（`session_node` 同源） |
    /// | 资源（创建 / 删除 / 改名 / 标题 / metadata） | 本函数 | 无载荷，消费端回读 / 重拉 |
    ///
    /// 资源信号**不带节点视图**：这类变更（改名 / metadata 写入）是粗粒度的，
    /// 消费方本来就按「重拉清单」处理；捎带快照只会让每个消费端都背一次
    /// 「逐字段合并」的成本。运行态走 `emit_session_state`，那里的视图是
    /// 「调用那一刻」从权威源构造的（与 `stat` 同源），不存在过期副本。
    ///
    /// ## 消费端怎么处理无载荷帧
    ///
    /// 「重拉清单」是**清单类**消费端（前端侧栏 / 会话 store）的做法；只关心
    /// **运行态**的消费端（CLI / 心跳守护）不需要回读——运行态变更恒带视图，
    /// 因此无载荷对它就等于「与本轮无关」，直接丢弃即可。判据是**用途**，不是
    /// 「无载荷」本身（CLI 侧原有一段"无载荷就回读 `stat` 分辨"的代码，实测
    /// 那笔请求永远改变不了结论，已删——见 `crates/symbio-cli/src/client.rs` 的说明）。
    pub(crate) fn notify_change(&self, id: &str) {
        self.change_subs.notify(&VdfsChange::bare(id));
    }

    // ==================== 转写发布（消息变更的唯一出口）====================
    //
    // 三个发布函数（一条 / 一批 / 整表重写）现在住在**活跃会话名册**上——
    // 按会话 id 解析到它的那份 `Transcript` 只需要那张表（见
    // [`super::active::ActiveSessionManager::transcript_apply`]）。
    //
    // 运行中的轮次另有唯一的常规写入者：消费循环（`orchestrator::consume`）把
    // 通道上的消息喂给同一个 `Transcript`——上游有多少个发射点都无所谓，
    // 到写入点只剩一个。
    //
    // 落库之后的**回包**（权威 `seq` 换入）另有唯一出口：`transcript::publish` 的
    // `append_and_publish`，三条落库路径（主循环增量 / 用户发言 / 压缩节点）都经它逐条下发权威副本。

    pub fn metadata() -> PluginMeta {
        PluginMeta::new("session", "会话")
            .with_description(
                "会话清单。每个会话是一份独立对话记录，可读 / 写 / 删；新建即创建一份新会话。",
            )
            .with_version("0.3.0")
            .with_order(1)
            .with_icon("session")
            // 会话是叶子文档：可列，不参与树遍历（会话内部的子结构另有容器语义）
            .with_root_access(VdfsAccess::LIST)
    }

    /// 静态工厂：从 PluginInvokeRequest 构造 Plugin 实例
    pub fn build(ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        // 自己的目录由容器经 `PLUGIN_DIR` 告知；配置就存在那里的 PLUGIN.yml
        // （反序列化使用 #[serde(default)]，自动忽略 storage_dir 等已废弃字段）
        let dir = plugin_dir_from_ctx(&*ctx, PLUGIN_ID_SESSION);
        let config: SessionConfig = match dir.load::<SessionConfig>() {
            Ok(Some(c)) => c,
            Ok(None) => SessionConfig::default(),
            Err(e) => {
                symbio_plugin_api::plugin_warn!("session", "读取自身配置失败，改用默认值：{e}");
                SessionConfig::default()
            }
        };

        let parent = ctx.parent();

        let plugin = Arc::new(SessionPlugin::new(parent, config, dir));
        // 收尾任务要 self 的 `'static` 句柄（见字段说明），装配期登记一次。
        let _ = plugin.self_ref.set(Arc::downgrade(&plugin));
        plugin
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

    /// 获取（或初始化）存储后端。全局单例，首次调用时创建。
    ///
    /// 实现在收敛手柄上（它与这里共用**同一个** `OnceCell`，见
    /// [`super::store::resolve_store`]）——插件这里只是门面，不是第二个初始化路径。
    pub(crate) async fn get_store(&self) -> Result<Arc<SessionStore>, PluginError> {
        self.convergence.get_store().await
    }

    /// 该会话的契约事实源：装配层下发 `FACT_SOURCE` 时**优先用同一句柄**，
    /// 否则走同一个登记开面（不是第二个 store）。
    pub(crate) async fn fact_source_for(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
        session_id: &str,
    ) -> Result<Arc<dyn FactSource>, PluginError> {
        if let Some(source) = ctx.get(FACT_SOURCE) {
            return Ok(source);
        }
        self.fact_source(session_id).await
    }

    /// 该会话的契约事实源：与装配层经开面打开的是同一底层 `WalStore`。
    ///
    /// 开轮写口必须拿到**本部署根**的驱动实例与真实转写出口：驱动句柄随事实源
    /// 一起绑好，`append` 落地后直接用它，不按会话 id 反查进程级入口。
    pub(crate) async fn fact_source(
        &self,
        session_id: &str,
    ) -> Result<Arc<dyn FactSource>, PluginError> {
        let sink = Arc::new(super::transcript::sink::TranscriptSink::new(
            Arc::clone(&self.active_mgr),
            session_id.to_string(),
        ));
        let store = self.get_store().await?;
        // 开面播种（D-340）：会话尚无事实时，把部署级已裁决的 `verdict.asserted`
        // 复制进本会话事实流。只在**这一处**开面调用，运行期不追发；跨会话索引
        // 不随 `DriveTarget` / `PortExecContext` / 执行体出现。
        store
            .seed_open_verdicts(session_id)
            .map_err(PluginError::InternalError)?;
        // 补充整合的批次边界折叠器：随**开轮写口**一起绑上（只读面不折叠）。
        // 它持的是「取批那三样」+ 消息列表写口，折叠全程留在会话域，不需要驱动器伸手来问。
        let state = self.active_mgr.get_or_create(session_id).await;
        // 中止信号格：**会话状态是唯一所有者**，事实源只拿同一个 `Arc` 的引用
        // （不是第二份 `Option`，否则动作写一边、驱动读另一边，中止永不生效）。
        let abort_signal = Arc::clone(&state.abort_signal);
        let supplements = super::transcript::supplements::SupplementDrain::new(
            Arc::clone(&self.config),
            Arc::clone(&self.change_subs),
            state,
            Arc::clone(&store),
        );
        store
            .fact_source_bound(
                session_id,
                self.drive_entry(),
                symbio_kernel::exec::ExecEventSink::direct(sink),
                Some(supplements),
                abort_signal,
            )
            .map_err(PluginError::InternalError)?
            .ok_or_else(|| PluginError::NotFound(format!("会话 {session_id} 没有事实源")))
    }

    /// 本实例所属**部署根**的驱动实例（父插件即根；按根构造一次，随插件留守）。
    ///
    /// 未装配驱动工厂（单测）或无父插件 ⇒ `None`：写口落地即止，不去猜入口。
    pub(crate) fn drive_entry(&self) -> Option<Arc<dyn symbio_plugin_api::DriveEntry>> {
        self.drive
            .get_or_init(|| {
                let root = self.parent.as_ref()?.upgrade()?;
                symbio_plugin_api::declared_drive_binding().map(|binding| binding.bind(root))
            })
            .clone()
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
    ) -> Result<Arc<PersistentChatSession>, PluginError> {
        self.convergence.open_chat_session(session_id).await
    }

    // ==================== 会话记忆（`<根>/session/<id>/MEMORY.md`）====================
    //
    // 机制在 `providers/memory`（三层记忆同一份实现），本插件只有「个性」：
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

    /// 本插件当前的**建树策略**（`SessionConfig::presentation_tree`）。
    ///
    /// 呈现读侧的投影取值点（`session_content` 的两个 Read 分支经它）。与写侧
    /// 请求级快照（`ChatOrchestrator::presentation_tree`）同一个 `SessionConfig`
    /// 来源、同一个解析（`presentation_tree()` 未知值回落 `grouped`）——锁不跨
    /// 呈现序列化，取一次值即放。
    pub(crate) async fn presentation_mode(&self) -> PresentationTree {
        self.config.read().await.presentation_tree()
    }

    /// 参与能力收集：交出**会话记忆**（系统提示词片段）。
    ///
    /// 无会话 id 时静默跳过——没有会话就没有「本会话的记忆」，这不是故障，
    /// 不该往收集期错误桶里塞东西。
    async fn contribute_memory(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
        visitor: &Arc<dyn symbio_kernel::capability::Registry>,
    ) {
        let sid = ctx.get(SESSION_ID).unwrap_or_default();
        let store = self.memory_store(&sid).await;
        if !store.has_scope() {
            return;
        }
        // 绝对地址 = 上下文父地址 + 相对地址（容器转发时已写入父地址）
        let address =
            symbio_sdk::vdfs::address::absolute_addr(ctx, &super::memory::memory_rel_path(&sid));
        match store.segment(&super::memory::segment_spec(&address)) {
            Ok(Some(segment)) => {
                visitor
                    .put(CapabilityRow::prompt(super::memory::SEGMENT_NAME, segment))
                    .await;
            }
            Ok(None) => {}
            Err(e) => {
                symbio_plugin_api::plugin_warn!("session", "读取会话记忆失败，本轮不注入：{e}")
            }
        }
    }
}

#[async_trait]
impl Plugin for SessionPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    fn get_vfs_provider(self: Arc<Self>) -> Option<Arc<dyn symbio_kernel::vdfs::VdfsProvider>> {
        Some(self)
    }

    /// 停机（[D-075](../../../docs/003-design/002-架构总览.md#d-075-卸载能力先于目录分离session-切分立三条前置会话记忆与存储留守 "DE-002-架构总览") 的前置 P1）：目录监听在这里交出资源。
    async fn stop(self: Arc<Self>, _reason: PluginStopReason) -> Result<(), PluginError> {
        // 目录监听：watcher 一 drop 事件流即断（OS 句柄随之内核回收）
        self.workdir_watches.close_all();
        Ok(())
    }

    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(symbio_sdk::keys::PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);

        // 会话存储已迁移到 <本插件目录>/ 全局目录，session/* 系列接口
        // 不再依赖 ctx.workdir；ctx.workdir 仅在 chat 路径和需要 Model 路由时使用。
        //
        // 每个分支都**自己返回**（`match` 的值不再被使用）：本表已经没有
        // 「算出一份 Data 载荷再统一包信封」的路径了——会话与消息的数据面
        // 全在 VDFS 上，这里只剩编排 / 控制。
        //
        // `chat/send` / `chat/abort` 已随 D-066 退役：输入面双轨（inbox 地址
        // 写入 vs 两条全局路由）收为单轨，全部消费方改走地址面——
        // 发消息 = `vdfs/write <会话地址>/inbox`，恢复 = `action(<消息地址>, <动作词>)`，
        // 中止 = `action(<会话地址>, "abort")`。裁决见 D-066。
        match path {
            // 控制：把某个工具的永久放行上限抬到这次的风险档（只抬不降）。
            // 写的是本插件的全局配置（`<homedir>/session/PLUGIN.yml`），不是会话
            // metadata——「永久」按定义要跨会话（D-170）。
            "grant" => return self.handle_grant(ctx).await,
            // 读数：本会话 v2 事实源的四列（时延 / 兜底 / 成本 / 断点）。
            // 纯计算、零副作用——既不落盘也不碰消息。
            "stats" => return self.handle_stats(ctx).await,
            // 编排：把收件箱队首落成 `user.message` 事实（开轮），返回该事实的 seq。
            // 驱动不在本插件里——宿主看到这条事实后单次驱动本会话。
            "consume" => return self.handle_consume(ctx).await,
            // 投影：把该 seq 对应的终答事实折成消息节点并收束本轮。
            "project" => return self.handle_project(ctx).await,
            // 读数：本会话改过哪些文件（D-169：`artifact.added` 的一条投影，不是新事实族）。
            // 同样是纯计算、零副作用。
            "changes" => return self.handle_changes(ctx).await,
            // 本表只留「不是数据 CRUD」的路由：会话与消息的增删改查**全部**经 VDFS
            // 地址完成（`vdfs/list|read|write|action|delete`）。在路由上再开一条数据面
            // 入口 = 同一能力的第二份实现，两条实现会各自漂移——判据与逐条退役记录见
            // 设计文档（`stream` 见
            // 设计文档；选项三条见
            // 设计文档）。
            // 剩下的都是只读读数或控制动作（会话与消息的增删改查全在 VDFS 地址面）。
            _ => return Err(PluginError::NotFound(format!("未知路径: {path}"))),
        }
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        // 选项收集：会话作为选项宿主贡献「自有选项」（工作目录 / 风险等级 /
        // 运行模式）。与其它插件同构——命中 available_options 时才注册，
        // 经统一的 OPTION_VISITOR 收集器汇流。
        if ctx.get(symbio_sdk::keys::PATH).unwrap_or_default()
            == symbio_sdk::plugin::TRAVERSE_AVAILABLE_OPTIONS
        {
            if let Some(visitor) = ctx.get(symbio_sdk::capability::OPTION_VISITOR) {
                for (order, field) in self.session_option_fields() {
                    visitor.put(CapabilityRow::option(order, field)).await;
                }
            }
        }
        // 能力收集（available_tools）：本插件**不再贡献工具**——`heartbeat` 那条随调度
        // 搬进 `crates/symbio-plugin-heartbeat`（[D-077](../../symbio-plugin-heartbeat/README.md#d-077-心跳片的落点定为一层的插件目录卸载单位与第二个消费单位都由它给出 "crates/symbio-plugin-heartbeat/src/README")），
        // 而它曾是 session 在 LLM 可见工具清单里唯一的一条。
        // VDFS 挂载点由容器从 `get_vfs_provider` 派生（目录名 = 实例名）。
        if let Some(visitor) = ctx.get(symbio_sdk::capability::CAPABILITY_VISITOR) {
            // 智能体自身的 `AGENTS.md`（`{homedir}` / `<agentdir>`）与本会话无关：
            // 那是「智能体自身」与「工作区」两个作用域，归 memory 插件（谁能读写它，谁负责
            // 注入它）。本会话的 `MEMORY.md` 归本插件——各作用域各有一个所有者，
            // 层数与地址见 `providers/memory` 的模块文档。

            // 会话记忆（`<根>/session/<id>/MEMORY.md`）：**本会话私有**，可读写、有地址、
            // 有两道容量闸门——因此它归共享实现那套机制，本插件只负责「落位 + 标题 + 地址」。
            // 作用域闸门在 `contribute_memory` 内一处收口（无 `ctx[SESSION_ID]` 即不注入）。
            self.contribute_memory(&ctx, &visitor).await;
            // 首响的运行时债表（`session.opening`）：装载期只挂「这一族归谁申报」，
            // 每轮 `rows()` 现算是否欠首响（开关读当轮 `session.config`）。
            visitor
                .put(symbio_plugin_api::debt::debt_row(
                    "session",
                    Arc::new(crate::opening::OpeningDebts)
                        as Arc<dyn symbio_plugin_api::debt::DebtSource>,
                ))
                .await;
            // 终答的运行时债表（`session.answer`）：`classify.answered` 一经落格就欠
            // 一笔终答，执行体问 compose/wording 或读 assist 回执，由本插件落格。
            // 本行在 `plugin_row` 端口认领 ⇒ 模型行按端口等它，不再读 `compose_enabled`。
            visitor
                .put(symbio_plugin_api::debt::debt_row(
                    "session.answer",
                    Arc::new(crate::answer::AnswerDebts)
                        as Arc<dyn symbio_plugin_api::debt::DebtSource>,
                ))
                .await;
            // 轮边界汇报的运行时债表（`session.progress`）：一「轮边界」一笔债，
            // 边界身份由 `tool.calls.done` 的事实 id 派生；开关与上界读当轮的
            // `session.config`。措辞仍归 compose，转写仍归本插件（D-020）。
            visitor
                .put(symbio_plugin_api::debt::debt_row(
                    "session.progress",
                    Arc::new(crate::progress::ProgressDebts)
                        as Arc<dyn symbio_plugin_api::debt::DebtSource>,
                ))
                .await;
        }
        // 顺带声明「本插件有一份配置文档」（设置页据此列出并指路）
        symbio_sdk::capability::capability_announce_configurable(&ctx, &self.config_file).await;
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

symbio_plugin_api::submit_object_creator!(PLUGIN_ID_SESSION, SessionPlugin::build, dyn Plugin);

// ==================== 配置文档（`<根>/session/PLUGIN.yml`） ====================

/// 会话配置的定义 —— **定义由配置的拥有者产出**。
///
/// ## 覆盖判据：本定义与 [`SessionConfig`] 必须**逐字段对齐**
///
/// 字段键集合与 `SessionConfig` 序列化后的键集合**完全相等**，由单测
/// `config_definition_covers_every_session_config_field` 守着。少一个字段就是
/// 「开关存在、产品里开不了」——`skill_compile_enabled`（技能编译）与
/// `conation_enabled`（自主层）这两个**高阶能力**的开关曾长期只躺在配置文件里，
/// 前端设置页根本没有它们的控件，正是这条判据要防的形态。
///
/// ## 默认值单一真源
///
/// 字段默认值一律从 [`SessionConfig::default()`] 读出，不写第二份字面量：
/// 历史上定义寄居在 plugin_manager 插件里，schema 与 serde 各写一份默认值，出现过
/// 「schema 显示 15 而 serde 实际 65535」的漂移（面板显示的默认值与实际行为不符）。
/// 不变式由 `config_definition_defaults_come_from_session_config` 锁定。
///
/// ## 数值字段刻意不声明 `min` / `max`
///
/// 边界是**策略不是事实**：[`DetailField::check`] 会在保存时按 `min` / `max` 拒收，
/// 而 `SessionConfig` 里这些字段的真约束只有「非负」（都是无符号整数）。编一套宽窄
/// 无从论证的边界就是造一套假约束（`session-complexity-audit` 已记过这条教训），
/// 还会拒掉用户手工写进配置文件里的合法值。前端因此拿到 `null`，输入框不加
/// `min` / `max` 属性。
///
/// ## 分区
///
/// 多分区走 `DetailDefinition` 的**直接构造**（与 `agent/host` / `mcp` / `model`
/// 同一条形状），骨架（`binding = option` + 「保存配置」动作）用 `..form(...)` 复用，
/// 不另抄一份动作字面量。
fn config_definition() -> DetailDefinition {
    let d = SessionConfig::default();
    DetailDefinition {
        sections: vec![
            // 「基础」不折叠：进这一页的人多半只想改消息数与上下文轮数。
            DetailSection {
                title: Some("基础".into()),
                collapsed: false,
                fields: vec![
                    unbounded_number(
                        "max_messages",
                        "最大消息数",
                        "每个会话保存的最大消息数量（超出后按轮 FIFO 淘汰）",
                        json!(d.max_messages),
                    ),
                    unbounded_number(
                        "context_messages",
                        "上下文消息数量",
                        "Model 对话时包含的上下文消息数量（0 表示不限制，6 表示 3 轮对话）",
                        json!(d.context_messages),
                    ),
                    // ── Q1：worker 启动判据（三层按序短路，空值 / 0 = 该层关闭）───
                    DetailField {
                        default: Some(json!(d.worker_force_prefix)),
                        ..DetailField::text(
                            "worker_force_prefix",
                            "委派前缀",
                            "发言以此前缀开头即启动 worker（带尾空格，如 `/work `）；留空 = 关闭这一层",
                        )
                    },
                    DetailField {
                        default: Some(json!(d.worker_keywords)),
                        ..DetailField::text(
                            "worker_keywords",
                            "委派关键词",
                            "逗号分隔、忽略大小写，命中即启动 worker；留空 = 关闭这一层",
                        )
                    },
                    unbounded_number(
                        "worker_min_chars",
                        "委派长度阈值",
                        "发言字符数达到此值即启动 worker（0 = 关闭这一层）",
                        json!(d.worker_min_chars),
                    ),
                    unbounded_number(
                        "task_cap",
                        "并发任务容量",
                        "一轮最多派几笔活（就绪集容量）。1 = 串行 FIFO；0 与 1 同义",
                        json!(d.task_cap),
                    ),
                ],
            },
            DetailSection {
                title: Some("上下文与工具".into()),
                collapsed: true,
                fields: vec![
                    DetailField::toggle(
                        "auto_compress",
                        "自动压缩",
                        "上下文 Token 用量达到有效上限 70% 时自动压缩历史（LLM 语义快照）",
                        d.auto_compress,
                    ),
                    DetailField::toggle(
                        "prune_tool_history",
                        "落库期工具链裁剪",
                        "开启时在落库阶段物理删除上下文窗口之外的 Tool / ToolCall / Reasoning 节点\
                         （存储不再保留完整原文）；关闭后存储严格保留全文",
                        d.prune_tool_history,
                    ),
                ],
            },
            DetailSection {
                title: Some("记忆".into()),
                collapsed: true,
                fields: vec![
                    unbounded_number(
                        "memory_max_bytes",
                        "记忆写入上限（字节）",
                        "会话记忆文件（每个会话自己的 `MEMORY.md`）单次写入的字节上限，超出会被拒绝",
                        json!(d.memory_max_bytes),
                    ),
                    unbounded_number(
                        "memory_inject_max_bytes",
                        "记忆注入上限（字节）",
                        "每轮注入系统提示词的会话记忆正文字节上限，超出部分截断并告知地址",
                        json!(d.memory_inject_max_bytes),
                    ),
                ],
            },
            DetailSection {
                title: Some("对话面机制".into()),
                collapsed: true,
                fields: vec![
                    DetailField::toggle(
                        "classify_enabled",
                        "轮首判决",
                        "每轮开始时请 `classify` 判一次「直接回答还是派活」；关闭后全部输入直接进工具循环",
                        d.classify_enabled,
                    ),
                    DetailField::toggle(
                        "compose_enabled",
                        "对话面措辞",
                        "判决结果由 `compose` 组织成一句话；关闭后「直接回答」降级进工具循环（不沉默）",
                        d.compose_enabled,
                    ),
                    DetailField::toggle(
                        "opening_enabled",
                        "对话面开场白",
                        "「派活」时在轮首先说一句开场白（如「我先看一下」）；关闭后本轮照旧干活、只是不说这句（真实答复不受影响）",
                        d.opening_enabled,
                    ),
                    DetailField::toggle(
                        "progress_enabled",
                        "中途汇报",
                        "长任务在轮边界主动说一句进度；关闭后一次都不判定、不汇报",
                        d.progress_enabled,
                    ),
                    unbounded_number(
                        "progress_interval_ms",
                        "汇报静默阈值（毫秒）",
                        "距最近一次对话动静超过它才汇报；同时是措辞里「已经 N 分钟了」的来源",
                        json!(d.progress_interval_ms),
                    ),
                    unbounded_number(
                        "progress_max_per_turn",
                        "每轮汇报上限",
                        "每轮最多汇报几次（0 = 一次都不汇报；与「关掉汇报」同效但语义不同）",
                        json!(d.progress_max_per_turn),
                    ),
                    unbounded_number(
                        "progress_min_rounds",
                        "汇报起始轮次",
                        "本轮走完这么多轮工具调用后才允许汇报（0 = 第一个轮边界即可汇报）",
                        json!(d.progress_min_rounds),
                    ),
                    unbounded_number(
                        "progress_lease_ms",
                        "汇报租约（毫秒）",
                        "同一轮边界最多占用多久；超时未落由环按这条租约裁决（策略数，债行只读不写）",
                        json!(d.progress_lease_ms),
                    ),
                    unbounded_number(
                        "progress_max_attempts",
                        "汇报尝试上限",
                        "同一边界至多重派几次（租约的第二半；1 = 不重试）",
                        json!(d.progress_max_attempts),
                    ),
                    DetailField::toggle(
                        "supplements_enabled",
                        "补充整合",
                        "运行中连续发来的多条消息抽干合并成**同一条**用户消息，而不是一条消息占一轮",
                        d.supplements_enabled,
                    ),
                    unbounded_number(
                        "supplements_max_per_drain",
                        "单次抽干上限",
                        "一次最多合并几条消息，超出者留队等下一个抽干点",
                        json!(d.supplements_max_per_drain),
                    ),
                    DetailField::select(
                        "presentation_tree",
                        "建树策略",
                        PresentationTree::OPTIONS
                            .iter()
                            .map(|(value, label)| DetailOption {
                                value: (*value).to_string(),
                                label: (*label).to_string(),
                                description: None,
                            })
                            .collect(),
                        &d.presentation_tree,
                    ),
                ],
            },
            DetailSection {
                title: Some("自主与学习".into()),
                collapsed: true,
                fields: vec![
                    DetailField::toggle(
                        "conation_enabled",
                        "「欲」升格为任务",
                        "心跳表达出的意图是否经闸门升格成长目标任务。关闭后意图照样入格、照样可读，\
                         但一条自主任务都不产生（退化为纯响应式，完整运行）",
                        d.conation_enabled,
                    ),
                    DetailField::toggle(
                        "skill_compile_enabled",
                        "技能编译",
                        "成功收束的轨迹固化成一条技能事实（同一句触发至多一条）；关闭后不编译、只检索",
                        d.skill_compile_enabled,
                    ),
                    DetailField::toggle(
                        "skill_fast_path",
                        "技能快路（命中时跳过模型）",
                        "本轮发言逐字命中某条已编译技能时，直接以技能正文收束、不调模型。\
                         命中判据取恒等（更宽的「同类」判定是算法问题）",
                        d.skill_fast_path,
                    ),
                ],
            },
            DetailSection {
                title: Some("工具授权".into()),
                collapsed: true,
                fields: vec![DetailField::map(
                    "approval_grants",
                    "永久放行",
                    "工具名 → 允许的最高风险档（low / medium / high）。审批卡上的「永久放行」\
                     写入这里，同档及以下从此不问；高于上限仍会审批。删掉一行即撤销授权。",
                    json!(d.approval_grants),
                )],
            },
        ],
        ..DetailDefinition::form("会话设置", vec![])
    }
}

/// 无边界数字字段：先取 [`DetailField::number`] 的骨架（`widget` / `step` / 说明 /
/// 默认值的处理全部复用），再把 `min` / `max` 清空——理由见 [`config_definition`]。
fn unbounded_number(key: &str, label: &str, description: &str, default: Value) -> DetailField {
    DetailField {
        min: None,
        max: None,
        ..DetailField::number(key, label, description, 0.0, 0.0, default)
    }
}

// ==================== VDFS 挂载点（/session） ====================
//
// 会话是 VDFS 的第二个原生 provider：
//
// - **根 = 会话清单**：`<根>/session` 的目录内容即全部会话；根下可新建「会话」
//   （根节点自述里的 `new_type`），**新建语义完全由本 provider 自持**——id 由
//   provider 生成、路径名作标题、经 `create` 写意图区分「新建」与「覆盖」；
// - **节点 = 单个会话**：`ext = session` → 前端聊天工作区渲染器（同一份详情实现
//   只服务 VDFS 机制）；
// - **实时**：会话的任何变更经 [`SessionPlugin::notify_change`] 广播 → `watch`
//   的转发任务 → VDFS 事件总线，前端列表无需轮询即可收敛。
//
// 读写删都转发既有会话能力（SessionStore + 会话 metadata 合并），不新造协议。

mod nodes;
mod turn;
mod vdfs_catalog;
mod vdfs_inbox;
mod vdfs_messages;
mod vdfs_provider;
mod words;

// 模块内共享面：`nodes` 与 `vdfs_*` 四个域文件经 `use super::*;` 取用，测试（`plugin.test.rs`）亦同。
// 未被本文件引用的项由编译器 `unused_imports` 兜底。
pub(crate) use self::nodes::{
    inbox_dir_node, inbox_item_node, inbox_item_path, internal_dirs, message_node, message_of,
    message_path, message_text, messages_dir_node, ordered, overlay_live, parse_inbox_message,
    parse_session_path, session_content, session_node, subtree_nodes, transcript_window,
    trigger_declaration, window_params, SessionRuntime, VdfsSessionPath,
};
pub(crate) use self::words::{
    EXT_MESSAGE, EXT_SESSION, KIND_INBOX, KIND_MESSAGES, OUTCOME_ABORTED, OUTCOME_COMPLETED,
    OUTCOME_FAILED,
};

// 收件箱条目类型：`nodes` / `vdfs_inbox` / `inbox` 经 `use super::*;` 取用
pub(crate) use super::active::InboxItem;

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
