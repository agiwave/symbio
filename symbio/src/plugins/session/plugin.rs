//! Session 插件实现
//!
//! 提供会话历史和上下文管理。
//!
//! 存储路径：`<homedir>/plugins/session/`（从 [`HomedirRegistry`] 直接派生）。
//! 不再使用 `storage_dir` 配置项，session 存储始终跟随系统目录。
//! 会话本身携带 `metadata.workdir` 用于 MODEL 工具调用上下文。
//!
//! ## 系统目录 (homedir)
//!
//! Session 存储目录由 [`HomedirRegistry::get()`] 派生：`<homedir>/plugins/session`。
//! 切换 homedir 后，新会话将写入新 homedir；存量数据**不会**自动迁移。

use super::chat_session::ChatSession;
use super::types::Session;
use crate::providers::vdfs_service::config::{self, ConfigDoc};
use crate::symbio_core::schemas::detail::{DetailDefinition, DetailField};
use crate::symbio_core::schemas::options::OPTIONS_LIST;
use crate::symbio_core::schemas::session::chat_message as cm;
use crate::symbio_core::schemas::session::session_chat_response;
pub use crate::symbio_core::schemas::session::session_config::SessionConfig;
use crate::symbio_core::vdfs;
use crate::symbio_core::{
    InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginError, PluginFrame, PluginMeta,
    PluginPayload, PLUGIN_SESSION,
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
    /// 配置文档（`.vdfs/session/配置`）——节点形状 / 校验 / 落盘推送给它
    pub(crate) config_doc: ConfigDoc,
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
    /// VDFS 实时：会话变更广播源。
    ///
    /// provider 是**变更源的持有者**：会话的任何写入 / 删除都经此广播，
    /// [`vdfs::VdfsProvider::watch`] 的转发任务订阅它并调用 sink，
    /// 变更因此无需轮询即可到达 VDFS 事件总线。
    pub(crate) change_tx: tokio::sync::broadcast::Sender<vdfs::VdfsChange>,
    /// VDFS 实时：被订阅路径 → 转发任务（`unwatch` 时取消）
    pub(crate) watch_tasks: Arc<tokio::sync::Mutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
}

use super::store::SessionStore;

