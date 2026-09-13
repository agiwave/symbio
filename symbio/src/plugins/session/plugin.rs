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
use crate::symbio_core::schemas::options::OPTIONS_LIST;
use crate::symbio_core::schemas::session::chat_message as cm;
pub use crate::symbio_core::schemas::session::session_config::SessionConfig;
use crate::symbio_core::vdfs;
use crate::symbio_core::{
    HomedirRegistry, InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginError,
    PluginMeta, PluginPayload, CONFIG_GET, CONFIG_SET, PLUGIN_SESSION,
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
    /// VDFS 实时：会话变更广播源。
    ///
    /// provider 是**变更源的持有者**：会话的任何写入 / 删除都经此广播，
    /// [`vdfs::VdfsProvider::watch`] 的转发任务订阅它并调用 sink，
    /// 变更因此无需轮询即可到达 VDFS 事件总线。
    pub(crate) change_tx: tokio::sync::broadcast::Sender<vdfs::VdfsChange>,
    /// VDFS 实时：被订阅路径 → 转发任务（`unwatch` 时取消）
    pub(crate) watch_tasks: Arc<tokio::sync::Mutex<HashMap<String, tokio::task::JoinHandle<()>>>>,
}

use super::store::{create_store, SessionStore};

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

    /// UI 配置 schema。
    ///
    /// 默认值单一真源 = `SessionConfig::default()`（其字段 serde default 与该 impl 同源）：
    /// 本函数只描述 UI 元信息（类型/标题/说明/枚举候选），默认值一律从序列化后的
    /// 默认配置读取，杜绝历史上 schema 与 serde 双份字面量的漂移
    /// （曾出现 `max_tool_rounds` schema=15 而 serde=65535）。
    ///
    /// 注：不可配置字段（如已删除的 `storage_dir` / `session_id`）不出现在本 schema 中，
    /// 由 `test_schema_covers_all_configurable_fields` 与结构体字段集合双向锁定。
    pub fn config_schema() -> Value {
        let defaults = serde_json::to_value(SessionConfig::default()).unwrap_or_else(|_| json!({}));
        let d = |key: &str| defaults.get(key).cloned().unwrap_or(Value::Null);
        json!({
            "type": "object",
            "properties": {
                "max_messages": {
                    "type": "integer",
                    "title": "最大消息数",
                    "description": "存储层保留的最大用户轮次数（超出按 FIFO 淘汰；0 表示不限制）",
                    "default": d("max_messages")
                },
                "auto_compress": {
                    "type": "boolean",
                    "title": "自动压缩",
                    "description": "上下文 Token 用量达到有效上限 70% 时自动压缩历史（LLM 语义快照）",
                    "default": d("auto_compress")
                },
                "enable_compact_tool": {
                    "type": "boolean",
                    "title": "工具压缩",
                    "description": "是否向模型提供主动压缩工具（context_compact）与水位提醒；关闭后仅保留自动压缩。默认关闭，需手动开启",
                    "default": d("enable_compact_tool")
                },
                "context_messages": {
                    "type": "integer",
                    "title": "上下文消息数量",
                    "description": "每次发送给 MODEL 的历史消息数量限制 (0 表示不限制)",
                    "default": d("context_messages")
                },
                "max_tool_rounds": {
                    "type": "integer",
                    "title": "最大工具轮数",
                    "description": "单轮会话中允许的最大工具调用迭代轮数（软上限，达到时明确提示后退出；0 表示不限制，即默认值——产品决策：不设硬性轮次上限）",
                    "default": d("max_tool_rounds")
                },
                "compress_line_threshold": {
                    "type": "integer",
                    "title": "内容节点淡化阈值（行）",
                    "description": "请求视图中单条内容消息超过此行数（或 token 超预算）时做头尾淡化；存储恒为完整原文",
                    "default": d("compress_line_threshold")
                },
                "compress_keep_recent": {
                    "type": "integer",
                    "title": "内容节点淡化保护数",
                    "description": "最近的 Text/Reasoning 内容节点在请求视图中豁免淡化的保护条数（B1 保护窗口），0 表示不保护",
                    "default": d("compress_keep_recent")
                },
                "tool_context_window": {
                    "type": "integer",
                    "title": "工具上下文窗口（轮数）",
                    "description": "保留完整结果的最近工具调用数量限制（滑动窗口）",
                    "default": d("tool_context_window")
                },
                "fade_activate_rounds": {
                    "type": "integer",
                    "title": "工具淡化激活轮数",
                    "description": "单轮请求的工具迭代轮数超过此值后，请求视图把较早轮次的工具结果压成头尾摘要（存储保留全文）；0 表示每一轮都淡化",
                    "default": d("fade_activate_rounds")
                },
                "fade_keep_recent_turns": {
                    "type": "integer",
                    "title": "工具淡化保留轮数",
                    "description": "淡化时保持原文的最近 user turn 数，保证模型对\"最近在做什么\"的记忆不被摘要打断；0 表示全部淡化",
                    "default": d("fade_keep_recent_turns")
                },
                "prune_tool_history": {
                    "type": "boolean",
                    "title": "写入期裁剪工具历史",
                    "description": "落库时物理删除 context_messages 轮之前的工具调用链（Tool/ToolCall/Reasoning）及其存档；关闭后存储保留完整原文，工具链裁剪只发生在请求视图（体积换可回看性）",
                    "default": d("prune_tool_history")
                },
                "store_kind": {
                    "type": "string",
                    "title": "存储后端",
                    "description": "会话数据的存储后端类型 (file: 目录文件; sqlite: SQLite 数据库; memory: 进程内内存，不落盘、进程退出即丢失)",
                    "enum": ["file", "sqlite", "memory"],
                    "default": d("store_kind")
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

/// 会话的通用元信息标签（工作目录名 + 消息数）。
///
/// 实体机制（`extra.meta_tags`）与 VDFS（节点 `attributes.meta_tags`）**共用同一份
/// 实现**，保证同一会话在两条链路上的列表呈现一致；前端原样渲染，不含语义。
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
        let _ = m.insert("meta_tags".to_string(), json!(session_meta_tags(s)));
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
        _ctx: &Arc<dyn InvokeRequest>,
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
            return super::workdir::list_children(&workdir, parent).await;
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

// ==================== VDFS 挂载点（/session） ====================
//
// 会话是 VDFS 的第二个原生 provider：
//
// - **根 = 会话清单**：`.vdfs/session` 的目录内容即全部会话；根下可新建「会话」
//   （`new_types`），**新建语义完全由本 provider 自持**——id 由 provider 生成、
//   路径名作标题、经 `create` 写意图区分「新建」与「覆盖」；
// - **节点 = 单个会话**：`ext = session` → 前端聊天工作区渲染器（同一份详情实现
//   同时服务实体机制与 VDFS 机制）；
// - **实时**：会话的任何变更经 [`SessionPlugin::notify_change`] 广播 → `watch`
//   的转发任务 → VDFS 事件总线，前端列表无需轮询即可收敛。
//
// 读写删都转发既有会话能力（SessionStore + 会话 metadata 合并），不新造协议。

/// 会话节点：`ext = session`（前端据此选聊天工作区渲染器）。
///
/// 标题 / 摘要 / 状态 / 元信息标签与实体机制的列表呈现**同源**（复用
/// `display_title`、`derive_session_summary` 与 [`session_meta_tags`]），
/// 保证同一会话在两条链路上的呈现一致。
///
/// 另在 `attributes` 上挂载会话清单所需字段（`message_count` / `metadata` /
/// `meta_tags`）——它们是**场景数据**，VDFS 只透传；会话清单由此可直接用
/// `vdfs/list` 承载（见 S8），无需再走 `entities/list`。
fn session_node(s: &Session, is_working: bool) -> vdfs::VdfsNode {
    let mut n = vdfs::VdfsNode::file(&s.id, s.display_title(), vdfs::VdfsAccess::READ_WRITE);
    n.kind = crate::symbio_core::entities::ENTITY_SESSION.to_string();
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
// 其内部结构（子会话 / 工作目录树）作为**会话同名目录**挂在会话之下：
//
//   <id>                  → 会话叶子（聊天详情）
//   <id>/子会话[/<sub>]    → 子会话清单 / 单个子会话（查看 · 删除）
//   <id>/工作目录[/<rel>]  → 工作目录树（文件可查看 / 编辑）
//
// 该寻址取代原「容器实体页」（`/container/session/<id>/entities`）——同一批
// 能力改由 VDFS 承载，机制侧零新增概念；场景实现仍复用 `workdir` 与
// `EntityProvider` 的容器钩子，两条链路共用同一份校验与 IO。

/// 会话挂载点内的路径解析结果
enum VdfsSessionPath<'a> {
    /// 挂载根 = 会话清单
    Root,
    /// `<id>`：单个会话（叶子）
    Session(&'a str),
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

/// 会话内部的两个虚拟子目录（工作目录按会话是否声明 workdir 决定是否出现）
fn internal_dirs(has_workdir: bool) -> Vec<vdfs::VdfsNode> {
    let mut out = vec![vdfs::VdfsNode::dir(
        super::workdir::SEG_SUB_SESSIONS,
        super::workdir::SEG_SUB_SESSIONS,
        vdfs::VdfsAccess::LIST,
    )];
    if has_workdir {
        out.push(vdfs::VdfsNode::dir(
            super::workdir::SEG_WORKDIR,
            super::workdir::SEG_WORKDIR,
            vdfs::VdfsAccess::LIST,
        ));
    }
    out
}

/// 工作目录条目（`EntitySummary`）→ VDFS 节点。
///
/// 目录 → 只读（`l`；工作目录不提供新建，与原容器语义一致）；
/// 文件 → 可读写（`rw`）。`ext` 不显式设置：由文件名推导（`a.md` → `md`），
/// 渲染器据此分发。
fn workdir_node(it: &crate::symbio_core::entities::EntitySummary) -> vdfs::VdfsNode {
    let is_dir = it
        .extra
        .get("is_dir")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let name = it.name.clone();
    if is_dir {
        vdfs::VdfsNode::dir(name.clone(), name, vdfs::VdfsAccess::LIST)
    } else {
        let mut n = vdfs::VdfsNode::file(name.clone(), name, vdfs::VdfsAccess::READ_WRITE);
        n.size = it.extra.get("size").and_then(|v| v.as_u64());
        n
    }
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

#[async_trait]
impl vdfs::VdfsProvider for SessionPlugin {
    fn label(&self) -> Option<&str> {
        // 标签 / 顺序取自实体注册表（单一真相源），使 `.vdfs` 左栏与实体页恒等
        Some(
            crate::symbio_core::entities::nav_meta_of(crate::symbio_core::entities::ENTITY_SESSION)
                .map_or("会话", |(label, _)| label),
        )
    }

    fn description(&self) -> Option<&str> {
        Some("会话清单。每个会话是一份独立对话记录，可读 / 写 / 删；新建即创建一份新会话。")
    }

    fn order(&self) -> i32 {
        crate::symbio_core::entities::nav_meta_of(crate::symbio_core::entities::ENTITY_SESSION)
            .map_or(1, |(_, order)| order)
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
        match parse_session_path(path)? {
            VdfsSessionPath::Root => {
                let sessions = self
                    .list_sessions()
                    .await
                    .map_err(vdfs::from_plugin_error)?;
                Ok(self.nodes_of_sessions(&sessions).await)
            }
            // 会话内部：两个虚拟子目录（会话存在性校验由 `session_of` 承担）
            VdfsSessionPath::Session(id) => {
                let session = self.session_of(id).await?;
                Ok(internal_dirs(
                    super::workdir::workdir_of(&session).is_some(),
                ))
            }
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
                // 目录树场景的实时监听（与实体机制同一套，按会话 id 引用计数）
                self.workdir_watches.ensure_watch(&workdir, id);
                let items = super::workdir::list_children(&workdir, Some(rel))
                    .await
                    .map_err(vdfs::from_plugin_error)?;
                Ok(items.iter().map(workdir_node).collect())
            }
        }
    }

    async fn stat(&self, _ctx: &vdfs::VdfsContext, path: &str) -> vdfs::VdfsResult<vdfs::VdfsNode> {
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
                n.kind = crate::symbio_core::entities::ENTITY_SESSION.to_string();
                Ok(n)
            }
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
                let it = super::workdir::read_node(&workdir, rel)
                    .await
                    .map_err(vdfs::from_plugin_error)?;
                Ok(workdir_node(&it))
            }
        }
    }

    /// 读取会话内容（转写全文 + 元数据，JSON）——转发既有存储，不新造协议
    async fn read(
        &self,
        _ctx: &vdfs::VdfsContext,
        path: &str,
    ) -> vdfs::VdfsResult<vdfs::VdfsContent> {
        match parse_session_path(path)? {
            VdfsSessionPath::Session(id) => {
                let session = self.session_of(id).await?;
                session_content(&session)
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
    /// 消息（转写）**不经 VDFS 写入**——聊天流由既有 chat 协议承载；VDFS 只承担
    /// 「资源读写」这一层，避免出现两套写路径。
    async fn write(
        &self,
        _ctx: &vdfs::VdfsContext,
        path: &str,
        content: &vdfs::VdfsContent,
    ) -> vdfs::VdfsResult<vdfs::VdfsWriteResponse> {
        // 工作目录分支：文件写回（与实体机制同一份实现——路径越界校验 + 落盘）
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
    /// 按 id 取会话（**存在性校验**：`load_session` 对未命中会返回空会话，
    /// 因此这里以清单为准判定存在性）
    async fn session_of(&self, id: &str) -> vdfs::VdfsResult<Session> {
        self.list_sessions()
            .await
            .map_err(vdfs::from_plugin_error)?
            .into_iter()
            .find(|s| s.id == id)
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

    /// 核心回归网：`config_schema` 的每个 `default` 必须等于 `SessionConfig::default()`
    /// 对应字段的值。历史上两处各自维护字面量，出现 `max_tool_rounds` schema=15 而
    /// serde=65535 的漂移（用户看到 15 的默认值，实际行为是无限轮次）。
    /// schema 现已改为从 `SessionConfig::default()` 派生，本测试锁定这一不变式。
    #[test]
    fn config_schema_defaults_match_session_config_default() {
        let schema = SessionPlugin::config_schema();
        let props = schema
            .get("properties")
            .and_then(|v| v.as_object())
            .expect("config_schema 必须有 properties 对象");
        let defaults = serde_json::to_value(SessionConfig::default())
            .expect("SessionConfig::default() 必须可序列化");

        for (key, field) in props {
            let declared = field
                .get("default")
                .unwrap_or_else(|| panic!("schema 字段 {key} 缺少 default"));
            let actual = defaults
                .get(key)
                .unwrap_or_else(|| panic!("schema 声明了 SessionConfig 不存在的字段 {key}"));
            assert_eq!(
                declared, actual,
                "配置契约漂移：schema[{key}].default={declared} 但 SessionConfig::default().{key}={actual}"
            );
        }
    }

    /// schema 不得遗漏 `SessionConfig` 的任何**可配置**字段（否则该字段在设置面板
    /// 不可见，且上面的逐字段比对会失去覆盖）。
    #[test]
    fn config_schema_covers_all_session_config_fields() {
        /// 非用户可配置字段白名单。历史上 `session_id` 在此豁免——它是全仓零消费者、
        /// 零赋值的死字段（配置按会话目录存放，id 由目录名决定），已随复杂度审计 R8
        /// 从 `SessionConfig` 删除，故白名单现为空。保留机制以便未来出现真正的
        /// 身份/路由字段时使用。
        const NON_CONFIGURABLE: &[&str] = &[];

        let schema = SessionPlugin::config_schema();
        let props = schema.get("properties").unwrap().as_object().unwrap();
        let defaults = serde_json::to_value(SessionConfig::default()).unwrap();
        let fields = defaults.as_object().unwrap();

        for key in fields.keys() {
            if NON_CONFIGURABLE.contains(&key.as_str()) {
                continue;
            }
            assert!(
                props.contains_key(key),
                "SessionConfig 可配置字段 {key} 未出现在 config_schema 中"
            );
        }
    }

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
        // 顺序取自实体注册表（**单一真相源**），使 `.vdfs` 左栏与实体页恒等
        // （S4 起不再是 provider 自定的 10）
        assert_eq!(
            p.order(),
            crate::symbio_core::entities::nav_meta_of(crate::symbio_core::entities::ENTITY_SESSION)
                .map_or(1, |(_, order)| order)
        );
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
    /// kind / 状态 / 更新时间 / 摘要与实体机制同源
    #[test]
    fn session_node_carries_renderer_ext_and_presentation() {
        let mut s = Session::new("abc");
        s.updated_at = 1_700_000_000_000;

        let idle = session_node(&s, false);
        assert_eq!(idle.name, "abc");
        assert_eq!(idle.effective_ext().as_deref(), Some("session"));
        assert_eq!(idle.kind, crate::symbio_core::entities::ENTITY_SESSION);
        assert_eq!(idle.status, vdfs::VFDS_STATUS_ACTIVE);
        assert_eq!(idle.updated_at, Some(1_700_000_000_000));
        assert_eq!(idle.access.flags(), "rw", "会话可读可写");
        assert!(!idle.is_dir(), "会话是文档而非目录");

        let busy = session_node(&s, true);
        assert_eq!(busy.status, vdfs::VFDS_STATUS_WORKING);
    }

    /// 会话内部寻址（S6）：`<id>` / `<id>/子会话[/<sub>]` / `<id>/工作目录[/<rel>]`。
    /// 未知区段与越界层级一律 NotFound——不给半通不通的路径留口子。
    #[test]
    fn vdfs_internal_path_parsing() {
        use VdfsSessionPath::*;
        assert!(matches!(parse_session_path("").unwrap(), Root));
        assert!(matches!(parse_session_path("/").unwrap(), Root));
        assert!(matches!(parse_session_path("abc").unwrap(), Session("abc")));
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
        // 未知区段、子会话越界层级 → NotFound
        assert!(parse_session_path("abc/nope").is_err());
        assert!(parse_session_path("abc/子会话/s1/deeper").is_err());
    }

    /// 会话内部的两个虚拟子目录：工作目录按会话是否声明 workdir 决定是否出现
    #[test]
    fn vdfs_internal_dirs_conditional() {
        let without = internal_dirs(false);
        assert_eq!(without.len(), 1);
        assert_eq!(without[0].name, workdir::SEG_SUB_SESSIONS);
        assert!(without[0].is_dir(), "子会话是目录");

        let with = internal_dirs(true);
        assert_eq!(with.len(), 2);
        assert_eq!(with[1].name, workdir::SEG_WORKDIR);
        assert!(
            with.iter().all(|n| n.is_dir() && !n.access.write),
            "两个内部区段都是只读目录（工作目录不提供新建）"
        );
    }

    /// 工作目录条目 → VDFS 节点：目录只读（`l`），文件可读写（`rw`）+ 字节数
    #[test]
    fn vdfs_workdir_node_shapes() {
        use crate::symbio_core::entities::EntitySummary;

        let mut d = EntitySummary::new(workdir::TREE_KIND, "src", "src");
        if let Value::Object(ref mut m) = d.extra {
            let _ = m.insert("is_dir".to_string(), json!(true));
        }
        let dn = workdir_node(&d);
        assert!(dn.is_dir());
        assert_eq!(dn.access.flags(), "l", "工作目录子目录只读");

        let mut f = EntitySummary::new(workdir::TREE_KIND, "README.md", "README.md");
        if let Value::Object(ref mut m) = f.extra {
            let _ = m.insert("is_dir".to_string(), json!(false));
            let _ = m.insert("size".to_string(), json!(42u64));
        }
        let file = workdir_node(&f);
        assert!(!file.is_dir());
        assert_eq!(file.access.flags(), "rw", "工作目录文件可编辑");
        assert_eq!(file.size, Some(42));
    }

    /// 会话节点自带清单字段（S8）：`message_count` / `metadata` / `meta_tags`
    /// 挂在 flatten 的 attributes 上，使会话清单无需再走 `entities/list`
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
}
