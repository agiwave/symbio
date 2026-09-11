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

use super::types::Session;
use crate::symbio_core::schemas::options::OPTIONS_LIST;
use crate::symbio_core::schemas::session::chat_message as cm;
pub use crate::symbio_core::schemas::session::session_config::SessionConfig;
use crate::symbio_core::{
    ChatSession, HomedirRegistry, InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin,
    PluginError, PluginMeta, PluginPayload, CONFIG_GET, CONFIG_SET, PLUGIN_SESSION,
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
    /// 父插件引用（用于获取工具列表等）
    pub(crate) parent: Option<Weak<dyn Plugin>>,
    /// 活跃会话管理器 (V2 整合版：处理长连接与广播)
    pub(crate) active_mgr: Arc<super::active::ActiveSessionManager>,
    /// 存储后端单例（全局共享，初始化一次后跨 workdir / 跨会话复用）
    pub(crate) store: OnceCell<Arc<dyn SessionStore>>,
    /// 心跳任务运行时状态：会话 id -> 最近一次"有效活动"时间戳（毫秒）。
    /// 调度器据此判断会话是否已空闲足够久。
    pub(crate) heartbeat_state: Arc<RwLock<HashMap<String, i64>>>,
    /// 工作目录监听管理器（目录树场景的实时数据变更通知）
    pub(crate) workdir_watches: super::workdir::WorkdirWatchManager,
}

use super::store::{create_store, SessionStore};

impl SessionPlugin {
    /// 主构造函数（Factory 机制使用）
    pub fn new(parent: Option<Weak<dyn Plugin>>, config: SessionConfig) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            parent,
            active_mgr: Arc::new(super::active::ActiveSessionManager::new()),
            store: OnceCell::new(),
            heartbeat_state: Arc::new(RwLock::new(HashMap::new())),
            workdir_watches: Default::default(),
        }
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

    /// Session 存储目录：从 [`HomedirRegistry`] 派生
    ///
    /// 路径：`<homedir>/plugins/session`
    ///
    /// 这是 session 存储目录的**唯一权威位置**，不再依赖任何 config 字段。
    /// 切换 homedir 后，下一次 `get_store` 调用将自动使用新 homedir 下的目录。
    pub fn session_storage_dir() -> PathBuf {
        HomedirRegistry::get().join("plugins").join("session")
    }

    /// 获取（或初始化）存储后端。全局单例，首次调用时创建。
    pub(crate) async fn get_store(&self) -> Result<Arc<dyn SessionStore>, PluginError> {
        if let Some(store) = self.store.get() {
            return Ok(Arc::clone(store));
        }

        let base_dir = Self::session_storage_dir();
        let kind = self.config.read().await.store_kind.clone();
        let store = create_store(base_dir, kind).await?;

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

    pub fn config_schema() -> Value {
        json!({
            "type": "object",
            "properties": {
                "max_messages": {
                    "type": "integer",
                    "title": "最大消息数",
                    "description": "单个会话保留的最大消息数量",
                    "default": 100
                },
                "auto_compress": {
                    "type": "boolean",
                    "title": "自动压缩",
                    "description": "上下文 Token 用量达到有效上限 70% 时自动压缩历史（LLM 语义快照）",
                    "default": true
                },
                "enable_compact_tool": {
                    "type": "boolean",
                    "title": "工具压缩",
                    "description": "是否向模型提供主动压缩工具（context_compact）与水位提醒；关闭后仅保留自动压缩。默认关闭，需手动开启",
                    "default": false
                },
                "context_messages": {
                    "type": "integer",
                    "title": "上下文消息数量",
                    "description": "每次发送给 MODEL 的历史消息数量限制 (0 表示不限制)",
                    "default": 6
                },
                "max_tool_rounds": {
                    "type": "integer",
                    "title": "最大工具轮数",
                    "description": "单轮会话中允许的最大工具调用迭代轮数",
                    "default": 15
                },
                "compress_line_threshold": {
                    "type": "integer",
                    "title": "内容节点淡化阈值（行）",
                    "description": "请求视图中单条内容消息超过此行数（或 token 超预算）时做头尾淡化；存储恒为完整原文",
                    "default": 200
                },
                "compress_keep_recent": {
                    "type": "integer",
                    "title": "内容节点淡化保护数",
                    "description": "最近的 Text/Reasoning 内容节点在请求视图中豁免淡化的保护条数（B1 保护窗口），0 表示不保护",
                    "default": 3
                },
                "tool_context_window": {
                    "type": "integer",
                    "title": "工具上下文窗口（轮数）",
                    "description": "保留完整结果的最近工具调用数量限制（滑动窗口）",
                    "default": 15
                },
                "store_kind": {
                    "type": "string",
                    "title": "存储后端",
                    "description": "会话数据的存储后端类型 (file: 目录文件; sqlite: SQLite 数据库)",
                    "enum": ["file", "sqlite"],
                    "default": "file"
                }
            }
        })
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

        // 统一实体协议：entities/list / get / upload / delete / status
        // （SessionPlugin 的 EntityProvider 实现见下方 impl 块）
        if let Some(resp) = crate::symbio_core::entities::dispatch(self.as_ref(), path, &ctx).await
        {
            return resp;
        }

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
            CONFIG_GET => self.invoke_config_get().await?,
            CONFIG_SET => self.invoke_config_set(ctx.clone()).await?,
            "config/schema" => self.invoke_config_schema().await?,
            // 级联选项机制：会话是选项宿主，根选项列表在全项目收集后一次下发
            // （子层经 payload.parent 懒加载，与实体机制 entities/list 同构）
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
                visitor.register_batch(self.build_option_nodes(&ctx).await).await;
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
                    .register(Arc::new(super::heartbeat_tool::HeartbeatTool::new(Arc::downgrade(
                        &self,
                    ))))
                    .await;
            }
        }
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