impl SessionPlugin {
    /// 主构造函数（Factory 机制使用）
    pub fn new(parent: Option<Weak<dyn Plugin>>, config: SessionConfig) -> Self {
        // 变更广播：容量只需覆盖「一次突发写入 + 少量并发订阅者」；
        // 无订阅者时 send 静默失败（broadcast 语义），因此不设保留位。
        let (change_tx, _) = tokio::sync::broadcast::channel(64);
        // 目录树场景同时服务 VDFS：文件变化经**同一广播源**转发给 `.vdfs`
        // 订阅方，VDFS 侧不必另开一套监听（实时链路在机制层合流）。
        let workdir_watches = super::workdir::WorkdirWatchManager::default();
        workdir_watches.set_vdfs_sender(change_tx.clone());
        Self {
            config: Arc::new(RwLock::new(config)),
            config_doc: ConfigDoc::new(PLUGIN_SESSION, "会话设置", config_definition()),
            parent,
            active_mgr: Arc::new(super::active::ActiveSessionManager::new()),
            store: OnceCell::new(),
            heartbeat_state: Arc::new(RwLock::new(HashMap::new())),
            workdir_watches,
            change_tx,
            watch_tasks: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    /// 广播一次会话变更（VDFS 实时链路的数据源）。
    ///
    /// 无订阅者时静默丢弃；`path` 是 provider 子树内的相对路径（= 会话 id）。
    pub(crate) fn notify_change(&self, id: &str, change: &str) {
        let _ = self
            .change_tx
            .send(vdfs::VdfsChange::new(id.to_string(), change.to_string()));
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
        let _ = self.change_tx.send(vdfs::VdfsChange::new(
            message_path(session_id, mid),
            vdfs::VFDS_CHANGE_DELETED,
        ));
    }

    /// 清空整份转写 → 落在 `消息` 目录本身上的 `deleted`（消费者清空列表）
    pub(crate) fn emit_transcript_cleared(&self, session_id: &str) {
        let _ = self.change_tx.send(vdfs::VdfsChange::new(
            message_dir_path(session_id),
            vdfs::VFDS_CHANGE_DELETED,
        ));
    }

    /// 就地改写单条消息 → `updated`（带节点视图 + 内容快照）
    pub(crate) fn emit_message_updated(&self, session_id: &str, msg: &cm::ChatMessage) {
        let _ = self.change_tx.send(message_payload(
            vdfs::VdfsChange::new(message_path(session_id, &msg.id), vdfs::VFDS_CHANGE_UPDATED),
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
        // 无订阅者时静默丢弃（broadcast 语义），因此这条发射对不关心 VDFS 的
        // 调用方零成本。
        let _ = self
            .change_tx
            .send(message_change(session_id, view, existed, appended));
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
        // 反序列化时使用 #[serde(default)]，自动忽略 storage_dir 等已被废弃的字段
        let config: SessionConfig = ctx
            .config()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();

        let parent = ctx.parent();

        let plugin = Arc::new(SessionPlugin::new(parent, config));

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

    /// Session 存储目录：`<homedir>/plugins/session`
    ///
    /// 构造式不在本插件里手写——直接取宿主层的资源类别根
    /// [`category_dir`](crate::providers::vdfs_service::entry::category_dir)
    /// （`<homedir>/plugins/<类别>`，类别段名 = 插件名）。会话因此与
    /// model / skill / mcp 共用同一条「插件名 → 存储类别」的映射，
    /// 不再各插件手拼一次 `join("plugins").join(...)`。
    ///
    /// 这是 session 存储目录的**唯一权威位置**，不依赖任何 config 字段。
    /// 切换 homedir 后 worker composite 会整体重建（`home/reload`），
    /// 新插件实例的下一次 `get_store` 因此用新 homedir 下的目录。
    pub fn session_storage_dir() -> PathBuf {
        crate::providers::vdfs_service::entry::category_dir(PLUGIN_SESSION)
    }

    /// 获取（或初始化）存储后端。全局单例，首次调用时创建。
    pub(crate) async fn get_store(&self) -> Result<Arc<SessionStore>, PluginError> {
        if let Some(store) = self.store.get() {
            return Ok(Arc::clone(store));
        }

        let store = Arc::new(SessionStore::new(Self::session_storage_dir()));

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

    pub(crate) async fn list_sessions(&self) -> Result<Vec<Session>, PluginError> {
        self.get_store().await?.list_sessions().await
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
}

#[async_trait]
impl Plugin for SessionPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let path = ctx.get(crate::symbio_core::PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);

        // 会话存储已迁移到 ~/.symbio/plugins/session/ 全局目录，session/* 系列接口
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
        }
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

crate::submit_object_creator!(PLUGIN_SESSION, SessionPlugin::build, dyn Plugin);

// ==================== 会话列表展示辅助 ====================

/// 会话列表一行摘要：最后一条含文本消息的首行（压缩空白、限长 60 字符）。
///
/// 与 [`crate::plugins::session::types::derive_session_title`] 同风格；
/// 供会话节点的 `description` 驱动列表「实时缩略」预览。
fn derive_session_summary(
    messages: &[crate::symbio_core::schemas::session::chat_message::ChatMessage],
) -> Option<String> {
    const SUMMARY_MAX_CHARS: usize = 60;
    let text = messages
        .iter()
        .rev()
        .filter_map(|m| m.content.as_ref().map(|c| c.to_text()))
        .map(|t| t.trim().to_string())
        .find(|t| !t.is_empty())?;
    let first_line = text.lines().map(str::trim).find(|l| !l.is_empty())?;
    let mut out = String::new();
    let mut chars = first_line.chars();
    for _ in 0..SUMMARY_MAX_CHARS {
        match chars.next() {
            Some(c) if c.is_whitespace() => {
                if !out.ends_with(' ') {
                    out.push(' ');
                }
            }
            Some(c) => out.push(c),
            None => return Some(out.trim_end().to_string()),
        }
    }
    Some(format!("{}…", out.trim_end()))
}

/// 会话的通用元信息标签（工作目录名 + 消息数）。
///
/// 标签由本函数单点产出，挂在 VDFS 节点的 `attributes.meta_tags` 上，保证同一
/// 会话在清单与详情里的呈现一致；前端原样渲染，不含语义。
fn session_meta_tags(s: &Session) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    if let Some(wd) = s
        .metadata
        .get("workdir")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let base = wd
            .trim_end_matches(['/', '\\'])
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(wd);
        if !base.is_empty() {
            tags.push(base.to_string());
        }
    }
    tags.push(format!("{} 条", s.messages.len()));
    tags
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

/// 会话节点：`ext = session`（前端据此选聊天工作区渲染器）。
///
/// 标题 / 摘要 / 状态 / 元信息标签与清单呈现**同源**（复用
/// `display_title`、`derive_session_summary` 与 [`session_meta_tags`]），
/// 保证同一会话在清单与聊天工作区里的呈现一致。
///
/// 另在 `attributes` 上挂载会话清单所需字段（`message_count` / `metadata` /
/// `meta_tags`）——它们是**场景数据**，VDFS 只透传；会话清单由此可直接用
/// `vdfs/list` 一次取全（见 S8）。
fn session_node(s: &Session, is_working: bool) -> vdfs::VdfsNode {
    let mut n = vdfs::VdfsNode::file(&s.id, s.display_title(), vdfs::VdfsAccess::READ_WRITE);
    n.kind = PLUGIN_SESSION.to_string();
    n.ext = Some(vdfs::VFDS_EXT_SESSION.to_string());
    n.status = if is_working {
        vdfs::VFDS_STATUS_WORKING
    } else {
        vdfs::VFDS_STATUS_ACTIVE
    }
    .to_string();
    n.updated_at = Some(s.updated_at);
    n.description = derive_session_summary(&s.messages);
    let _ = n
        .attributes
        .insert("message_count".to_string(), json!(s.messages.len()));
    let _ = n
        .attributes
        .insert("metadata".to_string(), s.metadata.clone());
    let _ = n
        .attributes
        .insert("meta_tags".to_string(), json!(session_meta_tags(s)));
    n
}

// ==================== VDFS：会话内部寻址 ====================
//
// 会话在 VDFS 上**保持叶子**（`ext = session`，点击进聊天详情，语义不变）；
// 其内部结构（转写 / 子会话 / 工作目录树）作为**会话同名目录**挂在会话之下：
//
//   <id>                  → 会话叶子（聊天详情）
//   <id>/消息[/<mid>]      → 转写列表 / 单条消息（**列表项**）
//   <id>/子会话[/<sub>]    → 子会话清单 / 单个子会话（查看 · 删除）
//   <id>/工作目录[/<rel>]  → 工作目录树（文件可查看 / 编辑）
//
// 该寻址取代原「容器页」——同一批能力改由 VDFS 承载，机制侧零新增概念；
// 场景实现仍复用 `workdir` 与子会话清单，VDFS 是这些能力的唯一入口。

/// 会话内部：转写列表的路径段（同时是展示名）。
///
/// 转写是**列表**：`.vdfs/session/<id>/消息` 的每一项是一条消息，顺序由
/// `seq`（唯一权威顺序锚点）决定。流式输出是列表项的**追加型变更**
/// （`VFDS_CHANGE_APPENDED`），不是另一条协议。
const SEG_MESSAGES: &str = "消息";

/// 转写列表本身的 provider 子树内路径（`<id>/消息`）。
///
/// 与 [`message_path`] 同源：列表与列表项是同一地址方案的两级。
fn message_dir_path(session_id: &str) -> String {
    format!("{session_id}/{SEG_MESSAGES}")
}

/// 单条消息的 provider 子树内路径（`<id>/消息/<mid>`）。
///
/// 与 [`parse_session_path`] 互逆，因此与它同处——地址的「拼」与「解」必须同源，
/// 分开写就会在改地址方案时漏改一边。变更发射（[`message_change`]）用它构造
/// `VdfsChange::path`，与 `list` 返回的节点地址严格一致；编排层的删除帧
/// （`orchestrator` 的消费循环）也用它——**同一条消息只有一个地址**，
/// 不能一处拼一种。
pub(crate) fn message_path(session_id: &str, mid: &str) -> String {
    format!("{session_id}/{SEG_MESSAGES}/{mid}")
}

/// 一条消息补丁 → VDFS 变更。
///
/// 规则见 `docs/design/vdfs-session-messages.md` §4：
///
/// | 补丁形状 | 变更 | 载荷 |
/// |---|---|---|
/// | 新 `id`（此前未出现） | `created` | 节点视图 + 内容快照 |
/// | 尾部追加了 `delta` | `appended` | **仅** `delta` |
/// | 其余（状态迁移 / 全量替换） | `updated` | 节点视图 + 内容快照 |
///
/// `existed` 与 `appended` 都来自实际合并结果（`orchestrator::merge_message_patch`
/// 的返回值），本函数**不做任何再判断**——它只把既成事实翻译成变更词汇。
/// 判据若在这里重算一遍，就有可能与合并方式不一致：合并按追加、这里判成替换，
/// 消费者按 `delta` 拼接就会得到错误内容。
///
/// `msg` 必须是**合并后的完整消息**（不是补丁）——载荷要能独立成立，
/// 消费者拿到它就不必再回读。
fn message_change(
    session_id: &str,
    msg: &cm::ChatMessage,
    existed: bool,
    appended: Option<String>,
) -> vdfs::VdfsChange {
    let path = message_path(session_id, &msg.id);
    if !existed {
        return message_payload(
            vdfs::VdfsChange::new(path, vdfs::VFDS_CHANGE_CREATED),
            session_id,
            msg,
        );
    }
    match appended.filter(|d| !d.is_empty()) {
        // 追加走**窄载荷**：这一路每帧都发，多挂一个字就是 O(n²)。
        Some(delta) => vdfs::VdfsChange::appended(path, delta),
        None => message_payload(
            vdfs::VdfsChange::new(path, vdfs::VFDS_CHANGE_UPDATED),
            session_id,
            msg,
        ),
    }
}

/// 给 `created` / `updated` 变更挂上**节点视图 + 内容快照**。
///
/// 这两类变更每轮只有寥寥数次，把「变成了什么」一并带上，消费者就不必
/// 为了填一条消息再跑 `stat` + `read` 两个来回——VDFS 承载会话转写因此
/// 不比既有的专用消息通道更贵。
///
/// `appended` **不走这里**：它每帧都发，载荷必须保持只有增量。
fn message_payload(
    change: vdfs::VdfsChange,
    session_id: &str,
    msg: &cm::ChatMessage,
) -> vdfs::VdfsChange {
    // 节点路径填成 provider 子树内相对路径，由分发层（`fs` / `composite` 的
    // watch 包装）经 `map_paths` 补成展示地址——与 `list` 返回的节点同口径。
    let mut node = message_node(msg);
    node.path = message_path(session_id, &msg.id);
    change.with_node(node).with_content(message_text(msg))
}

/// 会话挂载点内的路径解析结果
enum VdfsSessionPath<'a> {
    /// 挂载根 = 会话清单
    Root,
    /// `<id>`：单个会话（叶子）
    Session(&'a str),
    /// `<id>/消息[/<mid>]`：转写列表 / 单条消息；`mid` 空 = 列表本身
    Messages { id: &'a str, mid: Option<&'a str> },
    /// `<id>/子会话`：子会话清单
    SubSessions(&'a str),
    /// `<id>/子会话/<sub>`：单个子会话
    SubSession { id: &'a str, sub: &'a str },
    /// `<id>/工作目录[/<rel>]`：工作目录树；`rel` 空 = 工作目录根
    Workdir { id: &'a str, rel: &'a str },
}

/// 解析会话挂载点内的相对路径（首段 = 会话 id，次段 = 内部区段）。
fn parse_session_path(path: &str) -> vdfs::VdfsResult<VdfsSessionPath<'_>> {
    let p = path.trim_matches('/');
    if p.is_empty() {
        return Ok(VdfsSessionPath::Root);
    }
    let (id, rest) = match p.split_once('/') {
        Some((id, rest)) => (id, rest),
        None => return Ok(VdfsSessionPath::Session(p)),
    };
    let (seg, sub) = match rest.split_once('/') {
        Some((seg, sub)) => (seg, Some(sub)),
        None => (rest, None),
    };
    let not_found = || vdfs::VdfsError::not_found(format!("会话内部不存在该路径：{path}"));
    match seg {
        SEG_MESSAGES => match sub {
            None => Ok(VdfsSessionPath::Messages { id, mid: None }),
            Some(mid) if !mid.is_empty() && !mid.contains('/') => {
                Ok(VdfsSessionPath::Messages { id, mid: Some(mid) })
            }
            Some(_) => Err(vdfs::VdfsError::not_found(format!(
                "消息是列表项，没有更深层级：{path}"
            ))),
        },
        super::workdir::SEG_SUB_SESSIONS => match sub {
            None => Ok(VdfsSessionPath::SubSessions(id)),
            Some(sub) if !sub.is_empty() && !sub.contains('/') => {
                Ok(VdfsSessionPath::SubSession { id, sub })
            }
            Some(_) => Err(vdfs::VdfsError::not_found(format!(
                "子会话是叶子节点，没有更深层级：{path}"
            ))),
        },
        super::workdir::SEG_WORKDIR => Ok(VdfsSessionPath::Workdir {
            id,
            rel: sub.unwrap_or(""),
        }),
        _ => Err(not_found()),
    }
}

/// 会话内部的三个虚拟子目录（工作目录按会话是否声明 workdir 决定是否出现）
fn internal_dirs(has_workdir: bool) -> Vec<vdfs::VdfsNode> {
    let mut out = vec![
        vdfs::VdfsNode::dir(SEG_MESSAGES, SEG_MESSAGES, vdfs::VdfsAccess::LIST),
        vdfs::VdfsNode::dir(
            super::workdir::SEG_SUB_SESSIONS,
            super::workdir::SEG_SUB_SESSIONS,
            vdfs::VdfsAccess::LIST,
        ),
    ];
    if has_workdir {
        out.push(vdfs::VdfsNode::dir(
            super::workdir::SEG_WORKDIR,
            super::workdir::SEG_WORKDIR,
            vdfs::VdfsAccess::LIST,
        ));
    }
    out
}

// ==================== VDFS：转写列表项 ====================
//
// 呈现的分工只有一条：**正文进内容，结构进 `attributes`**。
// - `read` 取到的是这条消息的**正文**——流式追加的正是它，因此追加型变更的增量
//   可以直接拼在尾部，消费者无需为每个片段重读整条消息；
// - 角色 / 类型 / 状态 / 顺序 / 归属等**结构字段**放 `attributes`——它们是场景
//   数据，VDFS 只透传，渲染器按 `ext = message` 自行取用。
//
// 消息在 VDFS 上是**只读列表项**：发言由聊天协议承载（一次发言触发一整轮编排，
// 不是一次文件写入），VDFS 只做「读同一份数据」，因此不存在两条写路径。

/// 消息节点的标题：角色（工具调用补上工具名，否则一屏全是「助手」）
fn message_label(m: &cm::ChatMessage) -> String {
    let role = match m.role {
        Some(cm::MessageRole::User) => "用户",
        Some(cm::MessageRole::Assistant) => "助手",
        Some(cm::MessageRole::Tool) => "工具",
        Some(cm::MessageRole::System) => "系统",
        None => "消息",
    };
    match (m.msg_type.as_ref(), m.name.as_deref()) {
        (Some(cm::MessageType::ToolCall), Some(name)) if !name.is_empty() => {
            format!("{role} · {name}")
        }
        (Some(cm::MessageType::ToolCall), _) => format!("{role} · 工具调用"),
        _ => role.to_string(),
    }
}

/// 消息状态词——与 `MessageStatus` 的序列化名一致（前端按同一套词渲染角标）。
///
/// `completed` 与「未标注」都落到 VDFS 的常规状态词 `active`：节点状态只有一套
/// 词汇表（`VFDS_STATUS_*`），不为场景再造一套。
fn message_status(m: &cm::ChatMessage) -> &'static str {
    match m.status.as_ref() {
        Some(cm::MessageStatus::Pending) => "pending",
        Some(cm::MessageStatus::Streaming) => "streaming",
        Some(cm::MessageStatus::WaitingUserAction) => "waiting_user_action",
        Some(cm::MessageStatus::Failed) => "failed",
        _ => vdfs::VFDS_STATUS_ACTIVE,
    }
}

/// 消息摘要（首行、压空白、限长）——列表里的一行预览
fn message_preview(m: &cm::ChatMessage) -> Option<String> {
    let text = m.content.as_ref().map(|c| c.to_text()).unwrap_or_default();
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.is_empty() {
        return None;
    }
    const MAX: usize = 60;
    if flat.chars().count() <= MAX {
        return Some(flat);
    }
    Some(format!("{}…", flat.chars().take(MAX).collect::<String>()))
}

/// 单条消息 → VDFS 节点（**列表项**）
fn message_node(m: &cm::ChatMessage) -> vdfs::VdfsNode {
    let mut n = vdfs::VdfsNode::file(&m.id, message_label(m), vdfs::VdfsAccess::READ);
    n.ext = Some(vdfs::VFDS_EXT_MESSAGE.to_string());
    n.status = message_status(m).to_string();
    n.updated_at = m.timestamp;
    n.description = message_preview(m);
    for (k, v) in [
        ("role", json!(m.role)),
        ("type", json!(m.msg_type)),
        ("parent_id", json!(m.parent_id)),
        ("seq", json!(m.seq)),
        ("error", json!(m.error)),
    ] {
        let _ = n.attributes.insert(k.to_string(), v);
    }
    let _ = n
        .attributes
        .insert("meta".to_string(), m.meta.clone().unwrap_or(Value::Null));
    n
}

/// 消息正文——**流式追加的正是它**。
///
/// - `Text` / `Reasoning` / `UserPrompt`：纯文本正文；
/// - `Turn` / `ToolCall`（组合节点，本身无正文）：给一份稳定的 JSON 视图，
///   使前端与 LLM 在同一个地址上都能取到完整结构。
fn message_text(m: &cm::ChatMessage) -> String {
    match m.msg_type {
        Some(cm::MessageType::Turn) | Some(cm::MessageType::ToolCall) => {
            serde_json::to_string_pretty(m).unwrap_or_default()
        }
        _ => m.content.as_ref().map(|c| c.to_text()).unwrap_or_default(),
    }
}

/// 转写按 `seq` 升序——`seq` 是唯一权威顺序锚点。
///
/// 稳定排序：缺 `seq` 的（本轮**在途**消息——存储尚未写入、因而还没分配 `seq`）
/// 排在最后并保持原有相对顺序，恰好落在「最新的消息在末尾」，不会被排到历史之前。
fn ordered(mut msgs: Vec<cm::ChatMessage>) -> Vec<cm::ChatMessage> {
    msgs.sort_by_key(|m| m.seq.unwrap_or(i64::MAX));
    msgs
}

/// 把本轮**在途**消息叠加到落库转写上。
///
/// 同 id 时在途版本胜出（它是更新的那一份），但 `seq` 例外——顺序锚点只由存储
/// 在写入时分配，在途副本没有，必须从落库版本继承。否则同一条消息会以
/// 「有 seq / 无 seq」两种形态被排到列表的两个位置。
fn overlay_live(stored: Vec<cm::ChatMessage>, live: Vec<cm::ChatMessage>) -> Vec<cm::ChatMessage> {
    let mut out = stored;
    for l in live {
        match out.iter_mut().find(|m| m.id == l.id) {
            Some(s) => {
                let seq = s.seq;
                *s = l;
                s.seq = seq;
            }
            None => out.push(l),
        }
    }
    out
}

/// 按 id 取单条消息
fn message_of<'a>(msgs: &'a [cm::ChatMessage], mid: &str) -> vdfs::VdfsResult<&'a cm::ChatMessage> {
    msgs.iter()
        .find(|m| m.id == mid)
        .ok_or_else(|| vdfs::VdfsError::not_found(format!("消息不存在：{mid}")))
}

/// 会话内容（转写全文 + 元数据）→ VDFS 文本内容
fn session_content(session: &super::types::Session) -> vdfs::VdfsResult<vdfs::VdfsContent> {
    let payload = json!({
        "id": session.id,
        "title": session.display_title(),
        "metadata": session.metadata,
        "messages": session.messages,
        "updated_at": session.updated_at,
    });
    let text = serde_json::to_string_pretty(&payload)
        .map_err(|e| vdfs::VdfsError::internal(format!("会话序列化失败：{e}")))?;
    Ok(vdfs::VdfsContent::text("", text).with_mime("application/json"))
}

/// 新建会话的标题：路径名去掉扩展名（`<标题>.session` → `<标题>`）。
///
/// 新建时使用方给出的是**标题**而非会话 id——id 是存储细节，由 provider 生成
/// （见 [`vdfs::VdfsProvider::write`] 的 `create` 分支），不属于使用方的知识。
fn title_from_new_path(path: &str) -> String {
    let base = path.rsplit('/').next().unwrap_or(path);
    let stem = base
        .strip_suffix(&format!(".{}", vdfs::VFDS_EXT_SESSION))
        .unwrap_or(base)
        .trim();
    if stem.is_empty() {
        "新对话".to_string()
    } else {
        stem.to_string()
    }
}

/// 当前时间（Unix 毫秒）——与 `session/update` 的时间戳口径一致
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

// ==================== 配置文档（`.vdfs/session/配置`） ====================

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
        ],
    )
}

