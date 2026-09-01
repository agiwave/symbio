//! MCP (Model Context Protocol) 插件实现
//!
//! ## 职责划分（2026-07-06 重构）
//!
//! - **后端**（本插件）：MCP **配置**（CRUD）+ **客户端 transport**（stdio/http）
//! - **前端**（`tauri`）：**仅**负责 MCP Server 的配置管理（CRUD UI），
//!   不实现任何 transport 客户端
//!
//! ## 系统工具机制集成
//!
//! 与 [`web`](crate::plugins::web) 插件完全一致：
//! - 静态插件持有 `manager: Arc<McpManager>`（无状态 transport 路由器）
//! - `traverse(TRAVERSE_AVAILABLE_TOOLS)` 时遍历 `McpConfig.servers`，
//!   对每个 enabled server 调用 `manager.discover_tools` 动态发现工具，
//!   把每个工具包装为 [`McpToolCapability`] 注册到 `ctx.get(CAPABILITY_MANAGER)`
//! - agent 通过 `tool_manager.invoke("mcp.<server>.<tool>", ctx)` 调用
//!
//! ## 存储策略
//!
//! 每个 MCP Server 作为独立实体存放在
//! `~/.symbio/plugins/mcps/<name>/server.json`。
//! `McpConfig` 的内存视图（`servers: HashMap<name, McpServerConfig>`）
//! 通过从磁盘加载/回写保持一致。

use crate::symbio_core::create_object;
pub use crate::symbio_core::schemas::mcp::mcp_config::{McpConfig, McpServerConfig};
use crate::symbio_core::schemas::{common, mcp::mcp_servers};
use crate::symbio_core::{
    Capability, CapabilityMeta, InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin,
    PluginError, PluginMeta, PluginPayload, CONFIG_GET, CONFIG_SET, PLUGIN_MCP,
};
use async_trait::async_trait;
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;
use tracing::warn;

use super::capability::McpToolCapability;
use super::manager::McpManager;

/// MCP 插件
pub struct McpPlugin {
    /// 配置 (server_name -> config)
    config: Arc<RwLock<McpConfig>>,
    /// MCP transport 路由器（无状态，跨调用共享）
    manager: Arc<McpManager>,
    /// 父插件引用
    parent: Arc<RwLock<Option<Weak<dyn Plugin>>>>,
    /// 首次加载标志（防止 route / traverse 在 load_from_storage 完成前访问旧 config）
    loaded: Arc<tokio::sync::Mutex<bool>>,
}

impl McpPlugin {
    /// 静态工厂：从 InvokeRequest 构造 Plugin 实例
    pub fn build(ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Plugin> {
        // 同步 fallback：先从 ctx.config() 解析（兼容旧 config.yaml）
        let config: McpConfig = ctx
            .config()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();

        let parent = ctx.parent();
        let plugin = Arc::new(McpPlugin::new(parent, config));

        // 启动后异步触发：从新存储加载（并触发首启动数据迁移）
        let plugin_weak = Arc::downgrade(&plugin);
        let ctx_clone = ctx.clone();
        tokio::spawn(async move {
            if let Some(mcp) = plugin_weak.upgrade() {
                mcp.load_from_storage(&ctx_clone).await;
            }
        });

        plugin as Arc<dyn Plugin>
    }

    /// 主构造函数
    pub fn new(parent: Option<Weak<dyn Plugin>>, config: McpConfig) -> Self {
        Self {
            config: Arc::new(RwLock::new(config)),
            manager: Arc::new(McpManager::new()),
            parent: Arc::new(RwLock::new(parent)),
            loaded: Arc::new(tokio::sync::Mutex::new(false)),
        }
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new("mcp", "MCP 工具集成")
            .with_description("提供与 MCP 服务器的连接和交互功能")
            .with_version("0.3.0")
    }

    async fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        let guard = self.parent.read().await;
        guard.as_ref().and_then(|w| w.upgrade())
    }

