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
use crate::symbio_core::schemas::common;
use crate::symbio_core::{
    Capability, CapabilityMeta, InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin,
    PluginError, PluginMeta, PluginPayload, CONFIG_GET, CONFIG_SET, PLUGIN_MCP,
};
use crate::symbio_core::resources::{
    RESOURCES_DELETE, RESOURCES_LIST, RESOURCES_STATUS, RESOURCES_UPLOAD,
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

// ==================== 统一资源协议 (resources/*) ====================

impl McpPlugin {
    fn es(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
    ) -> Result<std::sync::Arc<dyn crate::symbio_core::providers::StorageService>, PluginError> {
        create_object::<dyn crate::symbio_core::providers::StorageService>(
            "storage_service",
            ctx.clone(),
        )
        .ok_or_else(|| PluginError::InternalError("storage_service 不可用".to_string()))
    }

    /// resources/list — 列出全部 MCP Server，统一 ResourceSummary 契约
    pub async fn resources_list(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
    ) -> Result<serde_json::Value, PluginError> {
        let store = self.es(ctx)?;
        let es = store.entity_store();
        let category = crate::symbio_core::providers::categories::MCP;
        let manifest = crate::symbio_core::providers::manifests::SERVER;

        let ids = es
            .list_entities(category)
            .await
            .map_err(|e| PluginError::InternalError(format!("列出 MCP 资源失败: {e}")))?;

        let mut items = Vec::new();
        for id in ids {
            let content = es.read_entity(category, &id, manifest).await.ok();
            let item = match content.and_then(|c| serde_json::from_str::<McpServerConfig>(&c).ok()) {
                Some(server) => {
                    let mut it = crate::symbio_core::resources::ResourceSummary::new(
                        crate::symbio_core::resources::RESOURCE_MCP,
                        &id,
                        &id,
                    );
                    it.status = if server.enabled {
                        "active".to_string()
                    } else {
                        "disabled".to_string()
                    };
                    it.summary = server
                        .command
                        .clone()
                        .or(server.url.clone());
                    let transport = format!("{:?}", server.transport_type).to_lowercase();
                    if let serde_json::Value::Object(ref mut m) = it.extra {
                        m.insert("transport".to_string(), transport.into());
                    }
                    it
                }
                None => crate::symbio_core::resources::ResourceSummary::new(
                    crate::symbio_core::resources::RESOURCE_MCP,
                    &id,
                    &id,
                ),
            };
            items.push(item);
        }

        let resp = crate::symbio_core::resources::ResourcesListResponse {
            kind: crate::symbio_core::resources::RESOURCE_MCP.to_string(),
            capabilities: crate::symbio_core::resources::capabilities_for(
                crate::symbio_core::resources::RESOURCE_MCP,
            ),
            items,
        };
        Ok(serde_json::to_value(resp)?)
    }

    /// resources/upload — 上传 zip 创建/更新 MCP Server（zip 根含 server.json）
    pub async fn resources_upload(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
    ) -> Result<serde_json::Value, PluginError> {
        let req: crate::symbio_core::resources::ResourceUploadRequest = ctx.payload()?;
        let name = req
            .name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| PluginError::ValidationError("MCP 资源名称不能为空".to_string()))?
            .to_string();
        let b64 = req.zip_b64.as_deref().ok_or_else(|| {
            PluginError::ValidationError("MCP 资源以 zip 上传（zip_b64）".to_string())
        })?;
        let bytes = crate::symbio_core::resources::decode_zip_b64(b64)?;

        let store = self.es(ctx)?;
        let es = store.entity_store();
        let category = crate::symbio_core::providers::categories::MCP;
        crate::symbio_core::resources::extract_zip_to_entity(es, category, &name, &bytes).await?;

        self.reload_server_from_storage(ctx, &name).await?;
        Ok(serde_json::to_value(
            crate::symbio_core::resources::ResourceUploadResponse {
                kind: crate::symbio_core::resources::RESOURCE_MCP.to_string(),
                id: name,
                created: true,
            },
        )?)
    }

    /// resources/delete — 删除 MCP Server（磁盘 + 内存 + 缓存）
    pub async fn resources_delete(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
    ) -> Result<serde_json::Value, PluginError> {
        let req: crate::symbio_core::resources::ResourceDeleteRequest = ctx.payload()?;
        let store = self.es(ctx)?;
        let es = store.entity_store();
        let category = crate::symbio_core::providers::categories::MCP;

        match es.delete_entity(category, &req.id).await {
            Ok(()) => {}
            Err(crate::symbio_core::providers::EntityStoreError::NotFound { .. }) => {
                crate::plugin_warn!("mcp", "磁盘上已无 server {} 目录，仅清理内存", req.id);
            }
            Err(e) => {
                return Err(PluginError::InternalError(format!("删除资源失败: {e}")));
            }
        }

        {
            let mut cfg = self.config.write().await;
            cfg.servers.remove(&req.id);
        }
        self.manager.forget_server(&req.id).await;

        Ok(serde_json::to_value(
            crate::symbio_core::resources::ResourceUploadResponse {
                kind: crate::symbio_core::resources::RESOURCE_MCP.to_string(),
                id: req.id,
                created: false,
            },
        )?)
    }

    /// resources/status — 连接测试单个 MCP Server
    ///
    /// 复用 `manager.test_connection` 发起真实连接校验（stdio 握手 / http streams），
    /// 返回统一 `ResourceStatusResponse`。不修改任何缓存或配置。
    pub async fn resources_status(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
    ) -> Result<serde_json::Value, PluginError> {
        let req: crate::symbio_core::resources::ResourceStatusRequest = ctx.payload()?;

        let server = {
            let store = self.es(ctx)?;
            let es = store.entity_store();
            let category = crate::symbio_core::providers::categories::MCP;
            let manifest = crate::symbio_core::providers::manifests::SERVER;
            let content = match es.read_entity(category, &req.id, manifest).await {
                Ok(c) => c,
                Err(e) => {
                    return Err(PluginError::NotFound(format!(
                        "未找到 MCP Server {}（读取失败: {e}）",
                        req.id
                    )))
                }
            };
            serde_json::from_str::<McpServerConfig>(&content).map_err(|e| {
                PluginError::InternalError(format!("解析 {} 配置失败: {e}", req.id))
            })?
        };

        let resp = match self.manager.test_connection(&req.id, &server).await {
            Ok(r) => crate::symbio_core::resources::ResourceStatusResponse {
                kind: crate::symbio_core::resources::RESOURCE_MCP.to_string(),
                id: req.id.clone(),
                status: "connected".to_string(),
                status_detail: Some(format!(
                    "{tools} tools · protocol {protocol}",
                    tools = r.tool_count,
                    protocol = r.protocol_version,
                )),
            },
            Err(e) => crate::symbio_core::resources::ResourceStatusResponse {
                kind: crate::symbio_core::resources::RESOURCE_MCP.to_string(),
                id: req.id.clone(),
                status: "failed".to_string(),
                status_detail: Some(e),
            },
        };

        // 通过 resource 事件总线把测试结果实时推送，供资源列表/详情即时刷新状态角标
        crate::symbio_core::event_bus::EventBus::publish_resource_status(
            crate::symbio_core::resources::RESOURCE_MCP,
            &req.id,
            &resp.status,
            resp.status_detail.clone(),
        )
        .await;

        Ok(serde_json::to_value(resp)?)
    }

    /// 上传后把单个 server 从磁盘回灌到内存 config（并失效相关缓存）
    pub async fn reload_server_from_storage(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
        name: &str,
    ) -> Result<(), PluginError> {
        let store = self.es(ctx)?;
        let es = store.entity_store();
        let category = crate::symbio_core::providers::categories::MCP;
        let manifest = crate::symbio_core::providers::manifests::SERVER;

        let content = es
            .read_entity(category, name, manifest)
            .await
            .map_err(|e| PluginError::InternalError(format!("读取已上传资源失败: {e}")))?;
        let server: McpServerConfig = serde_json::from_str(&content)
            .map_err(|e| PluginError::InternalError(format!("解析已上传资源失败: {e}")))?;

        {
            let mut cfg = self.config.write().await;
            cfg.servers.insert(name.to_string(), server);
        }
        self.manager.invalidate_discover_cache(name).await;
        self.manager.session_cache.remove(name).await;
        Ok(())
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

            // ============== 统一资源协议 (resources/*) ==============
            RESOURCES_LIST => self.resources_list(&ctx).await?,
            RESOURCES_UPLOAD => self.resources_upload(&ctx).await?,
            RESOURCES_DELETE => self.resources_delete(&ctx).await?,
            RESOURCES_STATUS => self.resources_status(&ctx).await?,

            _ => return Err(PluginError::NotFound(format!("未知路径: {path}"))),
        };

        Ok(PluginPayload::new(&data))
    }
}

crate::submit_object_creator!(PLUGIN_MCP, McpPlugin::build, dyn Plugin);