#[async_trait]
impl vdfs::VdfsProvider for SessionPlugin {
    fn label(&self) -> Option<&str> {
        // 标签 / 顺序由本 provider 自持（实体注册表已随 VDFS 收敛下线）
        Some("会话")
    }

    fn description(&self) -> Option<&str> {
        Some("会话清单。每个会话是一份独立对话记录，可读 / 写 / 删；新建即创建一份新会话。")
    }

    fn order(&self) -> i32 {
        1
    }

    fn icon(&self) -> Option<&str> {
        Some("session")
    }

    /// 会话是叶子文档：可列，不参与树遍历（会话内部的子结构另有容器语义）
    fn root_access(&self) -> vdfs::VdfsAccess {
        vdfs::VdfsAccess::LIST
    }

    /// 根下可新建「会话」（新建语义由 provider 自持，见 `write`）
    fn root_new_types(&self) -> Vec<vdfs::VdfsNewType> {
        vec![vdfs::VdfsNewType::new(vdfs::VFDS_EXT_SESSION, "会话").with_description("新建会话")]
    }

    async fn list(
        &self,
        _ctx: &vdfs::VdfsContext,
        path: &str,
    ) -> vdfs::VdfsResult<Vec<vdfs::VdfsNode>> {
        // 保留段优先：`配置` 是配置文档，不是会话
        if config::is_config_path(path) {
            return Err(vdfs::VdfsError::not_found(format!(
                "配置是文档，没有子项：{path}"
            )));
        }
        match parse_session_path(path)? {
            VdfsSessionPath::Root => {
                let sessions = self
                    .list_sessions()
                    .await
                    .map_err(vdfs::from_plugin_error)?;
                let mut nodes = self.nodes_of_sessions(&sessions).await;
                // 本插件的配置文档与资源并列（保留段，排在资源之后）
                nodes.push(self.config_doc.node());
                Ok(nodes)
            }
            // 会话内部：三个虚拟子目录（会话存在性校验由 `session_of` 承担）
            VdfsSessionPath::Session(id) => {
                let session = self.session_of(id).await?;
                Ok(internal_dirs(
                    super::workdir::workdir_of(&session).is_some(),
                ))
            }
            // 转写列表：每一条消息是一个列表项，顺序由 `seq` 决定。
            // 含**在途**消息——流式期间列表就是活的，不必等落库。
            VdfsSessionPath::Messages { id, mid: None } => {
                let msgs = self.transcript_of(id).await?;
                Ok(msgs.iter().map(message_node).collect())
            }
            VdfsSessionPath::Messages { mid: Some(_), .. } => Err(vdfs::VdfsError::not_found(
                format!("消息是列表项，没有子项：{path}"),
            )),
            VdfsSessionPath::SubSessions(id) => {
                self.session_of(id).await?;
                let store = self.get_store().await.map_err(vdfs::from_plugin_error)?;
                let subs = store
                    .list_sub_sessions(id)
                    .await
                    .map_err(vdfs::from_plugin_error)?;
                Ok(self.nodes_of_sessions(&subs).await)
            }
            VdfsSessionPath::SubSession { .. } => Err(vdfs::VdfsError::not_found(format!(
                "子会话是叶子节点，没有子项：{path}"
            ))),
            VdfsSessionPath::Workdir { id, rel } => {
                let workdir = self.workdir_of(id).await?;
                // 目录树场景的实时监听（按会话 id 引用计数）
                self.workdir_watches.ensure_watch(&workdir, id);
                super::workdir::list_children(&workdir, Some(rel))
                    .await
                    .map_err(vdfs::from_plugin_error)
            }
        }
    }