    /// 异步加载：从 `~/.symbio/plugins/mcps/` 读取所有 MCP Server
    ///
    /// - 若新存储为空，则触发首启动迁移（从 ctx.config()）
    pub async fn load_from_storage(&self, ctx: &Arc<dyn InvokeRequest>) {
        let store = match create_object::<dyn crate::symbio_core::providers::StorageService>(
            "storage_service",
            ctx.clone(),
        ) {
            Some(s) => s,
            None => {
                crate::plugin_warn!("mcp", "未找到 storage_service，跳过新存储加载");
                return;
            }
        };

        let es = store.entity_store();
        let category = crate::symbio_core::providers::categories::MCP;
        let manifest = crate::symbio_core::providers::manifests::SERVER;

        // 1. 列出新存储中的所有 MCP Server
        let ids = match es.list_entities(category).await {
            Ok(v) => v,
            Err(_e) => {
                crate::plugin_warn!("mcp", "list mcps 失败");
                return;
            }
        };

        // 2. 如果新存储为空，触发首启动迁移
        if ids.is_empty() {
            self.migrate_from_legacy_config(&*store).await;
            return;
        }

        // 3. 加载新存储的内容
        let mut new_servers = std::collections::HashMap::new();
        for id in &ids {
            match es.read_entity(category, id, manifest).await {
                Ok(content) => match serde_json::from_str::<McpServerConfig>(&content) {
                    Ok(s) => {
                        new_servers.insert(id.clone(), s);
                    }
                    Err(_e) => crate::plugin_warn!("mcp", "解析 server {id} 失败"),
                },
                Err(_e) => crate::plugin_warn!("mcp", "读取 server {id} 失败"),
            }
        }

        let mut cfg = self.config.write().await;
        cfg.servers = new_servers;
        crate::plugin_info!(
            "mcp",
            "从 ~/.symbio/plugins/mcps/ 加载了 {} 个 MCP Server",
            cfg.servers.len()
        );
    }

    /// 幂等保证：首次调用时阻塞执行 `load_from_storage`，后续调用直接返回
    ///
    /// 用于在 `route` / `traverse` 入口确保 `cfg.servers` 反映磁盘最新状态，
    /// 避免与 `build` 中 spawn 的异步加载发生时序竞争。
    async fn ensure_loaded(&self, ctx: &Arc<dyn InvokeRequest>) {
        let mut guard = self.loaded.lock().await;
        if !*guard {
            self.load_from_storage(ctx).await;
            *guard = true;
        }
    }

    /// 持久化单个 server 到磁盘
    ///
    /// 与旧的 `persist_to_disk`（全量重写所有 server）不同——只写一个 server，
    /// 避免修改 A 时意外覆盖 B 的磁盘内容。
    async fn persist_one_server(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
        name: &str,
        server: &McpServerConfig,
    ) -> Result<(), PluginError> {
        let store = create_object::<dyn crate::symbio_core::providers::StorageService>(
            "storage_service",
            ctx.clone(),
        )
        .ok_or_else(|| {
            PluginError::InternalError("storage_service 不可用，无法持久化".to_string())
        })?;

        let es = store.entity_store();
        let category = crate::symbio_core::providers::categories::MCP;
        let manifest = crate::symbio_core::providers::manifests::SERVER;

        let content = serde_json::to_string_pretty(server)
            .map_err(|e| PluginError::InternalError(format!("序列化 server 失败: {e}")))?;
        es.write_entity(category, name, manifest, &content)
            .await
            .map_err(|e| PluginError::InternalError(format!("写入 server 失败: {e}")))
    }

    /// 首启动迁移：从 ctx.config() 中残留的旧配置迁到新存储
    async fn migrate_from_legacy_config(
        &self,
        store: &dyn crate::symbio_core::providers::StorageService,
    ) {
        let current = self.config.read().await.clone();
        if current.servers.is_empty() {
            return;
        }

        crate::plugin_info!(
            "mcp",
            "检测到旧 config 中的 MCP Servers，开始迁移到 ~/.symbio/plugins/mcps/"
        );

        let es = store.entity_store();
        let category = crate::symbio_core::providers::categories::MCP;
        let manifest = crate::symbio_core::providers::manifests::SERVER;

        for (id, s) in &current.servers {
            let content = match serde_json::to_string_pretty(s) {
                Ok(s) => s,
                Err(_e) => {
                    crate::plugin_error!("mcp", "序列化 server {id} 失败");
                    continue;
                }
            };
            if let Err(_e) = es.write_entity(category, id, manifest, &content).await {
                crate::plugin_error!("mcp", "迁移 server {id} 失败");
            }
        }
    }
}

impl Default for McpPlugin {
    fn default() -> Self {
        Self::new(None, McpConfig::default())
    }
}

