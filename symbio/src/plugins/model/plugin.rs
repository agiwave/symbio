//! MODEL Plugin - Core implementation
//!
//! 负责：
//! - 单个活动 Model Provider 配置管理（向后兼容）
//! - 多 Model Provider 注册表（`ModelProvidersConfig`）
//! - 限流器：每个 Provider 独立的最小请求间隔
//! - chat 路由：解析请求中的 `provider_id` 并按注册表选定配置发起请求

use super::handlers;
use super::protocols::resolve_protocol_id;
use crate::symbio_core::schemas::common;
use crate::symbio_core::schemas::model::model_config::ModelConfig;
use crate::symbio_core::schemas::model::model_providers::{ModelProviderConfig, ModelProvidersConfig};
use crate::symbio_core::{
    create_object, InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginChannel,
    PluginError, PluginFrame, PluginMeta, PluginPayload, SimpleRequest, CONFIG_GET, CONFIG_SET,
    PLUGIN_MODEL,
};
use crate::{plugin_debug, plugin_error, plugin_info, plugin_warn};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};

/// 限流器：按 provider_id 记录"上次发起请求的时间"
///
/// 设计原则：
/// - 进程内单例（每个 Provider 一个时间戳）
/// - 加锁粒度：per-provider 单独互斥，避免 Provider A 的限流影响 Provider B
#[derive(Default)]
pub struct ProviderRateLimiter {
    last_request: Mutex<HashMap<String, Instant>>,
}

impl ProviderRateLimiter {
    pub fn new() -> Self {
        Self::default()
    }

    /// 阻塞等待直到距上次请求至少 `min_interval_ms` 毫秒
    ///
    /// - `min_interval_ms == 0` 时直接放行
    /// - **首次请求立即放行**，仅记录时间戳；后续请求才按 `min_interval_ms` 节流
    pub async fn wait(&self, provider_id: &str, min_interval_ms: u64) {
        if min_interval_ms == 0 {
            return;
        }
        let interval = Duration::from_millis(min_interval_ms);
        loop {
            let now = Instant::now();
            // 计算本次需要 sleep 的时长（None = 立即放行）
            let sleep_for: Option<Duration> = {
                let mut map = self.last_request.lock().await;
                match map.get(provider_id) {
                    None => {
                        // 首次请求：立即放行，仅记录时间戳供后续节流
                        map.insert(provider_id.to_string(), now);
                        None
                    }
                    Some(&last) => {
                        let elapsed = now.saturating_duration_since(last);
                        if elapsed >= interval {
                            // 已超过最小间隔：放行并刷新时间戳
                            map.insert(provider_id.to_string(), now);
                            None
                        } else {
                            Some(interval - elapsed)
                        }
                    }
                }
            };
            match sleep_for {
                None => return,
                Some(remaining) => {
                    // 提前 5ms 唤醒避免睡过头；若剩余时间已 < 5ms 则直接重试
                    let sleep = remaining.saturating_sub(Duration::from_millis(5));
                    if sleep.is_zero() {
                        continue;
                    }
                    tokio::time::sleep(sleep).await;
                }
            }
        }
    }
}

/// Universal MODEL Agent Plugin
#[derive(Clone)]
pub struct ModelPlugin {
    /// 多 Model Provider 注册表
    providers: Arc<RwLock<ModelProvidersConfig>>,
    /// Provider 维度限流器
    rate_limiter: Arc<ProviderRateLimiter>,
    /// 父插件引用（用于能力路由）
    parent: Arc<RwLock<Option<Weak<dyn Plugin>>>>,
}