    async fn stat(&self, _ctx: &vdfs::VdfsContext, path: &str) -> vdfs::VdfsResult<vdfs::VdfsNode> {
        // 保留段优先：`配置` 是配置文档，不是会话
        if config::is_config_path(path) {
            return Ok(self.config_doc.node());
        }
        match parse_session_path(path)? {
            VdfsSessionPath::Root => {
                // 自身根：**名字留空**——provider 不知道自己的挂载名，由使用方回填
                Ok(vdfs::VdfsNode::dir("", "会话", self.root_access()))
            }
            // `<id>` 有二重身份：清单里是**叶子**（`session_node`，`rw`，
            // 点开 = 聊天详情），被当作目录访问时则是**会话内部**的目录视图。
            // 这里按后者回答——`stat` 的结果由分发层用作「当前目录节点」，
            // 其访问位直接决定页面是否给出新建入口；会话内部不支持
            // 新建 / 建目录，故只声明 `l`。
            VdfsSessionPath::Session(id) => {
                let session = self.session_of(id).await?;
                let mut n =
                    vdfs::VdfsNode::dir(id, session.display_title(), vdfs::VdfsAccess::LIST);
                n.kind = PLUGIN_SESSION.to_string();
                Ok(n)
            }
            VdfsSessionPath::Messages { id, mid } => match mid {
                // 列表本身是个目录（`l` 位：可列，不参与树遍历——会话整体是叶子）。
                // 只需存在性校验，不必取全量转写。
                None => {
                    self.session_of(id).await?;
                    Ok(vdfs::VdfsNode::dir(
                        SEG_MESSAGES,
                        SEG_MESSAGES,
                        vdfs::VdfsAccess::LIST,
                    ))
                }
                Some(mid) => Ok(message_node(message_of(
                    &self.transcript_of(id).await?,
                    mid,
                )?)),
            },
            VdfsSessionPath::SubSessions(id) => {
                self.session_of(id).await?;
                Ok(vdfs::VdfsNode::dir(
                    super::workdir::SEG_SUB_SESSIONS,
                    super::workdir::SEG_SUB_SESSIONS,
                    vdfs::VdfsAccess::LIST,
                ))
            }
            VdfsSessionPath::SubSession { id, sub } => {
                let session = self.sub_session_of(id, sub).await?;
                Ok(session_node(&session, self.is_working(sub).await))
            }
            VdfsSessionPath::Workdir { id, rel } => {
                let workdir = self.workdir_of(id).await?;
                if rel.is_empty() {
                    return Ok(vdfs::VdfsNode::dir(
                        super::workdir::SEG_WORKDIR,
                        super::workdir::SEG_WORKDIR,
                        vdfs::VdfsAccess::LIST,
                    ));
                }
                super::workdir::read_node(&workdir, rel)
                    .await
                    .map_err(vdfs::from_plugin_error)
            }
        }
    }

    /// 读取会话内容（转写全文 + 元数据，JSON）——转发既有存储，不新造协议
    async fn read(
        &self,
        _ctx: &vdfs::VdfsContext,
        path: &str,
    ) -> vdfs::VdfsResult<vdfs::VdfsContent> {
        // 保留段优先：`配置` 是配置文档，不是会话
        if config::is_config_path(path) {
            return self.config_doc.read(&self.config).await;
        }
        match parse_session_path(path)? {
            VdfsSessionPath::Session(id) => {
                let session = self.session_of(id).await?;
                session_content(&session)
            }
            // 单条消息的正文（列表项内容）——流式追加的正是它。
            // 同样取含在途的转写：追加型变更的消费者读到的必须是**已含该增量**的正文。
            VdfsSessionPath::Messages { id, mid: Some(mid) } => {
                let msgs = self.transcript_of(id).await?;
                Ok(vdfs::VdfsContent::text(
                    "",
                    message_text(message_of(&msgs, mid)?),
                ))
            }
            VdfsSessionPath::SubSession { id, sub } => {
                let session = self.sub_session_of(id, sub).await?;
                session_content(&session)
            }
            VdfsSessionPath::Workdir { id, rel } if !rel.is_empty() => {
                let workdir = self.workdir_of(id).await?;
                let text = super::workdir::read_content(&workdir, rel)
                    .await
                    .map_err(vdfs::from_plugin_error)?;
                Ok(vdfs::VdfsContent::text("", text))
            }
            // 目录（挂载根 / 会话内部区段 / 工作目录子目录）无「内容」语义
            _ => Err(vdfs::VdfsError::invalid(format!(
                "该路径不可读取内容：{path}"
            ))),
        }
    }

