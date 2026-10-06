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
//! - [`vdfs_provider`]   `impl VdfsProvider` 及其私有辅助
//!
//! 本文件保留：插件结构体与构造、内部能力（`impl SessionPlugin`）、协议路由与
//! 能力注册（`impl Plugin`）、配置定义，以及各子模块的**共享面重导出**。

use super::chat_session::PersistentChatSession;
pub use super::config::SessionConfig;
use super::config::V2Mode;
use super::types::{Session, SessionSummary};
use crate::providers::MemoryFile;
use crate::symbio_core::{chat_message as cm, session_chat};
use crate::symbio_core::{
    plugin_dir_from_ctx, ExecEventSink, Plugin, PluginConfigFile, PluginDir, PluginError,
    PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse, PluginMeta, PluginPayload,
    PLUGIN_FILE, PLUGIN_ID_SESSION, SESSION_ID,
};
use crate::symbio_core::{
    DetailDefinition, DetailField, DetailOption, DetailSection, DynVdfsProvider, VdfsAccess,
    VdfsChange, VdfsChangeSubscriptions,
};
use crate::symbio_core::{VDFS_PARAM_BEFORE, VDFS_PARAM_LIMIT};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Weak};
use tokio::sync::{OnceCell, RwLock};

/// Session 插件
pub struct SessionPlugin {
    pub(crate) config: Arc<RwLock<SessionConfig>>,
    /// 配置文件的呈现与校验（`<根>/session/PLUGIN.yml`）——落盘写自己目录里的文件
    pub(crate) config_file: PluginConfigFile,
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
    /// 当前订阅者，[`crate::symbio_core::VdfsRequest::Watch`] 只往表里登记——变更因此无需
    /// 轮询即可到达 VDFS 事件总线，且**重叠订阅不会重复投递**（见该类型的文档）。
    ///
    /// 用 `Arc` 而非内联值：工作目录监听器（后台任务）也要投递进同一批订阅者，
    /// 需要共享所有权。
    pub(crate) change_subs: Arc<VdfsChangeSubscriptions>,
    /// 收件箱唤醒：入队时置位，常驻消费者据此醒来取件（见 `inbox` 模块）。
    /// 它是**唤醒**不是队列——队列本身在 `ActiveSessionStateInner::inbox`。
    pub(crate) inbox_wake: tokio::sync::Notify,
    /// 自身的弱引用（在 [`Self::build`] 里回填；见 [`Self::me`]）。
    self_ref: OnceCell<std::sync::Weak<SessionPlugin>>,
}

use super::store::SessionStore;

// ==================== 落库回包（§3.4 的唯一落地点） ====================
//
// 「谁落库、谁发帧」这条硬不变量（`docs/vdfs-session-messages.md` §3.4）原先由三个
// 落库调用点各自记住，于是各写各的：帧套不套 `llm_message_frame`、按捕获的 sink 发
// 还是按会话 id 现场解析转写，全都不同。漏发一次的后果是**静默的**（节点永持在途号、
// 下一条用户消息跳到它前面），因此把「落库 → 逐条下发权威副本」收成一个函数，
// 调用点只声明**走哪条通道**。

/// 落库回包的**发布通道**：拿到转写的方式有两种，帧怎么发只有一种。
///
/// | 通道 | 用在哪 | 落点 |
/// |---|---|---|
/// | [`Self::Sink`] | 执行期（chat_loop 主循环） | `ExecEventSink`（生产形态 `TranscriptSink` → `Transcript::apply`） |
/// | [`Self::Transcript`] | 编排入口与压缩域 | [`SessionPlugin::transcript_apply`]（按会话 id 现场解析） |
///
/// 两条通道的差别只在**怎么拿到转写**；帧的构造与逐条下发都在
/// [`append_and_publish`] 里，不允许在调用点再写一份。
pub(crate) enum PublishTarget<'a> {
    /// 执行期出口：帧直投 sink。
    Sink(&'a ExecEventSink),
    /// 插件出口：按会话 id 解析转写后逐条应用。
    Transcript {
        plugin: &'a SessionPlugin,
        session_id: &'a str,
    },
}

impl PublishTarget<'_> {
    async fn publish(&self, message: cm::ChatMessage) {
        match self {
            Self::Sink(sink) => sink.emit(message).await,
            Self::Transcript { plugin, session_id } => {
                plugin
                    .transcript_apply(session_id, crate::symbio_core::llm_message_frame(&message))
                    .await
            }
        }
    }
}