impl ModelPlugin {
    /// 静态工厂：从 InvokeRequest 构造 Plugin 实例
    ///
    /// 加载策略（按优先级）：
    /// 1. **新存储**：从 `~/.symbio/ais/<id>/provider.json` 加载所有 Provider
    /// 2. **回退到 ctx.config()**：home 通过 composite 传入的 MODEL 节点（用于旧 config.yaml 兼容）
    pub fn build(ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Plugin> {
        // 同步 fallback：先从 ctx.config() 解析（兼容旧 config.yaml）
        let providers_config: ModelProvidersConfig = ctx
            .config()
            .and_then(|v| serde_json::from_value::<ModelProvidersConfig>(v).ok())
            .unwrap_or_default();

        let parent = ctx.parent();
        let plugin = Arc::new(Self::new(parent, providers_config));

        // 启动后异步触发：从新存储加载（并触发首启动数据迁移）
        let plugin_weak = Arc::downgrade(&plugin);
        let ctx_clone = ctx.clone();
        tokio::spawn(async move {
            if let Some(ai) = plugin_weak.upgrade() {
                ai.load_from_storage(&ctx_clone).await;
            }
        });

        plugin as Arc<dyn Plugin>
    }

    /// 异步加载：从存储服务拉取所有 Provider
    ///
    /// 启动时调用此方法，**会**触发首启动数据迁移（从 config.yaml 迁到新存储）。
    pub async fn load_from_storage(&self, ctx: &Arc<dyn InvokeRequest>) {
        let store = match create_object::<dyn crate::symbio_core::providers::StorageService>(
            "storage_service",
            ctx.clone(),
        ) {
            Some(s) => s,
            None => {
                plugin_warn!("model", "未找到 storage_service，跳过新存储加载");
                return;
            }
        };

        let es = store.entity_store();
        let category = crate::symbio_core::providers::categories::MODEL;
        let manifest = crate::symbio_core::providers::manifests::PROVIDER;

        // 1. 列出新存储中的所有 Provider
        let ids = match es.list_entities(category).await {
            Ok(v) => v,
            Err(e) => {
                plugin_warn!("model", "list models 失败: {e}");
                return;
            }
        };

        // 1.5 兼容旧分类 `ai`：若 model 分类为空但 MODEL 分类有数据，自动迁移
        if ids.is_empty() {
            let legacy_category = "ai";
            if let Ok(legacy_ids) = es.list_entities(legacy_category).await {
                if !legacy_ids.is_empty() {
                    plugin_info!(
                        "model",
                        "检测到旧分类 '{}' 中有 {} 个 Provider，正在迁移到 '{}'",
                        legacy_category,
                        legacy_ids.len(),
                        category
                    );
                    for legacy_id in &legacy_ids {
                        if let Ok(content) =
                            es.read_entity(legacy_category, legacy_id, manifest).await
                        {
                            // 写入新分类
                            if let Err(e) = es
                                .write_entity(category, legacy_id, manifest, &content)
                                .await
                            {
                                plugin_warn!("model", "迁移 provider {legacy_id} 失败: {e}");
                            }
                        }
                    }
                    // 重新读取新分类
                    if let Ok(new_ids) = es.list_entities(category).await {
                        if !new_ids.is_empty() {
                            // 删除旧分类下的数据
                            for legacy_id in &legacy_ids {
                                let _ = es.delete_entity(legacy_category, legacy_id).await;
                            }
                            // 递归重入：用 Box::pin 避免无限大小 future
                            return Box::pin(self.load_from_storage(ctx)).await;
                        }
                    }
                }
            }
        }

        // 2. 如果新存储为空，触发首启动迁移
        if ids.is_empty() {
            self.migrate_from_legacy_config(&*store).await;
            return;
        }

        // 3. 加载新存储的内容
        let mut new_providers = std::collections::HashMap::new();
        for id in &ids {
            match es.read_entity(category, id, manifest).await {
                Ok(content) => match serde_json::from_str::<ModelProviderConfig>(&content) {
                    Ok(mut p) => {
                        if p.id.is_empty() {
                            p.id = id.clone();
                        }
                        if p.name.is_empty() {
                            p.name = id.clone();
                        }
                        new_providers.insert(id.clone(), p);
                    }
                    Err(e) => plugin_warn!("model", "解析 provider {id} 失败: {e}"),
                },
                Err(e) => plugin_warn!("model", "读取 provider {id} 失败: {e}"),
            }
        }

        // 4. 推算 default_provider_id
        let default_id = {
            let cfg = self.providers.read().await;
            cfg.default_provider_id
                .clone()
                .or_else(|| {
                    new_providers
                        .values()
                        .find(|p| p.enabled)
                        .map(|p| p.id.clone())
                })
                .or_else(|| new_providers.keys().next().cloned())
        };

        let mut cfg = self.providers.write().await;
        cfg.providers = new_providers;
        cfg.default_provider_id = default_id;
        plugin_info!(
            "model",
            "从 ~/.symbio/plugins/model/ 加载了 {} 个 Model Provider",
            cfg.providers.len()
        );
    }

    /// 首启动迁移：从 ctx.config() 中残留的旧配置迁到新存储
    async fn migrate_from_legacy_config(
        &self,
        store: &dyn crate::symbio_core::providers::StorageService,
    ) {
        let current = self.providers.read().await.clone();
        if current.providers.is_empty() {
            return;
        }

        plugin_info!(
            "model",
            "检测到旧 config 中的 Model Providers，开始迁移到 ~/.symbio/plugins/model/"
        );

        let es = store.entity_store();
        let category = crate::symbio_core::providers::categories::MODEL;
        let manifest = crate::symbio_core::providers::manifests::PROVIDER;

        for (id, p) in &current.providers {
            let content = match serde_json::to_string_pretty(p) {
                Ok(s) => s,
                Err(_e) => {
                    plugin_error!("model", "序列化 provider {id} 失败");
                    continue;
                }
            };
            if let Err(_e) = es.write_entity(category, id, manifest, &content).await {
                plugin_error!("model", "迁移 provider {id} 失败");
            }
        }
    }

    /// 主构造函数（Factory 机制使用）
    pub fn new(parent: Option<Weak<dyn Plugin>>, providers: ModelProvidersConfig) -> Self {
        Self {
            providers: Arc::new(RwLock::new(providers)),
            rate_limiter: Arc::new(ProviderRateLimiter::new()),
            parent: Arc::new(RwLock::new(parent)),
        }
    }

    pub fn config_schema() -> Value {
        json!({
            "type": "object",
            "required": ["provider", "model"],
            "properties": {
                "provider": {
                    "type": "string",
                    "title": "LLM 提供商",
                    "description": "供应商标识 (如 openMODEL, anthropic, lmstudio, ollama 等)",
                    "examples": ["openai", "anthropic", "lmstudio", "ollama"]
                },
                "api_base": { "type": "string", "title": "API 基础路径", "description": "API 基础路径" },
                "api_key": { "type": "string", "title": "API 密钥", "description": "API 密钥", "sensitive": true },
                "model": { "type": "string", "title": "模型名称", "description": "默认模型名称" },
                "api_protocol": {
                    "type": "string",
                    "title": "协议类型",
                    "description": "使用的 API 协议",
                    "default": "openai_responses"
                },
                "temperature": { "type": "number", "title": "温度", "minimum": 0, "maximum": 2, "default": 0.7 },
                "max_tokens": { "type": "integer", "title": "最大 Token", "minimum": 1, "default": 8192 },
                "system_prompt": { "type": "string", "title": "系统提示词", "description": "全局系统提示词" }
            }
        })
    }

    /// 解析请求中的 provider_id，依次回退到 default / 第一个 enabled
    pub async fn resolve_provider(&self, provider_id: Option<&str>) -> Option<ModelProviderConfig> {
        let cfg = self.providers.read().await;
        cfg.resolve(provider_id).cloned()
    }

    /// 获取父插件引用
    async fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        let guard = self.parent.read().await;
        guard.as_ref().and_then(|w| w.upgrade())
    }