    /// 写入：`create` → 新建会话；否则 → 合并会话 metadata（`session/update` 语义）。
    ///
    /// **转写列表（`<id>/消息`）只读**——发言不是一次文件写入，而是一次**动作**
    /// （触发一整轮编排：模型调用 → 工具执行 → 流式落库），因此入口仍是聊天协议。
    /// 这不是「两套写路径」：VDFS 侧根本没有消息的写路径，只有读路径——
    /// 前端与 LLM 在**同一个地址**上读同一份数据，写入的唯一入口依旧只有一处。
    async fn write(
        &self,
        ctx: &vdfs::VdfsContext,
        path: &str,
        content: &vdfs::VdfsContent,
    ) -> vdfs::VdfsResult<vdfs::VdfsWriteResponse> {
        // 保留段优先：`配置` 是配置文档，不是会话
        if config::is_config_path(path) {
            return self.config_doc.apply(ctx, &self.config, content).await;
        }
        // 工作目录分支：文件写回（与列读同一份实现——路径越界校验 + 落盘）
        match parse_session_path(path)? {
            VdfsSessionPath::Workdir { id, rel } if !rel.is_empty() => {
                let workdir = self.workdir_of(id).await?;
                let text = content.text.as_deref().unwrap_or("");
                super::workdir::write_node(&workdir, rel, text)
                    .await
                    .map_err(vdfs::from_plugin_error)?;
                self.workdir_watches.ensure_watch(&workdir, id);
                return Ok(vdfs::VdfsWriteResponse {
                    path: path.to_string(),
                    created: false,
                    etag: None,
                });
            }
            // 会话本身（`path` 即会话 id；新建时是 `<标题>.session`）
            VdfsSessionPath::Session(_) => {}
            VdfsSessionPath::Messages { .. } => {
                return Err(vdfs::VdfsError::Forbidden(
                    "转写列表只读：发言请走聊天协议（一次发言触发一整轮编排）".to_string(),
                ));
            }
            _ => return Err(vdfs::VdfsError::invalid(format!("该路径不可写入：{path}"))),
        }

        let text = content.text.as_deref().unwrap_or("");
        let value: Value = if text.trim().is_empty() {
            json!({})
        } else {
            serde_json::from_str(text)
                .map_err(|e| vdfs::VdfsError::invalid(format!("会话写入需要合法 JSON：{e}")))?
        };
        let obj = value
            .as_object()
            .ok_or_else(|| vdfs::VdfsError::invalid("会话写入需要 JSON 对象"))?;

        // 新建：id 由 provider 生成，路径名（去扩展名）作标题
        if content.create {
            let title = title_from_new_path(path);
            let id = uuid::Uuid::new_v4().to_string();
            let mut session = Session::new(&id);
            session.metadata = json!({ "title": title, "created_via": "vdfs" });
            session.updated_at = now_ms();
            self.save_session(&session)
                .await
                .map_err(vdfs::from_plugin_error)?;
            self.notify_change(&id, vdfs::VFDS_CHANGE_CREATED);
            return Ok(vdfs::VdfsWriteResponse {
                path: id,
                created: true,
                etag: None,
            });
        }

        // 覆盖：只接受 metadata / title 两类字段（其余字段无写入语义，明确拒绝，
        // 不静默丢弃使用方的意图）
        let has_title = obj.contains_key("title");
        let has_meta = obj.contains_key("metadata");
        if !has_title && !has_meta {
            return Err(vdfs::VdfsError::invalid(
                "会话写入支持 metadata / title 字段；消息请走聊天协议",
            ));
        }
        let mut session = self.session_of(path).await?;
        if let Some(incoming) = obj.get("metadata").and_then(Value::as_object) {
            let target = session
                .metadata
                .as_object_mut()
                .ok_or_else(|| vdfs::VdfsError::invalid("会话 metadata 不是对象"))?;
            for (k, v) in incoming {
                target.insert(k.clone(), v.clone());
            }
        }
        if let Some(title) = obj.get("title").and_then(Value::as_str) {
            if let Some(m) = session.metadata.as_object_mut() {
                m.insert("title".to_string(), Value::String(title.to_string()));
            }
        }
        session.updated_at = now_ms();
        self.save_session(&session)
            .await
            .map_err(vdfs::from_plugin_error)?;
        self.notify_change(path, vdfs::VFDS_CHANGE_UPDATED);
        Ok(vdfs::VdfsWriteResponse {
            path: path.to_string(),
            created: false,
            etag: None,
        })
    }

    async fn delete(
        &self,
        _ctx: &vdfs::VdfsContext,
        path: &str,
        _recursive: bool,
    ) -> vdfs::VdfsResult<()> {
        // 保留段优先：配置文档恒在，不可删
        if config::is_config_path(path) {
            return Err(vdfs::VdfsError::Forbidden(
                "配置文档不可删除".to_string(),
            ));
        }
        match parse_session_path(path)? {
            VdfsSessionPath::Root => {
                Err(vdfs::VdfsError::Forbidden("会话挂载根不可删除".to_string()))
            }
            VdfsSessionPath::Session(id) => {
                // 存在性校验：删除不存在的会话应报 NotFound 而非静默成功
                self.session_of(id).await?;
                self.delete_session_internal(id)
                    .await
                    .map_err(vdfs::from_plugin_error)
            }
            // 子会话：先 abort 活跃任务再删（`delete_session_internal` 同语义）
            VdfsSessionPath::SubSession { id, sub } => {
                self.sub_session_of(id, sub).await?;
                self.delete_session_internal(sub)
                    .await
                    .map_err(vdfs::from_plugin_error)
            }
            VdfsSessionPath::Workdir { id, rel } if !rel.is_empty() => {
                let workdir = self.workdir_of(id).await?;
                super::workdir::delete_node(&workdir, rel)
                    .await
                    .map_err(vdfs::from_plugin_error)
            }
            _ => Err(vdfs::VdfsError::Forbidden(format!(
                "该路径不可删除：{path}"
            ))),
        }
    }

    /// 订阅：把本插件内部的变更广播转发到 sink（`unwatch` 时取消任务）。
    ///
    /// provider 是**变更源的持有者**，因此这里不需要轮询——写入 / 删除路径
    /// 直接广播（见 [`SessionPlugin::notify_change`]）。
    ///
    /// ## 路径不在这里收敛（容易看错，特此写明）
    ///
    /// 广播源的路径是 **provider 根口径**（与 `list` / `stat` 同一坐标系：
    /// `<id>`、`<id>/消息/<mid>`），本方法**原样转发**、不做任何前缀处理。
    ///
    /// 看起来「应该」把它收敛成相对被订阅 `path` 的路径，但那样反而会错：
    /// 容器的 `watch` 包装器（`CompositeVdfs::watch`）只补**挂载名**（首段），
    /// 不是补被订阅的整条路径——它把 `(dir, rel)` 拆开，`rel` 传给了 provider，
    /// 回填时只加 `dir`。因此 provider 报出的路径必须仍是 provider 根口径，
    /// 容器补上挂载名后才是树内全路径：
    ///
    /// ```text
    /// watch("session/abc/消息") → dir = "session", rel = "abc/消息"
    /// provider 报 "abc/消息/m1" → 容器补成 "session/abc/消息/m1"  ✓
    /// 若这里先剥成 "m1"        → 容器补成 "session/m1"          ✗
    /// ```
    ///
    /// 代价是订阅者会收到兄弟子树的变更（订阅一个会话会看到别的会话的事件）；
    /// 消费者按路径前缀自行过滤即可，正确性不受影响。
    async fn watch(
        &self,
        _ctx: &vdfs::VdfsContext,
        path: &str,
        sink: vdfs::VdfsChangeSink,
    ) -> vdfs::VdfsResult<()> {
        // 工作目录子树：接入文件系统监听（文件变化经**同一广播源**到达本 sink，
        // 见 `SessionPlugin::new` 注入的 `set_vdfs_sender`）
        if let Ok(VdfsSessionPath::Workdir { id, .. }) = parse_session_path(path) {
            if let Ok(workdir) = self.workdir_of(id).await {
                self.workdir_watches.ensure_watch(&workdir, id);
            }
        }
        let mut rx = self.change_tx.subscribe();
        let handle = tokio::spawn(async move {
            while let Ok(change) = rx.recv().await {
                sink(change);
            }
        });
        // 同路径重复订阅：覆盖并取消旧任务（机制保证 watch/unwatch 严格配对，
        // 此处仅作防御）
        let old = self
            .watch_tasks
            .lock()
            .await
            .insert(path.to_string(), handle);
        if let Some(old) = old {
            old.abort();
        }
        Ok(())
    }

    async fn unwatch(&self, _ctx: &vdfs::VdfsContext, path: &str) -> vdfs::VdfsResult<()> {
        // 与 `watch` 严格配对：释放本会话对该工作目录的订阅（引用计数归零才停监听）
        if let Ok(VdfsSessionPath::Workdir { id, .. }) = parse_session_path(path) {
            if let Ok(workdir) = self.workdir_of(id).await {
                self.workdir_watches.release_watch(&workdir, id);
            }
        }
        let handle = self.watch_tasks.lock().await.remove(path);
        if let Some(handle) = handle {
            handle.abort();
        }
        Ok(())
    }
}

impl SessionPlugin {
    /// 会话转写（**含在途消息**）——`.vdfs/session/<id>/消息` 的唯一数据源。
    ///
    /// 落库转写 ∪ 本轮在途缓冲。之所以要并上后者：流式期间消息**还没落库**
    /// （`persist_messages` 只在每轮结束时写盘），只读存储的话列表在流式期间
    /// 就是空的——`created` / `appended` 变更到达时消费者去 `list` 会一无所获，
    /// 「转写即列表」当场失效。
    ///
    /// 存在性校验在此一并完成（`session_of`），因此调用方不必再查一次。
    async fn transcript_of(&self, id: &str) -> vdfs::VdfsResult<Vec<cm::ChatMessage>> {
        let session = self.session_of(id).await?;
        let live = self.live_messages_of(id).await;
        Ok(ordered(overlay_live(session.messages, live)))
    }