/// 落库 + **落库回包**的唯一实现（`docs/vdfs-session-messages.md` §3.4）。
///
/// 两步顺序写死在这里：`append_messages` 落库（存储在临界区内给消息补权威 `seq` /
/// `timestamp`，交回**权威副本**），随即把权威副本逐条下发——权威号由此换入实时面。
///
/// ## 为什么发的是交回的那份，而不是入参那份
///
/// 号是**临界区内补在存储自己的副本上**的，调用方手里那条没有号——发它等于把「没有
/// 号」写进前端（这也是 `append_messages` 返回消息而非条数的唯一原因）。因此这里
/// 下发的是它交回的权威副本，不必再从存储读回来找。
///
/// 正文以 `content`（整条替换）而非 `delta` 下发，正是为了这里：持有乐观副本 / 在途
/// 副本的消费端要**替换**成权威正文，而不是往自己那份后面再拼一遍。这也是 `delta` /
/// `content` 两个字段必须分开的原因——同一个消费端既可能需要追加（流式），也可能
/// 需要替换（权威副本对齐），而帧必须自证是哪一种。
///
/// ## 落库补身份在这里，不在别处（[plan/11 批 1](../../../docs/plan/11-多执行器与多主体加固实施方案.md) ②）
///
/// 本函数是**唯一的**消息追加写点（生产代码里 `append_messages` 只有这一处调用），
/// 所以 [`attributed`] 补在这里就等于「每条持久化消息都带身份」——补在各处建消息
/// 的地方则必然漏：建消息的写点有七八处（用户发言 / 助手正文 / 工具结果 / 压缩
/// 记录 / 系统说明），漏一处就造出「有身份、没有身份」两种同义消息。
///
/// `session_principal` 是**本会话主体**，谁说话按 role 分：人（`user`）不随会话变，
/// 其余都是本会话主体说的。
///
/// ## 用户发言那条为什么也在这个出口里
///
/// 它由 `orchestrator/entry.rs` 直连存储追加——前端手里只有自己的**乐观副本**，
/// 其 `seq` 是本地游标发的号，永远拿不到存储分配的那个；不回包就是两套序号空间并存。
///
/// 只在**成功**时下发；落库失败原样返回 `Err`，错误怎么呈现交给调用方——三条路径的
/// 降级策略各不相同（chat_loop 发 `Warn` 继续、编排广播失败结局并中断、压缩只记日志），
/// 那是各自的领域判定，不该被这个出口统一掉。
pub(crate) async fn append_and_publish(
    session: &PersistentChatSession,
    messages: Vec<cm::ChatMessage>,
    to: PublishTarget<'_>,
    session_principal: &str,
) -> Result<(), PluginError> {
    let persisted = session
        .append_messages(
            messages
                .into_iter()
                .map(|m| attributed(m, session_principal))
                .collect(),
        )
        .await?;
    for message in persisted {
        to.publish(message).await;
    }
    Ok(())
}

/// 落库前补身份的**唯一执行点**：写方知道这一轮是谁在说话，只补**没标过**的——
/// 转播进来的别的会话 / 子智能体消息带着自己的主体，不得被本会话覆盖（那是串主体）。
///
/// 人说的话是 `user`（会话外的一方，不随会话变），其余（助手正文、工具结果、
/// 压缩记录、系统说明）都是本会话主体说的。见 [`append_and_publish`]：为什么补
/// 在落库这一个点上、而不是各处建消息时。
fn attributed(m: cm::ChatMessage, session_principal: &str) -> cm::ChatMessage {
    if m.principal.is_some() {
        return m;
    }
    let mut out = m;
    out.principal = Some(match out.role {
        Some(cm::MessageRole::User) => crate::symbio_core::authz::PRINCIPAL_USER.to_string(),
        _ => session_principal.to_string(),
    });
    out
}