#[async_trait]
impl Plugin for McpPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        self.ensure_loaded(&ctx).await;

        let sub_path = ctx.get(crate::symbio_core::PATH).unwrap_or_default();
        if sub_path != crate::symbio_core::TRAVERSE_AVAILABLE_TOOLS {
            return Err(crate::symbio_core::PluginError::NotFound(format!(
                "未知遍历路径: {}",
                sub_path
            )));
        }

        // 仅当上游传入了 CAPABILITY_MANAGER 时才注册
        let Some(tool_manager) = ctx.get(crate::symbio_core::CAPABILITY_MANAGER) else {
            return Ok(PluginPayload::new(&Vec::<CapabilityMeta>::new()));
        };

        let cfg = self.config.read().await.clone();

        // 遍历已启用的 server；动态 discover 并注册
        for (name, server_cfg) in &cfg.servers {
            if !server_cfg.enabled {
                continue;
            }
            match self.manager.discover_tools(name, server_cfg).await {
                Ok(tools) => {
                    for tool in tools {
                        let cap: Arc<dyn Capability> = Arc::new(McpToolCapability::new(
                            name.clone(),
                            tool,
                            server_cfg.clone(),
                            self.manager.clone(),
                        ));
                        tool_manager.register(cap).await;
                    }
                }
                Err(e) => {
                    // 单个 server 失败不影响其它 server 的注册
                    warn!(server = name, error = %e, "MCP 工具发现失败，跳过该 server");
                }
            }
        }

        Ok(PluginPayload::new(&Vec::<CapabilityMeta>::new()))
    }

    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        self.ensure_loaded(&ctx).await;

        let path = ctx.get(crate::symbio_core::PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);

        let data = match path {
            CONFIG_GET => {
                // 新存储策略：实际 server 数据存放在
                // `~/.symbio/plugins/mcps/<name>/server.json`，
                // 不在 config.yaml 中。这里只返回元数据。
                let metadata = serde_json::json!({
                    "plugin_provider": "mcp",
                    "plugin_name": "mcp",
                    "_storage": "plugins/mcps",
                });
                serde_json::to_value(metadata)?
            }
            CONFIG_SET => {
                let new_cfg: McpConfig = ctx.payload()?;
                {
                    let mut cfg = self.config.write().await;
                    *cfg = new_cfg;
                }
                if let Some(p) = self.get_parent().await {
                    let save_ctx = ctx.fork();
                    save_ctx.set(crate::symbio_core::PATH, "save_config".to_string());
                    p.route(save_ctx).await?;
                }
                serde_json::to_value(common::SuccessResponse::default())?
            }

            // ============== 单服务器 CRUD（与 LLM Providers 对齐） ==============
            "servers/list" => {
                let cfg = self.config.read().await;
                serde_json::to_value(mcp_servers::mcp_servers_list::Response {
                    config: cfg.clone(),
                })?
            }
            "servers/get" => {
                let req: mcp_servers::mcp_servers_get::Request = ctx.payload()?;
                let cfg = self.config.read().await;
                let server = cfg.servers.get(&req.name).cloned().ok_or_else(|| {
                    PluginError::NotFound(format!("未找到 MCP Server: {}", req.name))
                })?;
                serde_json::to_value(mcp_servers::mcp_servers_get::Response {
                    name: req.name,
                    server,
                })?
            }
            "servers/set" => {
                use crate::symbio_core::schemas::mcp::mcp_config::McpTransportType;
                let req: mcp_servers::mcp_servers_set::Request = ctx.payload()?;
                if req.name.trim().is_empty() {
                    return Err(PluginError::ValidationError(
                        "MCP Server 名称不能为空".to_string(),
                    ));
                }
                // 按 transport_type 校验必填字段
                match req.server.transport_type {
                    McpTransportType::Stdio => {
                        if req
                            .server
                            .command
                            .as_deref()
                            .map(|s| s.trim().is_empty())
                            .unwrap_or(true)
                        {
                            return Err(PluginError::ValidationError(
                                "stdio transport 的 command 不能为空".to_string(),
                            ));
                        }
                    }
                    McpTransportType::Http | McpTransportType::Sse => {
                        if req
                            .server
                            .url
                            .as_deref()
                            .map(|s| s.trim().is_empty())
                            .unwrap_or(true)
                        {
                            return Err(PluginError::ValidationError(
                                "http/sse transport 的 url 不能为空".to_string(),
                            ));
                        }
                    }
                }

                let name = req.name.trim().to_string();

                // 先写盘：写盘失败直接返回错误，不修改内存
                if let Err(e) = self.persist_one_server(&ctx, &name, &req.server).await {
                    crate::plugin_error!("mcp", "持久化 server {name} 失败: {e}");
                    return Err(e);
                }

                // 写盘成功后再更新内存（包含之前存在的旧值快照）
                let existed;
                {
                    let mut cfg = self.config.write().await;
                    existed = cfg.servers.contains_key(&name);
                    cfg.servers.insert(name.clone(), req.server);
                }

                crate::plugin_info!(
                    "mcp",
                    "已{} MCP Server: {}",
                    if existed { "更新" } else { "创建" },
                    name
                );

                // 配置已变更：失效 discover 缓存 + 旧 session
                self.manager.invalidate_discover_cache(&name).await;
                self.manager.session_cache.remove(&name).await;

                let cfg = self.config.read().await;
                serde_json::to_value(mcp_servers::mcp_servers_set::Response {
                    config: cfg.clone(),
                })?
            }
            "servers/delete" => {
                let req: mcp_servers::mcp_servers_delete::Request = ctx.payload()?;

                // 内存层面：先取 server 快照（若不存在则报错）
                let snapshot = {
                    let cfg = self.config.read().await;
                    cfg.servers.get(&req.name).cloned()
                };
                let _snapshot = snapshot.ok_or_else(|| {
                    PluginError::NotFound(format!("未找到 MCP Server: {}", req.name))
                })?;

                // 磁盘层面：先删磁盘（FileEntityStore.delete_entity 会递归删除子目录）
                if let Some(store) = create_object::<
                    dyn crate::symbio_core::providers::StorageService,
                >("storage_service", ctx.clone())
                {
                    let es = store.entity_store();
                    let category = crate::symbio_core::providers::categories::MCP;
                    match es.delete_entity(category, &req.name).await {
                        Ok(()) => {}
                        Err(crate::symbio_core::providers::EntityStoreError::NotFound {
                            ..
                        }) => {
                            crate::plugin_warn!(
                                "mcp",
                                "磁盘上已无 server {} 目录（可能已外部删除），仅清理内存",
                                req.name
                            );
                        }
                        Err(e) => {
                            crate::plugin_error!("mcp", "删除 server {} 失败: {e}", req.name);
                            return Err(PluginError::InternalError(format!(
                                "删除磁盘目录失败: {e}"
                            )));
                        }
                    }
                }

                // 磁盘成功（或磁盘已不存在）后再清理内存
                {
                    let mut cfg = self.config.write().await;
                    cfg.servers.remove(&req.name);
                }
                crate::plugin_info!("mcp", "已删除 MCP Server: {}", req.name);

                // BUG-MR29：使用 forget_server 一次性清理 discover cache + session + per-server lock
                // 避免长期运行时 server_locks 累积孤儿条目
                self.manager.forget_server(&req.name).await;

                serde_json::to_value(mcp_servers::mcp_servers_delete::Response {
                    config: self.config.read().await.clone(),
                })?
            }
            "servers/test" => {
                // BUG-MR20：测试 MCP server 的连接（不修改配置/缓存）
                use mcp_servers::mcp_servers_test;
                let req: mcp_servers_test::Request = ctx.payload()?;

                let cfg = self.config.read().await;
                let server = cfg.servers.get(&req.name).cloned().ok_or_else(|| {
                    PluginError::NotFound(format!("未找到 MCP Server: {}", req.name))
                })?;
                drop(cfg);

                // BUG-MR30/MR32：使用 TestConnectionResult（携带 server_name/version/instructions）
                let test_result = self.manager.test_connection(&req.name, &server).await;

                let (
                    ok,
                    tool_count,
                    protocol_version,
                    server_name,
                    server_version,
                    instructions,
                    error,
                    elapsed_ms,
                ) = match test_result {
                    Ok(r) => (
                        true,
                        r.tool_count,
                        r.protocol_version,
                        r.server_name,
                        r.server_version,
                        r.instructions,
                        None,
                        r.elapsed_ms,
                    ),
                    Err(e) => (
                        false,
                        0,
                        DEFAULT_PROTOCOL_VERSION_FALLBACK.to_string(),
                        None,
                        None,
                        None,
                        Some(e),
                        0,
                    ),
                };

                serde_json::to_value(mcp_servers_test::Response {
                    name: req.name,
                    ok,
                    tool_count,
                    protocol_version,
                    server_name,
                    server_version,
                    instructions,
                    error,
                    elapsed_ms,
                })?
            }

            _ => return Err(PluginError::NotFound(format!("未知路径: {path}"))),
        };

        Ok(PluginPayload::new(&data))
    }
}

/// HTTP test_connection 在 server 不可用时使用的协议版本占位字符串
const DEFAULT_PROTOCOL_VERSION_FALLBACK: &str = "unknown";

crate::submit_object_creator!(PLUGIN_MCP, McpPlugin::build, dyn Plugin);