    /// 该会话本轮的在途消息（无活跃状态 ⇒ 无在途消息）
    async fn live_messages_of(&self, id: &str) -> Vec<cm::ChatMessage> {
        let active = self.active_mgr.sessions.read().await;
        match active.get(id) {
            Some(st) => st.live_messages.lock().await.clone(),
            None => Vec::new(),
        }
    }

    /// 按 id 取会话（**存在性校验**：`load_session` 对未命中会返回空会话，
    /// 因此这里走带存在性判据的 `load_session_checked`）
    async fn session_of(&self, id: &str) -> vdfs::VdfsResult<Session> {
        self.get_store()
            .await
            .map_err(vdfs::from_plugin_error)?
            .load_session_checked(id)
            .await
            .map_err(vdfs::from_plugin_error)?
            .ok_or_else(|| vdfs::VdfsError::not_found(format!("会话不存在：{id}")))
    }

    /// 单个会话的实时工作状态
    async fn is_working(&self, id: &str) -> bool {
        let active = self.active_mgr.sessions.read().await;
        active
            .get(id)
            .map(|st| st.inner.try_read().map(|i| i.is_working).unwrap_or(false))
            .unwrap_or(false)
    }

    /// 会话清单 → VDFS 节点（携带实时工作状态）
    async fn nodes_of_sessions(&self, sessions: &[super::types::Session]) -> Vec<vdfs::VdfsNode> {
        let active = self.active_mgr.sessions.read().await;
        sessions
            .iter()
            .map(|s| {
                let is_working = active
                    .get(&s.id)
                    .map(|st| st.inner.try_read().map(|i| i.is_working).unwrap_or(false))
                    .unwrap_or(false);
                session_node(s, is_working)
            })
            .collect()
    }

    /// 会话的工作目录（未声明 workdir ⇒ 该会话无目录树能力）
    async fn workdir_of(&self, id: &str) -> vdfs::VdfsResult<String> {
        let session = self.session_of(id).await?;
        super::workdir::workdir_of(&session)
            .ok_or_else(|| vdfs::VdfsError::not_found(format!("会话 {id} 没有工作目录")))
    }