    /// 触发父插件持久化（save_config 路由）
    async fn persist_to_parent(&self, ctx: &Arc<dyn InvokeRequest>) -> InvokeResponse<()> {
        if let Some(p) = self.get_parent().await {
            let save_ctx = ctx.fork();
            save_ctx.set(crate::symbio_core::PATH, "save_config".to_string());
            p.route(save_ctx).await?;
        } else {
            plugin_warn!("model", "未找到父插件，配置仅在内存中生效");
        }
        Ok(())
    }

    /// 持久化到新存储（按 Provider 子目录）
    ///
    /// 实际数据落盘到 `~/.symbio/plugins/model/<id>/provider.json`，
    /// 而非传统的 `config.yaml`。这是新存储策略的核心入口。
    async fn persist_to_disk(&self, ctx: &Arc<dyn InvokeRequest>) {
        let store = create_object::<dyn crate::symbio_core::providers::StorageService>(
            "storage_service",
            ctx.clone(),
        );
        let Some(store) = store else {
            plugin_warn!("model", "未找到 storage_service，跳过新存储写入");
            return;
        };

        let es = store.entity_store();
        let category = crate::symbio_core::providers::categories::MODEL;
        let manifest = crate::symbio_core::providers::manifests::PROVIDER;

        let providers = self.providers.read().await;

        // 1. 写入每个 Provider
        for (id, p) in &providers.providers {
            let content = match serde_json::to_string_pretty(p) {
                Ok(s) => s,
                Err(_e) => {
                    plugin_error!("model", "序列化 provider {id} 失败");
                    continue;
                }
            };
            if let Err(_e) = es.write_entity(category, id, manifest, &content).await {
                plugin_error!("model", "写入 provider {id} 失败");
            }
        }
    }