crate::submit_object_creator!(PLUGIN_SESSION, SessionPlugin::build, dyn Plugin);

// ==================== 会话列表展示辅助 ====================

/// 会话列表一行摘要：最后一条含文本消息的首行（压缩空白、限长 60 字符）。
///
/// 与 [`crate::plugins::session::types::derive_session_title`] 同风格；
/// 供 EntitySummary.summary（通用字段）驱动列表「实时缩略」预览。
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

/// 会话 → 统一实体摘要（顶层清单与容器子会话清单共用的单一实现）。
///
/// 显示名 = `display_title`（title 优先 → 内容自动生成 → 「新对话」）；
/// 状态 = working/active；extra 携带 message_count / is_working / metadata /
/// meta_tags（工作目录名 + 消息数）。
fn summarize_session(s: &Session, is_working: bool) -> crate::symbio_core::entities::EntitySummary {
    let mut it = crate::symbio_core::entities::EntitySummary::new(
        crate::symbio_core::entities::ENTITY_SESSION,
        &s.id,
        s.display_title(),
    );
    it.status = if is_working {
        "working".to_string()
    } else {
        "active".to_string()
    };
    it.updated_at = Some(s.updated_at);
    // 一行摘要（通用字段，前端 subtitle = description || summary）：
    // 最后一条含文本消息的首行——列表即「会话实时缩略」
    it.summary = derive_session_summary(&s.messages);
    if let serde_json::Value::Object(ref mut m) = it.extra {
        let _ = m.insert("message_count".to_string(), json!(s.messages.len()));
        let _ = m.insert("is_working".to_string(), json!(is_working));
        let _ = m.insert("metadata".to_string(), s.metadata.clone());
        // 通用元信息标签（前端 EntityCard tags 原样渲染）：工作目录名 + 消息数
        let mut meta_tags: Vec<String> = Vec::new();
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
                meta_tags.push(base.to_string());
            }
        }
        meta_tags.push(format!("{} 条", s.messages.len()));
        let _ = m.insert("meta_tags".to_string(), json!(meta_tags));
    }
    it
}