    /// 取子会话（**归属校验**：必须确实挂在 `id` 之下，避免跨会话越权访问）
    async fn sub_session_of(&self, id: &str, sub: &str) -> vdfs::VdfsResult<super::types::Session> {
        let store = self.get_store().await.map_err(vdfs::from_plugin_error)?;
        let session = store
            .load_session(sub)
            .await
            .map_err(vdfs::from_plugin_error)?;
        if session.id == sub && session.parent_session_id() == Some(id) {
            Ok(session)
        } else {
            Err(vdfs::VdfsError::not_found(format!(
                "会话 {id} 下不存在子会话 {sub}"
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // trait 方法（list/stat/read/write/delete/watch/unwatch）需 trait 在作用域内才可解析
    use crate::symbio_core::vdfs_provider::VdfsProvider;
    // 目录树场景模块（本文件非测试码用 `super::workdir`；测试模块需显式引入）
    use crate::plugins::session::workdir;

    /// 验证 session 存储目录**只**从 HomedirRegistry 派生，不依赖 config；
    /// 且它就是宿主层的资源类别根（`category_dir(PLUGIN_SESSION)`）——
    /// 会话因此不再手拼一份 `<homedir>/plugins/<类别>` 布局。
    #[test]
    fn test_session_storage_dir_from_homedir() {
        let dir = SessionPlugin::session_storage_dir();
        let expected = crate::symbio_core::HomedirRegistry::get()
            .join("plugins")
            .join("session");
        assert_eq!(
            dir, expected,
            "session_storage_dir 必须等于 <homedir>/plugins/session"
        );
        assert_eq!(
            dir,
            crate::providers::vdfs_service::entry::category_dir(PLUGIN_SESSION),
            "会话存储根必须与 VDFS 资源类别根同一条构造式"
        );
        assert!(
            dir.is_absolute(),
            "session_storage_dir 必须是绝对路径: {}",
            dir.display()
        );
    }

    /// 验证 SessionConfig 不再包含已删除的死字段：
    /// - `storage_dir`（存储根由 HomedirRegistry 统一决定）
    /// - `session_id`（零消费者；id 由会话目录名决定，配置内自指冗余）
    #[test]
    fn test_session_config_has_no_dead_fields() {
        let cfg = SessionConfig::default();
        let json = serde_json::to_value(&cfg).unwrap();
        for key in ["storage_dir", "session_id"] {
            assert!(
                json.get(key).is_none(),
                "SessionConfig 不应再包含 {key} 字段, got: {json}"
            );
        }
    }

    /// 验证从含旧字段的配置反序列化时，未知键被静默忽略（旧 session_config.json
    /// 无需迁移即可继续加载）。
    #[test]
    fn test_session_config_deserialize_ignores_legacy_keys() {
        let json = serde_json::json!({
            "storage_dir": "/tmp/should_be_ignored",
            "session_id": "stale-id-should-be-ignored",
            "max_messages": 42,
        });
        let cfg: SessionConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.max_messages, 42, "max_messages 应被正确反序列化");
    }

    // ==================== 配置契约一致性（复杂度审计 P0-1 / P1-①）====================

    /// `max_tool_rounds` 的契约翻译：0 → `None`（不限制），>0 → `Some(n)`（软上限）。
    /// 修复前编排层三处无条件 `Some(...)`，使 chat_loop 的 `None` = 无限语义在主路径不可达。
    #[test]
    fn max_tool_rounds_zero_maps_to_unlimited() {
        let cfg = SessionConfig {
            max_tool_rounds: 15,
            ..Default::default()
        };
        assert_eq!(
            cfg.model_chat_max_tool_rounds(),
            Some(15),
            "显式 >0 的值应作为软上限下发"
        );

        let cfg = SessionConfig {
            max_tool_rounds: 0,
            ..Default::default()
        };
        assert_eq!(
            cfg.model_chat_max_tool_rounds(),
            None,
            "0 必须翻译为 None（不限制），而非 Some(0)（0 轮即熔断）"
        );
    }

    /// 默认配置的产品意图锁定：默认**不设硬性轮次上限**，以显式语义 `0`（不限制）
    /// 表达，而非魔法数 65535（旧默认，行为等价但语义含糊）。
    #[test]
    fn default_max_tool_rounds_is_effectively_unlimited() {
        let cfg = SessionConfig::default();
        assert_eq!(
            cfg.max_tool_rounds, 0,
            "默认轮次上限必须是 0 = 不限制（用户明确要求不设硬上限）"
        );
        assert_eq!(
            cfg.model_chat_max_tool_rounds(),
            None,
            "默认配置翻译到 Request 层必须是 None（无限轮次）"
        );
    }

    // ==================== VDFS provider ====================

    fn vctx() -> vdfs::VdfsContext {
        vdfs::VdfsContext::empty()
    }

    /// provider 自描述：**不含挂载名**——挂载名由使用方在注册时选定
    /// （见 `traverse` 里的 `register_vdfs_provider(PLUGIN_SESSION, ..)`）
    #[tokio::test]
    async fn vdfs_self_description_has_no_mount() {
        let p = SessionPlugin::new(None, SessionConfig::default());
        assert_eq!(p.label(), Some("会话"));
        assert_eq!(p.icon(), Some("session"));
        // 顺序由本 provider 的 order() 自持（**单一真相源**），
        // 使 `.vdfs` 左栏与详情恒等
        assert_eq!(p.order(), 1);
        assert_eq!(p.root_access().flags(), "l");
        assert!(!p.root_access().traverse, "会话是叶子，不参与树遍历");

        // 根下可新建「会话」——类型清单即「新建」入口的唯一依据
        let types = p.root_new_types();
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].ext, vdfs::VFDS_EXT_SESSION);
        assert_eq!(types[0].title, "会话");
    }

    /// 新建标题：路径名去掉 `.session` 扩展名；空名回落「新对话」
    #[test]
    fn title_from_new_path_strips_session_ext() {
        assert_eq!(title_from_new_path("我的会话.session"), "我的会话");
        assert_eq!(title_from_new_path("dir/我的会话.session"), "我的会话");
        assert_eq!(title_from_new_path("未命名"), "未命名");
        assert_eq!(title_from_new_path(".session"), "新对话");
        assert_eq!(title_from_new_path(""), "新对话");
    }

    /// 会话节点：`ext = session`（前端据此选聊天工作区渲染器）、
    /// kind / 状态 / 更新时间 / 摘要与 `session_node` 的单点形状同源
    #[test]
    fn session_node_carries_renderer_ext_and_presentation() {
        let mut s = Session::new("abc");
        s.updated_at = 1_700_000_000_000;

        let idle = session_node(&s, false);
        assert_eq!(idle.name, "abc");
        assert_eq!(idle.effective_ext().as_deref(), Some("session"));
        assert_eq!(idle.kind, PLUGIN_SESSION);
        assert_eq!(idle.status, vdfs::VFDS_STATUS_ACTIVE);
        assert_eq!(idle.updated_at, Some(1_700_000_000_000));
        assert_eq!(idle.access.flags(), "rw", "会话可读可写");
        assert!(!idle.is_dir(), "会话是文档而非目录");

        let busy = session_node(&s, true);
        assert_eq!(busy.status, vdfs::VFDS_STATUS_WORKING);
    }

    /// 会话内部寻址（S6/S16）：`<id>` / `<id>/消息[/<mid>]` /
    /// `<id>/子会话[/<sub>]` / `<id>/工作目录[/<rel>]`。
    /// 未知区段与越界层级一律 NotFound——不给半通不通的路径留口子。
    #[test]
    fn vdfs_internal_path_parsing() {
        use VdfsSessionPath::*;
        assert!(matches!(parse_session_path("").unwrap(), Root));
        assert!(matches!(parse_session_path("/").unwrap(), Root));
        assert!(matches!(parse_session_path("abc").unwrap(), Session("abc")));
        // 转写列表：目录本身与列表项两级
        assert!(matches!(
            parse_session_path("abc/消息").unwrap(),
            Messages {
                id: "abc",
                mid: None
            }
        ));
        assert!(matches!(
            parse_session_path("abc/消息/m1").unwrap(),
            Messages {
                id: "abc",
                mid: Some("m1")
            }
        ));
        assert!(matches!(
            parse_session_path("abc/子会话").unwrap(),
            SubSessions("abc")
        ));
        assert!(matches!(
            parse_session_path("abc/子会话/s1").unwrap(),
            SubSession {
                id: "abc",
                sub: "s1"
            }
        ));
        assert!(matches!(
            parse_session_path("abc/工作目录").unwrap(),
            Workdir { id: "abc", rel: "" }
        ));
        assert!(matches!(
            parse_session_path("abc/工作目录/src/lib.rs").unwrap(),
            Workdir {
                id: "abc",
                rel: "src/lib.rs"
            }
        ));
        // 未知区段、列表项越界层级 → NotFound
        assert!(parse_session_path("abc/nope").is_err());
        assert!(parse_session_path("abc/消息/m1/deeper").is_err());
        assert!(parse_session_path("abc/子会话/s1/deeper").is_err());
    }

    /// 会话内部的三个虚拟子目录：转写列表恒在，工作目录按会话是否声明 workdir 出现
    #[test]
    fn vdfs_internal_dirs_conditional() {
        let without = internal_dirs(false);
        assert_eq!(without.len(), 2);
        assert_eq!(without[0].name, SEG_MESSAGES, "转写列表恒在（会话的本体）");
        assert_eq!(without[1].name, workdir::SEG_SUB_SESSIONS);
        assert!(without.iter().all(|n| n.is_dir()));

        let with = internal_dirs(true);
        assert_eq!(with.len(), 3);
        assert_eq!(with[2].name, workdir::SEG_WORKDIR);
        assert!(
            with.iter().all(|n| n.is_dir() && !n.access.write),
            "三个内部区段都是只读目录（工作目录不提供新建）"
        );
    }

    /// 转写列表项：`ext = message`、只读、正文进内容、结构进 `attributes`
    #[test]
    fn vdfs_message_node_splits_text_and_structure() {
        let mut m = cm::ChatMessage {
            id: "m1".into(),
            role: Some(cm::MessageRole::Assistant),
            msg_type: Some(cm::MessageType::Text),
            content: Some(cm::MessageContent::Text("你好，世界".into())),
            status: Some(cm::MessageStatus::Completed),
            seq: Some(7),
            timestamp: Some(1_700_000_000_000),
            ..Default::default()
        };
        let n = message_node(&m);
        assert_eq!(n.name, "m1");
        assert_eq!(n.effective_ext().as_deref(), Some("message"));
        assert_eq!(n.title, "助手");
        assert_eq!(n.description.as_deref(), Some("你好，世界"));
        assert_eq!(n.access.flags(), "r", "消息是只读列表项");
        assert_eq!(n.updated_at, Some(1_700_000_000_000));
        assert_eq!(n.status, vdfs::VFDS_STATUS_ACTIVE, "completed 落常规态");
        // 结构字段全在 attributes 里（VDFS 只透传）
        assert_eq!(n.attributes.get("seq"), Some(&json!(7)));
        assert_eq!(n.attributes.get("role"), Some(&json!("assistant")));
        assert_eq!(n.attributes.get("type"), Some(&json!("text")));

        // 正文即内容；流式追加的正是它
        assert_eq!(message_text(&m), "你好，世界");

        // 工具调用：标题补工具名、状态词直通、正文退化为 JSON 视图
        m.msg_type = Some(cm::MessageType::ToolCall);
        m.name = Some("vdfs_list".into());
        m.status = Some(cm::MessageStatus::Streaming);
        assert_eq!(message_node(&m).title, "助手 · vdfs_list");
        assert_eq!(message_node(&m).status, "streaming");
        assert!(message_text(&m).contains("vdfs_list"));

        // 失败态：错误原因也在 attributes 里（前端按 ext 自行取用）
        m.status = Some(cm::MessageStatus::Failed);
        m.error = Some("模型超时".into());
        assert_eq!(message_node(&m).status, "failed");
        assert_eq!(
            message_node(&m).attributes.get("error"),
            Some(&json!("模型超时"))
        );
    }

    fn msg(id: &str, seq: Option<i64>) -> cm::ChatMessage {
        cm::ChatMessage {
            id: id.into(),
            seq,
            ..Default::default()
        }
    }

    /// 列表顺序以 `seq` 为准（唯一权威顺序锚点）：缺 `seq` 的排最后且不打乱相对顺序
    #[test]
    fn vdfs_messages_are_ordered_by_seq() {
        let msgs = vec![
            msg("c", Some(30)),
            msg("x", None),
            msg("a", Some(10)),
            msg("y", None),
            msg("b", Some(20)),
        ];
        let ids: Vec<String> = ordered(msgs).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec!["a", "b", "c", "x", "y"]);
    }

    /// 在途消息叠加在落库转写之上：同 id 时**在途胜出，但继承落库的 `seq`**。
    ///
    /// 继承 `seq` 是关键：不继承的话同一条消息会以「有 seq / 无 seq」两种形态
    /// 被排到列表的两个位置（一份在历史里、一份在末尾）。
    #[test]
    fn overlay_live_keeps_seq_from_stored() {
        let mut stored = msg("m1", Some(5));
        stored.content = Some(cm::MessageContent::Text("落库正文".into()));
        stored.status = Some(cm::MessageStatus::Completed);
        let mut live = msg("m1", None);
        live.content = Some(cm::MessageContent::Text("落库正文 + 流式追加".into()));
        live.status = Some(cm::MessageStatus::Streaming);

        let merged = overlay_live(vec![stored], vec![live, msg("m2", None)]);
        let ids: Vec<String> = ordered(merged).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, vec!["m1", "m2"], "m1 继承 seq=5 仍在 m2（无 seq）之前");

        // 在途版本的内容与状态胜出
        let merged = overlay_live(
            vec![msg("m1", Some(5))],
            vec![{
                let mut l = msg("m1", None);
                l.content = Some(cm::MessageContent::Text("增量".into()));
                l
            }],
        );
        assert_eq!(merged.len(), 1, "同 id 不得出现两条");
        assert_eq!(merged[0].seq, Some(5), "顺序锚点由落库版本继承");
        assert_eq!(message_text(&merged[0]), "增量");
    }