    /// 验证给定的 Model Provider 配置（不写入状态）
    async fn validate_provider(
        provider: &ModelProviderConfig,
        parent: &Option<Arc<dyn Plugin>>,
    ) -> Option<String> {
        let cfg = provider.to_model_config();
        Self::validate_config(&cfg, parent).await
    }

    async fn handle_chat_session(
        &self,
        channel: PluginChannel,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<()> {
        let tx = channel.tx.clone();
        if let Err(e) = self.handle_chat_session_internal(channel, ctx).await {
            plugin_error!("model", format!("Chat session error: {}", e));
            let _ = tx.send(e.to_frame()).await;
            return Err(e);
        }
        Ok(())
    }

    async fn handle_chat_session_internal(
        &self,
        channel: PluginChannel,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<()> {
        // 解析 provider_id → 选定本次会话的活动 Provider
        //
        // payload 是必填项（由 agent/handlers/chat.rs 构造），但为了健壮性，
        // 在缺少 payload 时使用 Default 而不中断整个 chat 流程。
        let req: crate::symbio_core::schemas::model::model_chat::Request = ctx
            .payload::<crate::symbio_core::schemas::model::model_chat::Request>()
            .unwrap_or_default();

        plugin_info!(
            "model",
            "[DIAG] handle_chat_session_internal entered, requested provider_id={:?}",
            req.provider_id
        );

        let provider_cfg = self
            .resolve_provider(req.provider_id.as_deref())
            .await
            .ok_or_else(|| {
                PluginError::ValidationError(
                    "未找到可用的 Model Provider，请先在设置中配置并启用至少一个".to_string(),
                )
            })?;

        plugin_info!(
            "model",
            "[DIAG] resolved provider: id={}, api_base={}, model={}, api_protocol={}, rate_limit_ms={}",
            provider_cfg.id,
            provider_cfg.api_base,
            provider_cfg.model,
            provider_cfg.api_protocol,
            provider_cfg.rate_limit_ms
        );

        // 限流：按 provider_id 维度等待最小请求间隔
        self.rate_limiter
            .wait(&provider_cfg.id, provider_cfg.rate_limit_ms)
            .await;

        plugin_info!("model", "[DIAG] rate limiter passed, creating protocol");

        let ai_cfg = provider_cfg.to_model_config();
        let parent = self.get_parent().await;

        let protocol = create_object::<dyn super::protocols::ModelProtocol>(
            resolve_protocol_id(&ai_cfg.api_protocol),
            ctx.clone(),
        )
        .expect("MODEL protocol creator not found");
        plugin_info!(
            "model",
            "[DIAG] protocol created, calling handle_chat_stream, api_url={}",
            protocol.get_api_url(&ai_cfg)
        );
        let resp = protocol.handle_chat_stream(&ai_cfg, &parent, ctx).await?;
        plugin_info!(
            "model",
            "[DIAG] handle_chat_stream returned, matching payload"
        );

        match resp {
            PluginPayload::Data(_) => {
                if let Ok(value) = resp.get::<serde_json::Value>() {
                    let _ = channel.tx.send(PluginFrame::Data(value)).await;
                }
            }
            PluginPayload::Session(peer) => {
                let (peer_tx, mut peer_rx) = (peer.tx, peer.rx);
                let (my_tx, mut my_rx) = (channel.tx, channel.rx);

                // Forward: peer -> my (MODEL -> Client)
                let forward_task = tokio::spawn(async move {
                    while let Some(frame) = peer_rx.recv().await {
                        if my_tx.send(frame).await.is_err() {
                            break;
                        }
                    }
                });

                // Backward: my -> peer (Client -> AI)
                let backward_task = tokio::spawn(async move {
                    while let Some(frame) = my_rx.recv().await {
                        plugin_debug!("model", "incoming frame from client: {:?}", frame);
                        if peer_tx.send(frame).await.is_err() {
                            break;
                        }
                    }
                });

                // Wait for either task to finish or errors
                tokio::select! {
                    _ = forward_task => {},
                    _ = backward_task => {},
                }
            }
            _ => {
                let _ = channel
                    .tx
                    .send(PluginFrame::Error(
                        "Invalid payload type for chat session".into(),
                        None,
                    ))
                    .await;
            }
        }
        Ok(())
    }

    /// 验证配置是否可用
    async fn validate_config(
        config: &ModelConfig,
        parent: &Option<Arc<dyn Plugin>>,
    ) -> Option<String> {
        let ctx = Arc::new(SimpleRequest::new(None, None));
        let protocol = create_object::<dyn super::protocols::ModelProtocol>(
            resolve_protocol_id(&config.api_protocol),
            ctx.clone(),
        )
        .expect("MODEL protocol creator not found");

        let validate_input = protocol.get_validation_input();
        let _ = ctx.set_payload(validate_input);

        match protocol.handle_chat_stream(config, parent, ctx).await {
            Ok(resp) => match resp {
                PluginPayload::Data(_) => None,
                PluginPayload::Session(mut peer) => {
                    while let Some(frame) = peer.rx.recv().await {
                        match frame {
                            PluginFrame::Data(d) => {
                                if let Ok(event) = serde_json::from_value::<
                                    crate::symbio_core::schemas::session::session_chat_response::StreamEvent,
                                >(d)
                                {
                                    match event {
                                        crate::symbio_core::schemas::session::session_chat_response::StreamEvent::Error { error } => {
                                            return Some(error);
                                        }
                                        crate::symbio_core::schemas::session::session_chat_response::StreamEvent::Status { status }
                                            if status == "idle" => {
                                                return None;
                                            }
                                        _ => {}
                                    }
                                }
                            }
                            PluginFrame::Error(e, _) => return Some(e),
                        }
                    }
                    None
                }
                _ => None,
            },
            Err(e) => Some(e.to_string()),
        }
    }
}

impl ModelPlugin {
    pub fn metadata() -> PluginMeta {
        PluginMeta::new("model", "MODEL 核心引擎")
            .with_description("Universal MODEL Agent Engine (LLM API Router)")
            .with_version("0.3.0")
    }
}

impl Default for ModelPlugin {
    fn default() -> Self {
        Self::new(None, ModelProvidersConfig::default())
    }
}

// ==================== 统一资源协议 (resources/*) ====================

impl ModelPlugin {
    /// resources/list — 列出全部 Model Provider（统一 ResourceSummary 契约）
    ///
    /// 列表项 `extra` 中展开 `config`（完整 `ModelProviderConfig`）与 `is_default`，
    /// 使 chat 侧（`listModelProviders`）与资源管理页共用同一读取入口，无需再走
    /// 遗留 `providers/list`。
    pub async fn resources_model_list(&self) -> InvokeResponse<PluginPayload> {
        let providers = self.providers.read().await;
        let items = providers
            .providers
            .values()
            .map(|p| {
                let mut it = crate::symbio_core::resources::ResourceSummary::new(
                    crate::symbio_core::resources::RESOURCE_MODEL,
                    &p.id,
                    p.name.clone(),
                );
                it.status = if p.enabled {
                    "active".to_string()
                } else {
                    "disabled".to_string()
                };
                it.description = Some(p.model.clone());
                let is_default = providers.default_provider_id.as_deref() == Some(p.id.as_str());
                if let serde_json::Value::Object(ref mut m) = it.extra {
                    let _ = m.insert("provider".to_string(), serde_json::json!(p.provider));
                    let _ = m.insert("model".to_string(), serde_json::json!(p.model));
                    let _ = m.insert(
                        "api_protocol".to_string(),
                        serde_json::json!(p.api_protocol),
                    );
                    let _ = m.insert("temperature".to_string(), serde_json::json!(p.temperature));
                    let _ = m.insert("is_default".to_string(), serde_json::json!(is_default));
                    if let Ok(cfg) = serde_json::to_value(p) {
                        let _ = m.insert("config".to_string(), cfg);
                    }
                }
                it
            })
            .collect::<Vec<_>>();

        let resp = crate::symbio_core::resources::ResourcesListResponse {
            kind: crate::symbio_core::resources::RESOURCE_MODEL.to_string(),
            capabilities: crate::symbio_core::resources::capabilities_for(
                crate::symbio_core::resources::RESOURCE_MODEL,
            ),
            items,
        };
        Ok(PluginPayload::new(&resp))
    }

    /// resources/upload — 以 JSON 表单（manifest）创建/更新 Model Provider
    pub async fn resources_model_upload(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        let req: crate::symbio_core::resources::ResourceUploadRequest = ctx.payload()?;
        let id = req
            .name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| PluginError::ValidationError("Model 资源名称不能为空".to_string()))?
            .to_string();
        let manifest = req.manifest.ok_or_else(|| {
            PluginError::ValidationError("Model 以 JSON 表单（manifest）上传".to_string())
        })?;
        let mut provider: ModelProviderConfig = serde_json::from_value(manifest)
            .map_err(|e| PluginError::ValidationError(format!("Provider 配置无效: {e}")))?;
        if provider.id.is_empty() {
            provider.id = id.clone();
        }
        if provider.name.is_empty() {
            provider.name = id.clone();
        }
        if provider.provider.is_empty() {
            return Err(PluginError::ValidationError(
                "Provider 的 provider 字段不能为空".to_string(),
            ));
        }

        // 对齐旧 `providers/set`：保存前校验连接 + 落盘到新存储
        let parent = self.get_parent().await;
        if let Some(err) = Self::validate_provider(&provider, &parent).await {
            plugin_error!("model", format!("Provider 配置验证未通过: {}", err));
            return Err(PluginError::ValidationError(format!(
                "Provider 配置验证失败: {err}"
            )));
        }

        let existed = {
            let mut providers = self.providers.write().await;
            let existed = providers.providers.contains_key(&id);
            providers.providers.insert(id.clone(), provider);
            if providers.default_provider_id.is_none() {
                providers.default_provider_id = Some(id.clone());
            }
            existed
        };
        self.persist_to_disk(ctx).await;
        let _ = self.persist_to_parent(ctx).await;

        Ok(PluginPayload::new(
            &crate::symbio_core::resources::ResourceUploadResponse {
                kind: crate::symbio_core::resources::RESOURCE_MODEL.to_string(),
                id,
                created: !existed,
            },
        ))
    }

    /// resources/delete — 删除 Model Provider（内存 + 磁盘）
    pub async fn resources_model_delete(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        let req: crate::symbio_core::resources::ResourceDeleteRequest = ctx.payload()?;
        {
            let mut providers = self.providers.write().await;
            providers.providers.remove(&req.id);
            if providers.default_provider_id.as_deref() == Some(req.id.as_str()) {
                providers.default_provider_id = None;
            }
        }
        if let Some(store) = create_object::<
            dyn crate::symbio_core::providers::StorageService,
        >("storage_service", ctx.clone())
        {
            let es = store.entity_store();
            let category = crate::symbio_core::providers::categories::MODEL;
            if let Err(e) = es.delete_entity(category, &req.id).await {
                crate::plugin_warn!("model", "删除 provider 目录失败: {e}");
            }
        }
        Ok(PluginPayload::new(
            &crate::symbio_core::resources::ResourceUploadResponse {
                kind: crate::symbio_core::resources::RESOURCE_MODEL.to_string(),
                id: req.id,
                created: false,
            },
        ))
    }

    /// resources/status — 连接测试单个 Model Provider
    ///
    /// 复用 `validate_provider` 发起真实连接校验（对齐旧 `providers/test`），
    /// 返回统一 `ResourceStatusResponse`；不修改任何注册表或配置。
    pub async fn resources_model_status(
        &self,
        ctx: &Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        let req: crate::symbio_core::resources::ResourceStatusRequest = ctx.payload()?;
        let provider = {
            let providers = self.providers.read().await;
            providers.providers.get(&req.id).cloned()
        }
        .ok_or_else(|| PluginError::NotFound(format!("未找到 Model Provider: {}", req.id)))?;

        let parent = self.get_parent().await;
        let resp = match Self::validate_provider(&provider, &parent).await {
            None => crate::symbio_core::resources::ResourceStatusResponse {
                kind: crate::symbio_core::resources::RESOURCE_MODEL.to_string(),
                id: req.id.clone(),
                status: "connected".to_string(),
                status_detail: Some(format!(
                    "校验通过（{} / {}）",
                    provider.provider, provider.model
                )),
            },
            Some(e) => crate::symbio_core::resources::ResourceStatusResponse {
                kind: crate::symbio_core::resources::RESOURCE_MODEL.to_string(),
                id: req.id.clone(),
                status: "failed".to_string(),
                status_detail: Some(e),
            },
        };

        // 通过 resource 事件总线把测试结果实时推送，供资源列表/详情即时刷新状态角标
        crate::symbio_core::event_bus::EventBus::publish_resource_status(
            crate::symbio_core::resources::RESOURCE_MODEL,
            &req.id,
            &resp.status,
            resp.status_detail.clone(),
        )
        .await;

        Ok(PluginPayload::new(&resp))
    }
}

crate::submit_object_creator!(PLUGIN_MODEL, ModelPlugin::build, dyn Plugin);

#[async_trait]
impl Plugin for ModelPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let path = ctx.get(crate::symbio_core::PATH).unwrap_or_default();
        match path.as_str() {
            CONFIG_GET => {
                // 新存储策略：实际 provider 数据存放在
                // `~/.symbio/plugins/model/<id>/provider.json`，不在 config.yaml 中。
                //
                // 这里只返回**元数据**（default_provider_id），让 home 的
                // save_config 不会把完整 provider 写回 config.yaml。
                let providers = self.providers.read().await.clone();
                let metadata = serde_json::json!({
                    "default_provider_id": providers.default_provider_id,
                    "plugin_provider": "model",
                    "plugin_name": "model",
                    "_storage": "plugins/model",  // 标记数据已迁移到新存储
                });
                Ok(PluginPayload::new(&metadata))
            }
            CONFIG_SET => {
                let new_cfg: ModelProvidersConfig = ctx.payload()?;
                {
                    let mut providers = self.providers.write().await;
                    *providers = new_cfg;
                }
                self.persist_to_parent(&ctx).await?;
                Ok(PluginPayload::new(&common::SuccessResponse::default()))
            }
            "config/schema" => Ok(PluginPayload::new(&common::SchemaResponse {
                schema: Self::config_schema(),
            })),

            "chat" => {
                let (my_channel, peer_channel) = PluginChannel::pair(64);
                let plugin = self.clone();
                let ctx_clone = ctx.fork();
                tokio::spawn(async move {
                    if let Err(e) = plugin.handle_chat_session(my_channel, ctx_clone).await {
                        plugin_error!("model", format!("Chat session error: {}", e));
                    }
                });
                Ok(PluginPayload::Session(peer_channel))
            }
            "chat_sync" => Err(PluginError::NotImplemented),
            "status" => {
                let providers = self.providers.read().await;
                let active = providers
                    .resolve(providers.default_provider_id.as_deref())
                    .cloned()
                    .map(|p| p.to_model_config())
                    .unwrap_or_default();
                Ok(PluginPayload::new(&handlers::handle_status(&active)))
            }

            // ============== 统一资源协议 (resources/*，independent_form 启用于 model) ==============
            crate::symbio_core::resources::RESOURCES_LIST => {
                self.resources_model_list().await
            }
            crate::symbio_core::resources::RESOURCES_UPLOAD => {
                self.resources_model_upload(&ctx).await
            }
            crate::symbio_core::resources::RESOURCES_DELETE => {
                self.resources_model_delete(&ctx).await
            }
            crate::symbio_core::resources::RESOURCES_STATUS => {
                self.resources_model_status(&ctx).await
            }

            _ => Err(PluginError::NotFound(format!("未知路径: {path}"))),
        }
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        // Model 插件目前不直接暴露工具
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}