// ==================== 统一实体协议接入 ====================

#[async_trait]
impl crate::symbio_core::entities::EntityProvider for SessionPlugin {
    fn kind(&self) -> &'static str {
        crate::symbio_core::entities::ENTITY_SESSION
    }

    /// 会话列表来自 SessionStore（非 EntityStore 实体目录），
    /// 摘要携带实时工作状态（is_working）供前端列表即时渲染。
    async fn list_items(
        &self,
        _ctx: &Arc<dyn InvokeRequest>,
    ) -> Result<Vec<crate::symbio_core::entities::EntitySummary>, PluginError> {
        let sessions = self.list_sessions().await?;
        let active = self.active_mgr.sessions.read().await;
        Ok(sessions
            .iter()
            .map(|s| {
                let is_working = active
                    .get(&s.id)
                    .map(|st| st.inner.try_read().map(|i| i.is_working).unwrap_or(false))
                    .unwrap_or(false);
                summarize_session(s, is_working)
            })
            .collect())
    }

    // ==================== 容器子实体（统一协议 container 语义） ====================
    //
    // 条目（会话）即容器，内部托管两类子实体（声明见 SESSION_CONTAINER_KINDS）：
    // - 子会话（列表视图）：系统管理型，存储归属由 `metadata.parent_session_id`
    //   声明、文件后端路由到父会话目录的 `sessions/` 子目录；
    // - 目录树（tree 机制的场景实现）：会话工作目录的层级浏览（只读），
    //   经 `parent` 请求参数逐层懒加载，场景实现见 `super::workdir`。

    /// 列出容器（父会话）内的子实体（按 sub_kind 分流：子会话 / 目录树）
    async fn list_container_items(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
        sub_kind: Option<&str>,
        container: &str,
        parent: Option<&str>,
    ) -> Result<Vec<crate::symbio_core::entities::EntitySummary>, PluginError> {
        if sub_kind == Some(super::workdir::TREE_KIND) {
            // tree 场景：会话工作目录的下一层节点（接入实时监听，文件变化
            // 经粗粒度 `data` 事件驱动前端树视图与详情防抖重载）
            let store = self.get_store().await?;
            let session = store.load_session(container).await?;
            let Some(workdir) = super::workdir::workdir_of(&session) else {
                return Ok(Vec::new());
            };
            self.workdir_watches.ensure_watch(&workdir, container);
            return super::workdir::list_children(ctx, &workdir, parent).await;
        }

        // 子会话清单（sub_kind 为 None 时容器页做全量分箱，目录树为懒加载
        // 不参与全量下发）
        if sub_kind.is_some_and(|k| k != crate::symbio_core::entities::ENTITY_SESSION) {
            return Ok(Vec::new());
        }
        let store = self.get_store().await?;
        let sessions = store.list_sub_sessions(container).await?;
        let active = self.active_mgr.sessions.read().await;
        Ok(sessions
            .iter()
            .map(|s| {
                let is_working = active
                    .get(&s.id)
                    .map(|st| st.inner.try_read().map(|i| i.is_working).unwrap_or(false))
                    .unwrap_or(false);
                summarize_session(s, is_working)
            })
            .collect())
    }

    /// 顶层实体详情（非 EntityStore 型 provider 重写）：会话摘要
    /// （容器实体页的条目名展示依赖此钩子）
    async fn get_item(
        &self,
        _ctx: &Arc<dyn InvokeRequest>,
        id: &str,
    ) -> Result<crate::symbio_core::entities::EntitySummary, PluginError> {
        let session = self.get_store().await?.load_session(id).await?;
        if session.id != id {
            return Err(PluginError::NotFound(format!("会话不存在: {id}")));
        }
        let active = self.active_mgr.sessions.read().await;
        let is_working = active
            .get(id)
            .map(|st| st.inner.try_read().map(|i| i.is_working).unwrap_or(false))
            .unwrap_or(false);
        Ok(summarize_session(&session, is_working))
    }

    /// 读取单个子实体详情：先按子会话解析（归属校验），否则按目录树节点
    /// （文件内容置于 extra.content；接入实时监听）
    async fn get_container_item(
        &self,
        _ctx: &Arc<dyn InvokeRequest>,
        id: &str,
        container: &str,
    ) -> Result<crate::symbio_core::entities::EntitySummary, PluginError> {
        let store = self.get_store().await?;

        // 1) 子会话分支：load_session 命中且归属当前容器
        let session = store.load_session(id).await?;
        if session.id == id && session.parent_session_id() == Some(container) {
            let active = self.active_mgr.sessions.read().await;
            let is_working = active
                .get(&session.id)
                .map(|st| st.inner.try_read().map(|i| i.is_working).unwrap_or(false))
                .unwrap_or(false);
            return Ok(summarize_session(&session, is_working));
        }

        // 2) 目录树节点分支（父会话无工作目录时无此场景）
        let container_session = store.load_session(container).await?;
        let Some(workdir) = super::workdir::workdir_of(&container_session) else {
            return Err(PluginError::NotFound(format!(
                "会话 {container} 下不存在子实体 {id}"
            )));
        };
        self.workdir_watches.ensure_watch(&workdir, container);
        super::workdir::read_node(&workdir, id).await
    }

    /// 订阅容器数据变更（目录树场景 → 工作目录监听；子会话无实时语义 no-op）
    async fn watch_container(
        &self,
        _ctx: &Arc<dyn InvokeRequest>,
        sub_kind: Option<&str>,
        container: &str,
    ) -> Result<(), PluginError> {
        if sub_kind != Some(super::workdir::TREE_KIND) {
            return Ok(());
        }
        let store = self.get_store().await?;
        let session = store.load_session(container).await?;
        if let Some(workdir) = super::workdir::workdir_of(&session) {
            self.workdir_watches.ensure_watch(&workdir, container);
        }
        Ok(())
    }

    /// 取消订阅（树视图卸载；引用计数归零后监听停止）
    async fn unwatch_container(
        &self,
        _ctx: &Arc<dyn InvokeRequest>,
        sub_kind: Option<&str>,
        container: &str,
    ) -> Result<(), PluginError> {
        if sub_kind != Some(super::workdir::TREE_KIND) {
            return Ok(());
        }
        let store = self.get_store().await?;
        let session = store.load_session(container).await?;
        if let Some(workdir) = super::workdir::workdir_of(&session) {
            self.workdir_watches.release_watch(&workdir, container);
        }
        Ok(())
    }

    /// 写入目录树文件节点（目录树文件编辑的写回；子会话不可写）
    async fn put_container_item(
        &self,
        _ctx: &Arc<dyn InvokeRequest>,
        id: &str,
        content: &str,
        container: &str,
    ) -> Result<crate::symbio_core::entities::EntityUploadResponse, PluginError> {
        let store = self.get_store().await?;

        // 子会话分支不可写（系统管理型，仅可删除）
        let session = store.load_session(id).await?;
        if session.id == id && session.parent_session_id() == Some(container) {
            return Err(PluginError::ValidationError(
                "子会话不支持内容写回".to_string(),
            ));
        }

        let container_session = store.load_session(container).await?;
        let Some(workdir) = super::workdir::workdir_of(&container_session) else {
            return Err(PluginError::NotFound(format!(
                "会话 {container} 下不存在子实体 {id}"
            )));
        };
        self.workdir_watches.ensure_watch(&workdir, container);
        super::workdir::write_node(&workdir, id, content).await?;
        Ok(crate::symbio_core::entities::EntityUploadResponse {
            kind: super::workdir::TREE_KIND.to_string(),
            id: id.to_string(),
            created: false,
        })
    }

    /// 删除子实体：子会话 → abort 后删除；目录树节点 → 删文件（目录拒绝）
    async fn delete_container_item(
        &self,
        _ctx: &Arc<dyn InvokeRequest>,
        id: &str,
        container: &str,
    ) -> Result<crate::symbio_core::entities::EntityUploadResponse, PluginError> {
        let store = self.get_store().await?;
        let session = store.load_session(id).await?;

        // 1) 子会话分支（先 abort 活跃任务再删，与顶层 delete_item 同语义）
        if session.id == id && session.parent_session_id() == Some(container) {
            self.delete_session_internal(id).await?;
            return Ok(crate::symbio_core::entities::EntityUploadResponse {
                kind: crate::symbio_core::entities::ENTITY_SESSION.to_string(),
                id: id.to_string(),
                created: false,
            });
        }

        // 2) 目录树文件节点分支
        let container_session = store.load_session(container).await?;
        let Some(workdir) = super::workdir::workdir_of(&container_session) else {
            return Err(PluginError::NotFound(format!(
                "会话 {container} 下不存在子实体 {id}"
            )));
        };
        super::workdir::delete_node(&workdir, id).await?;
        Ok(crate::symbio_core::entities::EntityUploadResponse {
            kind: super::workdir::TREE_KIND.to_string(),
            id: id.to_string(),
            created: false,
        })
    }

    /// 删除会话（统一协议 entities/delete；非 EntityStore 型 provider 重写）。
    ///
    /// 复用 `delete_session_internal`：先 abort 活跃任务再删除——与旧
    /// `session/clear` 路由同语义，前端机制列表的删除按钮直接受益。
    async fn delete_item(
        &self,
        _ctx: &Arc<dyn InvokeRequest>,
        id: &str,
    ) -> Result<(), PluginError> {
        self.delete_session_internal(id).await
    }

    /// 查询单个会话的实时工作状态
    async fn test_status(
        &self,
        _ctx: &Arc<dyn InvokeRequest>,
        id: &str,
    ) -> Result<crate::symbio_core::entities::EntityStatusResponse, PluginError> {
        let active = self.active_mgr.sessions.read().await;
        let is_working = active
            .get(id)
            .map(|st| st.inner.try_read().map(|i| i.is_working).unwrap_or(false))
            .unwrap_or(false);
        Ok(crate::symbio_core::entities::EntityStatusResponse {
            kind: crate::symbio_core::entities::ENTITY_SESSION.to_string(),
            id: id.to_string(),
            status: if is_working {
                "working".to_string()
            } else {
                "active".to_string()
            },
            status_detail: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 验证 session 存储目录**只**从 HomedirRegistry 派生，不依赖 config
    #[test]
    fn test_session_storage_dir_from_homedir() {
        let dir = SessionPlugin::session_storage_dir();
        let expected = HomedirRegistry::get().join("plugins").join("session");
        assert_eq!(
            dir, expected,
            "session_storage_dir 必须等于 <homedir>/plugins/session"
        );
        assert!(
            dir.is_absolute(),
            "session_storage_dir 必须是绝对路径: {}",
            dir.display()
        );
    }

    /// 验证 SessionConfig 不再包含 storage_dir 字段
    #[test]
    fn test_session_config_has_no_storage_dir() {
        let cfg = SessionConfig::default();
        let json = serde_json::to_value(&cfg).unwrap();
        assert!(
            json.get("storage_dir").is_none(),
            "SessionConfig 不应再包含 storage_dir 字段, got: {json}"
        );
    }

    /// 验证从含 storage_dir 的旧配置反序列化时，字段被忽略
    #[test]
    fn test_session_config_deserialize_ignores_storage_dir() {
        let json = serde_json::json!({
            "storage_dir": "/tmp/should_be_ignored",
            "max_messages": 42,
        });
        let cfg: SessionConfig = serde_json::from_value(json).unwrap();
        assert_eq!(cfg.max_messages, 42, "max_messages 应被正确反序列化");
    }
}