    /// 补丁 → 变更的映射：新 id 是 `created`，有增量是 `appended`，其余 `updated`。
    ///
    /// 同时钉住**载荷宽度**：`created` / `updated` 带节点视图与内容快照（冷路径免回读），
    /// `appended` **只**带增量（热路径逐帧，多一个字段就是 O(n²)）。
    #[test]
    fn message_change_maps_patch_to_change_kind() {
        let mut patch = msg("m1", None);
        patch.content = Some(cm::MessageContent::Text("正文".into()));

        // 新 id
        let c = message_change("abc", &patch, false, None);
        assert_eq!(c.change, vdfs::VFDS_CHANGE_CREATED);
        assert_eq!(c.path, "abc/消息/m1", "变更地址与 list 的节点地址同源");
        assert!(c.delta.is_none());
        // 载荷：结构（attributes）与正文各自就位，消费者无需回读
        assert_eq!(
            c.node.as_ref().map(|n| n.path.as_str()),
            Some("abc/消息/m1"),
            "载荷节点的路径与事件路径同口径（分发层据此补前缀）"
        );
        assert_eq!(
            c.node.as_ref().and_then(|n| n.effective_ext()),
            Some(vdfs::VFDS_EXT_MESSAGE.to_string())
        );
        assert_eq!(c.content.as_deref(), Some("正文"));

        // 已有 id + 增量 → appended（只带 delta，不带节点视图 / 内容快照）
        let c = message_change("abc", &patch, true, Some("追加".into()));
        assert_eq!(c.change, vdfs::VFDS_CHANGE_APPENDED);
        assert_eq!(c.delta.as_deref(), Some("追加"));
        assert!(
            c.node.is_none() && c.content.is_none(),
            "追加是热路径：载荷必须保持只有增量"
        );

        // 已有 id、无增量（状态迁移 / 全量替换）→ updated（同样带全量载荷）
        let c = message_change("abc", &patch, true, None);
        assert_eq!(c.change, vdfs::VFDS_CHANGE_UPDATED);
        assert!(c.delta.is_none());
        assert!(c.node.is_some() && c.content.is_some());

        // 空增量不算追加（避免发一条什么都不带的 appended 让消费者空转）
        let c = message_change("abc", &patch, true, Some(String::new()));
        assert_eq!(c.change, vdfs::VFDS_CHANGE_UPDATED);
    }

    /// 地址的「拼」与「解」互逆——改地址方案时漏改一边会被这条挡住
    #[test]
    fn message_path_round_trips_through_parser() {
        let p = message_path("abc", "m1");
        assert_eq!(p, "abc/消息/m1");
        assert!(matches!(
            parse_session_path(&p).unwrap(),
            VdfsSessionPath::Messages {
                id: "abc",
                mid: Some("m1")
            }
        ));
    }

    // 工作目录节点的形状由 `workdir::tree_node` 单点保证（见其单测）：
    // 目录 `l`、文件 `rw`——本文件不再另设一份转换逻辑，因此无需重复断言。

    /// 会话节点自带清单字段（S8）：`message_count` / `metadata` / `meta_tags`
    /// 挂在 flatten 的 attributes 上，使会话清单无需再走一次额外的列接口
    #[test]
    fn vdfs_session_node_carries_list_fields() {
        let mut s = Session::new("abc");
        s.updated_at = 1_700_000_000;
        s.metadata = json!({ "workdir": "/tmp/proj/demo", "title": "T" });

        let n = session_node(&s, false);
        assert_eq!(n.attributes.get("message_count"), Some(&json!(0)));
        assert_eq!(
            n.attributes.get("metadata").and_then(|v| v.get("workdir")),
            Some(&json!("/tmp/proj/demo"))
        );
        let tags = n
            .attributes
            .get("meta_tags")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        assert_eq!(tags.len(), 2, "工作目录名 + 消息数");
        assert_eq!(tags[0], json!("demo"), "标签取工作目录 basename");
        assert_eq!(tags[1], json!("0 条"));

        // is_working 仍由 status 承载（机制口径，不另设 is_working 字段）
        assert_eq!(n.status, vdfs::VFDS_STATUS_ACTIVE);
        assert_eq!(session_node(&s, true).status, vdfs::VFDS_STATUS_WORKING);
    }

    /// 会话内容（VDFS `read`）：转写全文 + 元数据，JSON
    #[test]
    fn vdfs_session_content_is_json() {
        let mut s = Session::new("abc");
        s.updated_at = 1_700_000_000_000;
        let c = session_content(&s).unwrap();
        let text = c.text.as_deref().unwrap_or("");
        let v: Value = serde_json::from_str(text).unwrap();
        assert_eq!(v["id"], "abc");
        assert!(v.get("messages").is_some(), "聊天转写随内容下发");
    }

    /// 不存在的会话：list / stat 一律 NotFound（不做静默降级）
    #[tokio::test]
    async fn vdfs_list_unknown_session_is_not_found() {
        let p = SessionPlugin::new(None, SessionConfig::default());
        assert!(p.list(&vctx(), "abc").await.is_err());
        assert!(p.stat(&vctx(), "abc").await.is_err());
    }

    /// 实时：provider 自持的变更广播经 `watch` 的转发任务到达 sink；
    /// `unwatch` 取消任务（严格配对）
    #[tokio::test]
    async fn vdfs_watch_forwards_session_changes() {
        let p = SessionPlugin::new(None, SessionConfig::default());
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<vdfs::VdfsChange>();
        let sink: vdfs::VdfsChangeSink = Arc::new(move |c| {
            let _ = tx.send(c);
        });

        p.watch(&vctx(), "", sink).await.unwrap();
        p.notify_change("abc", vdfs::VFDS_CHANGE_CREATED);

        // 转发任务与本测试同 runtime：让出一次即可收到
        let got = rx.recv().await.expect("变更应经 watch 转发到 sink");
        assert_eq!(got.path, "abc");
        assert_eq!(got.change, vdfs::VFDS_CHANGE_CREATED);

        p.unwatch(&vctx(), "").await.unwrap();
        assert!(
            p.watch_tasks.lock().await.is_empty(),
            "unwatch 必须移除任务"
        );
    }

    // ==================== 配置文档（`.vdfs/session/配置`） ====================

    /// **定义与配置同源**：面板字段的默认值一律来自 `SessionConfig::default()`，
    /// 且 serde 默认值函数与 `Default` impl 不漂移（两处各自书写必然漂移）。
    #[test]
    fn config_definition_defaults_come_from_session_config() {
        let defaults = serde_json::to_value(SessionConfig::default()).unwrap();
        let def = config_definition();
        let fields = &def.sections[0].fields;
        assert!(!fields.is_empty(), "会话配置必须有字段");
        for f in fields {
            let declared = f
                .default
                .clone()
                .unwrap_or_else(|| panic!("字段 {} 缺少 default", f.key));
            let actual = defaults
                .get(&f.key)
                .unwrap_or_else(|| panic!("SessionConfig 不存在字段 {}", f.key));
            assert_eq!(
                &declared, actual,
                "面板 {}.default={declared} 与 SessionConfig::default().{}={actual} 不一致",
                f.key, f.key
            );
        }

        let from_empty: SessionConfig =
            serde_json::from_str("{}").expect("空对象应能反序列化出默认配置");
        assert_eq!(
            serde_json::to_value(&from_empty).unwrap(),
            defaults,
            "SessionConfig 的 serde 默认值与 Default impl 漂移了（两处各自书写）"
        );
    }

    /// 配置文档与资源并列在根下（保留段排在资源之后），且是 `ext = form` 的可写文档
    #[tokio::test]
    async fn config_document_sits_beside_the_sessions() {
        let p = SessionPlugin::new(None, SessionConfig::default());
        let items = p.list(&vctx(), "").await.unwrap();
        let last = items.last().expect("根下至少应有配置文档");
        assert_eq!(last.name, config::SEG_CONFIG);
        assert_eq!(last.ext.as_deref(), Some(vdfs::VFDS_EXT_FORM));
        assert_eq!(last.access.flags(), "rw");
        assert!(last.schema.is_some(), "定义随节点下发");

        // `配置` 是保留段：不会被当成会话 id
        assert_eq!(p.stat(&vctx(), config::SEG_CONFIG).await.unwrap().name, config::SEG_CONFIG);
        let content = p.read(&vctx(), config::SEG_CONFIG).await.unwrap();
        let cfg: SessionConfig = serde_json::from_str(content.text.as_deref().unwrap()).unwrap();
        assert_eq!(cfg.max_messages, SessionConfig::default().max_messages);

        // 文档没有子项，也不可删除
        assert!(p.list(&vctx(), config::SEG_CONFIG).await.is_err());
        assert!(p.delete(&vctx(), config::SEG_CONFIG, false).await.is_err());
    }

    /// 配置写入：校验先于一切（字段级错误），坏值不会改动内存
    #[tokio::test]
    async fn config_write_validates_before_applying() {
        let p = SessionPlugin::new(None, SessionConfig::default());
        let before = p.config.read().await.max_messages;
        let bad = vdfs::VdfsContent::text("", r#"{"max_messages": 1}"#);
        match p.write(&vctx(), config::SEG_CONFIG, &bad).await {
            Err(vdfs::VdfsError::Invalid(v)) => assert_eq!(v.fields[0].field, "max_messages"),
            other => panic!("应为字段级校验错误，实得 {other:?}"),
        }
        assert_eq!(p.config.read().await.max_messages, before);
    }
}