impl SessionPlugin {
    /// 主构造函数（Factory 机制使用）
    pub fn new(parent: Option<Weak<dyn Plugin>>, config: SessionConfig, dir: PluginDir) -> Self {
        // 变更订阅表：provider 自持一份，工作目录监听器共享同一份（见下方注入）
        let change_subs = Arc::new(VdfsChangeSubscriptions::default());
        // 目录树场景同时服务 VDFS：文件变化经**同一张订阅表**转发给 `<根>`
        // 订阅方，VDFS 侧不必另开一套监听（实时链路在机制层合流）。
        let workdir_watches = super::workdir::WorkdirWatchManager::default();
        workdir_watches.set_vdfs_subs(change_subs.clone());
        Self {
            config: Arc::new(RwLock::new(config)),
            config_file: PluginConfigFile::new(dir, "会话设置", config_definition()),
            parent,
            active_mgr: Arc::new(super::active::ActiveSessionManager::new(
                change_subs.as_ref().clone(),
            )),
            store: OnceCell::new(),
            heartbeat_state: Arc::new(RwLock::new(HashMap::new())),
            workdir_watches,
            change_subs,
            inbox_wake: tokio::sync::Notify::new(),
            self_ref: OnceCell::new(),
        }
    }

    /// 取回 `Arc<Self>`（装配完成后恒可用）。
    ///
    /// ## 为什么需要它
    ///
    /// `VdfsProvider::dispatch` 拿到的是 `&self`，而「开跑一轮」（`start_turn`）要把
    /// `Arc<Self>` 移进后台任务。没有这条自引用，provider 侧想触发运行就只能绕道
    /// **路由**——而路由是 address-less 的（恒落根实例），那正是子智能体空间的消息
    /// 被投到父空间的成因。有了它，「一次节点动作 → 本空间开跑」在 provider 内部
    /// 就闭合了，不需要任何跨插件调用。
    pub(crate) fn me(&self) -> Result<Arc<Self>, PluginError> {
        self.self_ref
            .get()
            .and_then(|w| w.upgrade())
            .ok_or_else(|| PluginError::InternalError("会话插件未完成装配（缺少自引用）".into()))
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
    /// 那笔请求永远改变不了结论，已删——见 `cli/src/client.rs` 的说明）。
    pub(crate) fn notify_change(&self, id: &str) {
        self.change_subs.notify(&VdfsChange::bare(id));
    }

    // ==================== 转写发布（消息变更的唯一出口）====================
    //
    // 一切消息级变更（压缩节点 / 前端 CRUD 动作 / 用户消息定稿 / 失败与中止的
    // 终态收敛）都经 `Transcript::apply` 发布——内存图、seq、核心日志、发布在
    // 同一个函数里完成。VDFS 转写列表的在途叠加读的是同一份内存图，
    // 「存储一份 + 在途一份」两种表示从此同源。
    //
    // 运行中的轮次另有唯一的常规写入者：消费循环（`orchestrator::consume`）把
    // 通道上的消息喂给同一个 `Transcript`——上游有多少个发射点都无所谓，
    // 到写入点只剩一个。
    //
    // 落库之后的**回包**（权威 `seq` 换入）另有唯一出口：模块级 `append_and_publish`，
    // 三条落库路径（主循环增量 / 用户发言 / 压缩节点）都经它逐条下发权威副本。

    /// 向会话转写发布一条消息帧（唯一出口）。
    pub(crate) async fn transcript_apply(&self, session_id: &str, message: cm::ChatMessage) {
        let state = self.active_mgr.get_or_create(session_id).await;
        state.transcript.lock().await.apply(message);
    }

    /// 向会话转写发布一批消息帧（同一把锁内顺序应用，保持发布顺序）。
    pub(crate) async fn transcript_apply_all(
        &self,
        session_id: &str,
        messages: Vec<cm::ChatMessage>,
    ) {
        let state = self.active_mgr.get_or_create(session_id).await;
        let mut tr = state.transcript.lock().await;
        for message in messages {
            tr.apply(message);
        }
    }

    /// **整表重写**（L2 语义压缩）后的收敛：被压掉的消息逐条删除帧，
    /// 新的首条快照作为一条**完整消息**下发（`content` = 整条替换）。
    ///
    /// 逐条删除帧是元数据（id + 状态，每条几十字节），条数受上下文窗口上界
    /// 约束——比「清空 + 整份重读」便宜得多：保留的消息不再重传。协议里也没有
    /// 「清空重读」这种形态（消费端要重读只有一条路：自己发现序号缺口）。
    pub(crate) async fn emit_transcript_rewritten(
        &self,
        session_id: &str,
        dropped: &[String],
        head: &cm::ChatMessage,
    ) {
        let mut frames: Vec<cm::ChatMessage> = dropped
            .iter()
            .map(|mid| crate::symbio_core::llm_removed_frame(mid))
            .collect();
        // 新的首条（压缩快照）：一条完整消息（身份 + 正文 + 终态同帧）。
        frames.push(crate::symbio_core::llm_message_frame(head));
        self.transcript_apply_all(session_id, frames).await;
    }

    // 落库回包的**下发半程**已与落库合并在模块级的 `append_and_publish`：
    // 「落库 → 逐条下发权威副本」是一个动作，不该由每个落库调用点各写一遍。

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
                crate::plugin_warn!("session", "读取自身配置失败，改用默认值：{e}");
                SessionConfig::default()
            }
        };

        let parent = ctx.parent();

        let plugin = Arc::new(SessionPlugin::new(parent, config, dir));
        // 自引用：provider 侧的「开跑一轮」需要 `Arc<Self>`（见 [`SessionPlugin::me`]）
        let _ = plugin.self_ref.set(Arc::downgrade(&plugin));

        // 启动心跳任务调度器（后台常驻）。仅在存在 Tokio runtime 时启动，
        // 避免单元测试（无 runtime）中 `tokio::spawn` 触发 panic。
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            let scheduler = plugin.clone();
            handle.spawn(async move {
                scheduler.run_heartbeat_loop().await;
            });
        }

        // 收件箱消费者（后台常驻，见 `inbox` 模块）：与心跳同处构造点，
        // 因此**早于任何写入**——"谁来消费第一次写入"不依赖装配顺序。
        plugin.clone().spawn_inbox_consumer();

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
    ) -> Result<Arc<PersistentChatSession>, PluginError> {
        let store = self.get_store().await?;
        Ok(Arc::new(super::chat_session::PersistentChatSession::new(
            session_id.to_string(),
            self.config.clone(),
            store,
        )))
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

    /// 参与能力收集：交出**会话记忆**（系统提示词片段）。
    ///
    /// 无会话 id 时静默跳过——没有会话就没有「本会话的记忆」，这不是故障，
    /// 不该往收集期错误桶里塞东西。
    async fn contribute_memory(
        &self,
        ctx: &Arc<dyn PluginInvokeRequest>,
        visitor: &Arc<dyn crate::symbio_core::CapabilityVisitor>,
    ) {
        let sid = ctx.get(SESSION_ID).unwrap_or_default();
        let store = self.memory_store(&sid).await;
        if !store.has_scope() {
            return;
        }
        // 绝对地址 = 上下文父地址 + 相对地址（容器转发时已写入父地址）
        let address = crate::symbio_core::absolute_addr(ctx, &super::memory::memory_rel_path(&sid));
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

    fn get_vfs_provider(self: Arc<Self>) -> Option<Arc<dyn crate::symbio_core::VdfsProvider>> {
        Some(self)
    }

    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(crate::symbio_core::PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);

        // 会话存储已迁移到 <本插件目录>/ 全局目录，session/* 系列接口
        // 不再依赖 ctx.workdir；ctx.workdir 仅在 chat 路径和需要 Model 路由时使用。
        //
        // 每个分支都**自己返回**（`match` 的值不再被使用）：本表已经没有
        // 「算出一份 Data 载荷再统一包信封」的路径了——会话与消息的数据面
        // 全在 VDFS 上，这里只剩编排 / 控制。
        match path {
            "chat/send" => return self.handle_chat_send_oneoff(ctx).await,
            "chat/abort" => return self.handle_chat_abort_oneoff(ctx).await,
            // 读数：本会话 v2 事实源的四列（时延 / 兜底 / 成本 / 断点）。
            // 纯计算、零副作用——既不落盘也不碰消息。
            "stats" => return self.handle_stats(ctx).await,
            // 本表只留「不是数据 CRUD」的路由：会话与消息的增删改查**全部**经 VDFS
            // 地址完成（`vdfs/list|read|write|action|delete`）。在路由上再开一条数据面
            // 入口 = 同一能力的第二份实现，两条实现会各自漂移——判据与逐条退役记录见
            // `docs/archive/legacy-route-migration.md`（`stream` 见
            // `docs/archive/session-realtime-vdfs-watch.md`；选项三条见
            // `docs/archive/session-options-unification.md`）。
            // 剩下的都不是 CRUD：前三条是**编排 / 控制 / 读数**。
            _ => return Err(PluginError::NotFound(format!("未知路径: {path}"))),
        }
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        // 选项收集：会话作为选项宿主贡献「自有选项」（工作目录 / 风险等级 /
        // 运行模式 / 心跳）。与其它插件同构——命中 available_options 时才注册，
        // 经统一的 OPTION_VISITOR 收集器汇流。
        if ctx.get(crate::symbio_core::PATH).unwrap_or_default()
            == crate::symbio_core::TRAVERSE_AVAILABLE_OPTIONS
        {
            if let Some(visitor) = ctx.get(crate::symbio_core::OPTION_VISITOR) {
                for (order, field) in self.session_option_fields() {
                    visitor.register_option_field(order, field).await;
                }
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
                    .register(Arc::new(super::tools::HeartbeatTool::new(Arc::downgrade(
                        &self,
                    ))))
                    .await;
            }
        }
        // VDFS 挂载点：与会话工具共用同一次能力广播，把自己注册为一份 VDFS 资源。
        // 挂载名由使用方（此处即本插件）选定——约定用插件名（`PLUGIN_ID_SESSION`），
        // 插件名在宿主内唯一，天然就是合格的挂载名；provider 自身不含此概念。
        if let Some(visitor) = ctx.get(crate::symbio_core::CAPABILITY_VISITOR) {
            let me: DynVdfsProvider = self.clone();
            visitor.register_vdfs_provider(PLUGIN_ID_SESSION, me).await;

            // 智能体自身的 `AGENTS.md`（`{homedir}` / `<agentdir>`）**不再在此注入**：
            // 那是「智能体自身目录」这个作用域，归 plugin_manager 插件（谁能读写它，谁负责
            // 注入它）。工作区 `AGENTS.md` 归 work 插件，本会话的 `MEMORY.md` 归本
            // 插件——三层各有一个所有者，见 `providers/memory` 的模块文档。

            // 会话记忆（`<根>/session/<id>/MEMORY.md`）：**本会话私有**，可读写、有地址、
            // 有两道容量闸门——因此它归共享实现那套机制，本插件只负责「落位 + 标题 + 地址」。
            // 作用域闸门在 `contribute_memory` 内一处收口（无 `ctx[SESSION_ID]` 即不注入）。
            self.contribute_memory(&ctx, &visitor).await;
        }
        // 顺带声明「本插件有一份配置文档」（设置页据此列出并指路）
        crate::symbio_core::capability_announce_configurable(&ctx, &self.config_file).await;
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

crate::submit_object_creator!(PLUGIN_ID_SESSION, SessionPlugin::build, dyn Plugin);

// ==================== 配置文档（`<根>/session/PLUGIN.yml`） ====================

/// 会话配置的定义 —— **定义由配置的拥有者产出**。
///
/// ## 覆盖判据：本定义与 [`SessionConfig`] 必须**逐字段对齐**
///
/// 字段键集合与 `SessionConfig` 序列化后的键集合**完全相等**，由单测
/// `config_definition_covers_every_session_config_field` 守着。少一个字段就是
/// 「开关存在、产品里开不了」——`skill_compile_enabled`（S11 技能编译）与
/// `conation_enabled`（S12 自主层）这两个**高阶能力**的开关曾长期只躺在配置文件里，
/// 前端设置页根本没有它们的控件，正是这条判据要防的形态。
///
/// ## 默认值单一真源
///
/// 字段默认值一律从 [`SessionConfig::default()`] 读出，不写第二份字面量：
/// 历史上定义寄居在 plugin_manager 插件里，schema 与 serde 各写一份默认值，出现过
/// `max_tool_rounds` schema=15 而 serde=65535 的漂移（面板显示的默认值与
/// 实际行为不符）。不变式由 `config_definition_defaults_come_from_session_config`
/// 锁定。
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
                    unbounded_number(
                        "max_tool_rounds",
                        "工具轮次上限",
                        "单轮最多工具迭代次数（0 = 不限制）。达到上限会先给出明确提示再正常退出，不是静默熔断",
                        json!(d.max_tool_rounds),
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
                        "enable_compact_tool",
                        "工具压缩",
                        "向模型提供主动压缩工具（context_compact）与 55% 水位提醒；关闭后仅保留自动压缩兜底",
                        d.enable_compact_tool,
                    ),
                    unbounded_number(
                        "compress_line_threshold",
                        "淡化行数阈值",
                        "请求视图中超过此行数的内容节点做头尾淡化（存储恒为完整原文）",
                        json!(d.compress_line_threshold),
                    ),
                    unbounded_number(
                        "compress_keep_recent",
                        "淡化保护窗口",
                        "请求视图中最近的 N 条内容节点豁免淡化（0 表示不保护）",
                        json!(d.compress_keep_recent),
                    ),
                    unbounded_number(
                        "tool_context_window",
                        "工具结果保留窗口",
                        "保留完整结果的最近工具调用数量，更早的按骨架化策略替换为占位文案",
                        json!(d.tool_context_window),
                    ),
                    unbounded_number(
                        "fade_activate_rounds",
                        "工具结果淡化阈值",
                        "单轮工具迭代轮数超过此值后，较早轮次的工具结果在请求视图中压成头尾摘要",
                        json!(d.fade_activate_rounds),
                    ),
                    unbounded_number(
                        "fade_keep_recent_turns",
                        "淡化保留轮数",
                        "最近 N 个用户轮次的工具结果保持原文，更早的才淡化",
                        json!(d.fade_keep_recent_turns),
                    ),
                    DetailField::toggle(
                        "prune_tool_history",
                        "落库期工具链裁剪",
                        "开启时在落库阶段物理删除上下文窗口之外的 Tool / ToolCall / Reasoning 节点\
                         （存储不再保留完整原文）；关闭后存储严格保留全文，裁剪只发生在请求视图",
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
                        "progress_min_rounds",
                        "汇报最少轮次",
                        "已完成的工具轮次达到它才有「进展」可报（第一轮就跑完的任务不该被打断）",
                        json!(d.progress_min_rounds),
                    ),
                    unbounded_number(
                        "progress_max_per_turn",
                        "每轮汇报上限",
                        "每轮最多汇报几次（0 = 一次都不汇报；与「关掉汇报」同效但语义不同）",
                        json!(d.progress_max_per_turn),
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
                         命中判据取恒等（更宽的「同类」判定是算法问题）；只在 full 档生效",
                        d.skill_fast_path,
                    ),
                    DetailField::select(
                        "v2_mode",
                        "事实链路档位",
                        vec![
                            DetailOption {
                                value: "off".into(),
                                label: "off（纯 v1：不转写事实）".into(),
                                description: None,
                            },
                            DetailOption {
                                value: "bridge".into(),
                                label: "bridge（v1 运行 + v2 事实累积，默认）".into(),
                                description: None,
                            },
                            DetailOption {
                                value: "full".into(),
                                label: "full（整体切换：轮次走 v2 引擎）".into(),
                                description: None,
                            },
                        ],
                        v2_mode_name(d.v2_mode),
                    )
                    .with_description(
                        "管辖范围 = v2 事实链路的切换档位。`full` 的覆盖面是**非交互轮次**\
                         （有工具 / 无工具都走 v2）；交互轮（审批 / 问答等待）的恢复仍是独立一批",
                    ),
                ],
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

/// 档位名（[`V2Mode`] 的 serde 表示）——面板的候选值与默认值共用它，不在 schema 侧
/// 另写一份档位字面量。漂移由 `config_definition_defaults_come_from_session_config`
/// 兜底（它比的就是这里的默认值与 `SessionConfig::default()` 的序列化结果）。
fn v2_mode_name(mode: V2Mode) -> &'static str {
    match mode {
        V2Mode::Off => "off",
        V2Mode::Bridge => "bridge",
        V2Mode::Full => "full",
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
mod vdfs_provider;
mod words;

// 模块内共享面：`nodes` / `vdfs_provider` 经 `use super::*;` 取用，测试（`plugin.test.rs`）亦同。
// 未被本文件引用的项由编译器 `unused_imports` 兜底。
pub(crate) use self::nodes::{
    inbox_dir_node, inbox_item_node, inbox_item_path, internal_dirs, message_node, message_of,
    message_path, message_text, messages_dir_node, ordered, overlay_live, parse_inbox_message,
    parse_session_path, session_content, session_node, transcript_window, window_params,
    SessionRuntime, VdfsSessionPath,
};
pub(crate) use self::words::{
    EXT_MESSAGE, EXT_SESSION, KIND_INBOX, KIND_MESSAGES, OUTCOME_ABORTED, OUTCOME_COMPLETED,
    OUTCOME_FAILED,
};

// 收件箱条目类型：`nodes` / `vdfs_provider` / `inbox` 经 `use super::*;` 取用
pub(crate) use super::active::InboxItem;

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
