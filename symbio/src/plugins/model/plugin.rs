//! MODEL Plugin - Core implementation
//!
//! 负责：
//! - 多 Model Provider 注册表（`ModelProvidersConfig`，持久化 serde schema）
//! - Provider 注册：traverse 时按上下文解析出唯一生效 Provider
//!   （ctx[PROVIDER_ID] > 默认 > 首个启用），绑定配置与协议实现为
//!   `BoundProvider`（core `ModelProvider` trait 的生产实现）注册进
//!   CAPABILITY_VISITOR。本插件只做无状态 LLM 网关：chat 编排与限流属于
//!   session 插件

use super::bound_provider::BoundProvider;
use super::model_providers::{ModelProviderConfig, ModelProvidersConfig};
use super::protocols::resolve_protocol_id;
use crate::symbio_core::schemas::common;
use crate::symbio_core::{
    create_object, InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginError,
    PluginMeta, PluginPayload, SimpleRequest, CONFIG_GET, CONFIG_SET, PLUGIN_MODEL,
};
use crate::{plugin_error, plugin_info, plugin_warn};
use async_trait::async_trait;
use serde_json::{json, Map, Value};
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;

/// Universal MODEL Agent Plugin
#[derive(Clone)]
pub struct ModelPlugin {
    /// 多 Model Provider 注册表
    providers: Arc<RwLock<ModelProvidersConfig>>,
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
        let mut marked_default: Option<String> = None;
        for id in &ids {
            match es.read_entity(category, id, manifest).await {
                Ok(content) => {
                    // 先解析为 Value 提取 is_default 标记，再解析为强类型配置
                    let parsed =
                        serde_json::from_str::<serde_json::Value>(&content).and_then(|v| {
                            serde_json::from_value::<ModelProviderConfig>(v.clone()).map(|p| (v, p))
                        });
                    match parsed {
                        Ok((raw, mut p)) => {
                            if p.id.is_empty() {
                                p.id = id.clone();
                            }
                            if p.name.is_empty() {
                                p.name = id.clone();
                            }
                            if raw
                                .get("is_default")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false)
                            {
                                marked_default = Some(id.clone());
                            }
                            new_providers.insert(id.clone(), p);
                        }
                        Err(e) => plugin_warn!("model", "解析 provider {id} 失败: {e}"),
                    }
                }
                Err(e) => plugin_warn!("model", "读取 provider {id} 失败: {e}"),
            }
        }

        // 4. 推算 default_provider_id（显式 is_default 标记 > 现有指向 > 首个可用）
        let existing_default = self.providers.read().await.default_provider_id.clone();
        let default_id = marked_default
            .or(existing_default)
            .or_else(|| {
                new_providers
                    .values()
                    .find(|p| p.enabled)
                    .map(|p| p.id.clone())
            })
            .or_else(|| new_providers.keys().next().cloned());

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
            parent: Arc::new(RwLock::new(parent)),
        }
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

    /// 验证给定的 Model Provider 配置（不写入状态）
    async fn validate_provider(provider: &ModelProviderConfig) -> Option<String> {
        Self::validate_config(provider).await
    }

    /// 验证配置是否可用
    ///
    /// 直调 `ModelProtocol::ping`（最小代价请求探测 endpoint / key / model
    /// 可用性），`Ok(())` → 配置可用；`Err(e)` → 携带失败原因。
    async fn validate_config(config: &ModelProviderConfig) -> Option<String> {
        let ctx = Arc::new(SimpleRequest::new(None, None));
        let protocol_id = resolve_protocol_id(&config.api_protocol);
        let protocol = create_object::<dyn super::protocols::ModelProtocol>(protocol_id, ctx)
            .expect("MODEL protocol creator not found");

        match protocol.ping(config).await {
            Ok(()) => None,
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

    /// 参与 `available_options` 收集：贡献「Model」选择项。
    ///
    /// 形态：`sub` 节点，子项 = 每个启用的 Provider；选中即把
    /// `metadata.provider_id` 落库（后端 `resolve_session_params` 按 metadata
    /// 回退解析，故会话发起无需前端传参）。当前选中值由宿主注入的
    /// `ctx[PROVIDER_ID]` 回填——本插件无需加载会话。
    async fn contribute_options(&self, ctx: &Arc<dyn InvokeRequest>) {
        use crate::symbio_core::schemas::options::OptionNode;

        let Some(visitor) = ctx.get(crate::symbio_core::OPTION_VISITOR) else {
            return;
        };

        // 展示顺序号段约定：30 = Model（见 session::options 模块文档）
        const ORDER: i32 = 30;

        let providers = self.providers.read().await;
        let requested = ctx
            .get(crate::symbio_core::PROVIDER_ID)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        // 生效 Provider：请求显式 > 默认（与后端解析链一致，供展示回填）
        let effective = requested.clone().or_else(|| {
            providers
                .resolve(providers.default_provider_id.as_deref())
                .map(|p| p.id.clone())
        });

        let mut enabled: Vec<&ModelProviderConfig> =
            providers.providers.values().filter(|p| p.enabled).collect();
        enabled.sort_by(|a, b| a.id.cmp(&b.id));

        let mut children: Vec<OptionNode> = Vec::with_capacity(enabled.len());
        let mut current_label: Option<String> = None;
        for p in &enabled {
            if effective.as_deref() == Some(p.id.as_str()) {
                current_label = Some(if p.name.is_empty() {
                    p.id.clone()
                } else {
                    p.name.clone()
                });
            }
            let model = if p.model.is_empty() {
                "未设置模型"
            } else {
                p.model.as_str()
            };
            children.push(
                OptionNode::session_state(
                    format!("model_provider:{}", p.id),
                    if p.name.is_empty() {
                        p.id.clone()
                    } else {
                        p.name.clone()
                    },
                    "provider_id",
                    json!(p.id),
                )
                .with_description(format!("{} · {}", p.provider, model)),
            );
        }

        let node = if children.is_empty() {
            // 无可用 Provider：仍下发节点（禁用态 + 引导文案），前端零特判
            OptionNode::sub("model_provider", "Model", Vec::new())
                .with_icon("model")
                .with_order(ORDER)
                .with_description("暂无可用 Model，请前往「设置 → 模型」添加")
                .with_status("disabled")
                .with_value_label("", "未配置")
        } else {
            let node = OptionNode::sub("model_provider", "Model", children)
                .with_icon("model")
                .with_order(ORDER)
                .with_description("选择本次会话使用的 Model Provider（含默认）");
            match current_label {
                Some(label) => node.with_value_label(effective.unwrap_or_default(), label),
                None => node.with_value(effective.unwrap_or_default()),
            }
        };

        visitor.register_option(node).await;
    }
}

impl Default for ModelPlugin {
    fn default() -> Self {
        Self::new(None, ModelProvidersConfig::default())
    }
}

// ==================== VDFS 挂载点（`.vdfs/model`） ====================
//
// 本插件**直接实现 `VdfsProvider`**：VDFS 是唯一协议、唯一地址空间，列 / 读 /
// 写 / 删 / 动作的语义都在这里表达（不再经实体层与适配器）。
// 跨插件共享的存储原语（写盘 / 删除）走 `symbio_core::entities` 的自由函数——
// 它们只做 `EntityStore` 的落盘；校验、内存注册表同步是 model 的差异部分。
//
// 列表项 `extra` 展开 `config`（完整 ModelProviderConfig）与 `is_default`，
// 使 chat 侧（`listModelProviders`）与 VDFS 详情共用同一读取入口。

use crate::symbio_core::entities::{self, EntitySummary, ENTITY_MODEL};
use crate::symbio_core::providers::{categories, manifests};
use crate::symbio_core::vdfs::{from_plugin_error, host_ctx, unwatch_changes, watch_changes};
use crate::symbio_core::vdfs_provider::{
    VdfsAccess, VdfsActionResult, VdfsChangeSink, VdfsContent, VdfsContext, VdfsError, VdfsNewType,
    VdfsNode, VdfsProvider, VdfsResult, VdfsWriteResponse, VFDS_ACTION_TEST, VFDS_EXT_FORM,
};

const LABEL: &str = "模型";

/// 路径末段 → 条目 id（去掉 `.<kind>` 呈现扩展名）
fn id_of(path: &str) -> String {
    let base = path.rsplit('/').next().unwrap_or(path);
    base.strip_suffix(&format!(".{ENTITY_MODEL}"))
        .unwrap_or(base)
        .to_string()
}

/// 摘要 → VDFS 节点（`ext = form` + 详情定义随节点 `schema` 下发）
fn node_of(item: &EntitySummary) -> VdfsNode {
    let mut n = VdfsNode::file(&item.id, item.name.clone(), VdfsAccess::READ_WRITE);
    n.kind = ENTITY_MODEL.to_string();
    n.ext = Some(VFDS_EXT_FORM.to_string());
    n.schema = serde_json::to_value(super::detail::model_detail_definition()).ok();
    n.status = item.status.clone();
    n.description = item.description.clone().or_else(|| item.summary.clone());
    n.updated_at = item.updated_at;
    n
}

/// 单个 Provider 的摘要 `extra`。
///
/// 详情页 `read`（经 `summarize`）与列表 `list_items`（经 `provider.list_items`）
/// 必须产出同一份 `extra`——尤其是完整 `config`——否则详情读不到配置、
/// 表单回退成「新建 Provider」空表单（VDFS 迁移后暴露：详情读经由
/// `summary_of → summarize`，而 model 此前只在 `list_items` 里填了 config）。
fn model_summary_extra(p: &ModelProviderConfig, is_default: bool) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("provider".to_string(), json!(p.provider));
    m.insert("model".to_string(), json!(p.model));
    m.insert("api_protocol".to_string(), json!(p.api_protocol));
    m.insert("temperature".to_string(), json!(p.temperature));
    m.insert("is_default".to_string(), json!(is_default));
    if let Ok(Value::Object(o)) = serde_json::to_value(p) {
        m.insert("config".to_string(), Value::Object(o));
    }
    m
}

impl ModelPlugin {
    /// 内存注册表里的一条配置 → 摘要。
    ///
    /// 详情 `read` 与列表 `list` 必须产出同一份 `extra`——尤其是完整 `config`——
    /// 否则详情读不到配置、表单回退成「新建 Provider」空表单。
    async fn summary_of_config(&self, p: &ModelProviderConfig) -> EntitySummary {
        let mut it = EntitySummary::new(ENTITY_MODEL, &p.id, p.name.clone());
        it.status = if p.enabled {
            "active".to_string()
        } else {
            "disabled".to_string()
        };
        it.description = Some(p.model.clone());
        let is_default = {
            let providers = self.providers.read().await;
            providers.default_provider_id.as_deref() == Some(p.id.as_str())
        };
        if let Value::Object(ref mut m) = it.extra {
            m.extend(model_summary_extra(p, is_default));
        }
        it
    }

    /// 读盘 → 摘要（`stat` / `read` / `delete` 的存在性校验都走这里）
    async fn summary_of(
        &self,
        host: &Arc<dyn InvokeRequest>,
        id: &str,
    ) -> VdfsResult<EntitySummary> {
        let content = entities::read_manifest(host, categories::MODEL, manifests::PROVIDER, id)
            .await
            .map_err(from_plugin_error)?;
        Ok(
            match serde_json::from_str::<ModelProviderConfig>(&content).ok() {
                Some(raw) => self.summary_of_config(&raw).await,
                None => EntitySummary::new(ENTITY_MODEL, id, id),
            },
        )
    }

    /// 表单 manifest → 规范化配置（写盘前的校验：字段缺省补全 + 连接校验）
    async fn validate_manifest(&self, id: &str, manifest: &Value) -> Result<Value, PluginError> {
        let mut provider: ModelProviderConfig = serde_json::from_value(manifest.clone())
            .map_err(|e| PluginError::ValidationError(format!("Provider 配置无效: {e}")))?;
        if provider.id.is_empty() {
            provider.id = id.to_string();
        }
        if provider.name.is_empty() {
            provider.name = id.to_string();
        }
        if provider.provider.is_empty() {
            return Err(PluginError::ValidationError(
                "Provider 的 provider 字段不能为空".to_string(),
            ));
        }

        // 保存前必须校验连接（manifest 携带 skip_validation=true 时跳过）
        let skip_validation = manifest
            .get("skip_validation")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !skip_validation {
            if let Some(err) = Self::validate_provider(&provider).await {
                plugin_error!("model", format!("Provider 配置验证未通过: {}", err));
                return Err(PluginError::ValidationError(format!(
                    "Provider 配置验证失败: {err}"
                )));
            }
        }

        let mut v =
            serde_json::to_value(&provider).map_err(|e| PluginError::ParseError(e.to_string()))?;
        // 保留「设为默认」标记（写盘 + 内存同步时读取；skip_validation 不落盘）
        if manifest
            .get("is_default")
            .and_then(|b| b.as_bool())
            .unwrap_or(false)
        {
            v["is_default"] = serde_json::json!(true);
        }
        Ok(v)
    }

    /// VDFS 新建（`write { create }`）的最小配置：先落一份「可用的默认 Provider」，
    /// 用户随后在详情里填 key / 调模型。默认字段取预设首项——与新建表单「选中
    /// 第一个预设」的预填**同源**，两条链路创建出的初始配置因此一致。
    fn new_manifest(&self, id: &str, title: &str) -> Value {
        let (provider, api_base, model, api_protocol) = super::detail::default_provider_fields();
        let mut cfg = ModelProviderConfig {
            id: id.to_string(),
            name: if title.is_empty() {
                id.to_string()
            } else {
                title.to_string()
            },
            provider: provider.to_string(),
            api_base: api_base.to_string(),
            model: model.to_string(),
            ..Default::default()
        };
        if !api_protocol.is_empty() {
            cfg.api_protocol = api_protocol.to_string();
        }
        let mut v =
            serde_json::to_value(&cfg).unwrap_or_else(|_| json!({ "id": id, "name": title }));
        // 新建态用户尚未填写 key / base，跳过连接校验。
        // `validate_manifest` 消费该标记后**丢弃**（不落盘）。
        if let Value::Object(ref mut m) = v {
            let _ = m.insert("skip_validation".to_string(), Value::Bool(true));
        }
        v
    }

    /// 写盘后同步内存注册表（回读磁盘 + 默认 provider 兜底 + 触发父级持久化）
    async fn on_uploaded(&self, host: &Arc<dyn InvokeRequest>, id: &str) -> Result<(), PluginError> {
        let content = entities::read_manifest(host, categories::MODEL, manifests::PROVIDER, id)
            .await
            .map_err(|e| PluginError::InternalError(format!("回读 provider 失败: {e}")))?;
        let raw: Value = serde_json::from_str(&content)
            .map_err(|e| PluginError::ParseError(format!("回读 provider 解析失败: {e}")))?;
        // 「设为默认」标记（validate_manifest 保留、随 manifest 落盘）
        let is_default = raw
            .get("is_default")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let provider: ModelProviderConfig = serde_json::from_value(raw)
            .map_err(|e| PluginError::ParseError(format!("回读 provider 解析失败: {e}")))?;
        {
            let mut providers = self.providers.write().await;
            providers.providers.insert(id.to_string(), provider);
            if is_default || providers.default_provider_id.is_none() {
                providers.default_provider_id = Some(id.to_string());
            }
        }
        let _ = self.persist_to_parent(host).await;
        Ok(())
    }

    /// 删除后清理内存注册表与默认 provider 指向
    async fn on_deleted(&self, id: &str) {
        let mut providers = self.providers.write().await;
        providers.providers.remove(id);
        if providers.default_provider_id.as_deref() == Some(id) {
            providers.default_provider_id = None;
        }
    }

    /// 连接测试（复用 `validate_provider`）：失败也返回 `Ok`，由 `message` 承载原因
    async fn test_of(&self, id: &str) -> Result<(bool, String), PluginError> {
        let provider = {
            let providers = self.providers.read().await;
            providers.providers.get(id).cloned()
        }
        .ok_or_else(|| PluginError::NotFound(format!("未找到 Model Provider: {id}")))?;
        Ok(match Self::validate_provider(&provider).await {
            None => (
                true,
                format!("校验通过（{} / {}）", provider.provider, provider.model),
            ),
            Some(e) => (false, e),
        })
    }
}

#[async_trait]
impl VdfsProvider for ModelPlugin {
    fn label(&self) -> Option<&str> {
        Some(LABEL)
    }

    fn description(&self) -> Option<&str> {
        Some("Model Provider 配置（每项一份 provider.json），是大模型接入的唯一来源。")
    }

    fn order(&self) -> i32 {
        2
    }

    fn icon(&self) -> Option<&str> {
        Some(ENTITY_MODEL)
    }

    /// 根下只能新建「模型」条目（model 不支持整包导入）
    fn root_new_types(&self) -> Vec<VdfsNewType> {
        vec![VdfsNewType::new(ENTITY_MODEL, LABEL).with_description(format!(
            "新建{LABEL}（先落一份默认配置，随后在详情里完善）"
        ))]
    }

    async fn list(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<Vec<VdfsNode>> {
        if !path.is_empty() {
            return Err(VdfsError::not_found(format!(
                "{LABEL}是叶子资源，没有子项：{path}"
            )));
        }
        // 列表来自内存注册表（启动时镜像磁盘）
        let providers = self.providers.read().await;
        let mut out = Vec::with_capacity(providers.providers.len());
        for p in providers.providers.values() {
            out.push(node_of(&self.summary_of_config(p).await));
        }
        Ok(out)
    }

    async fn stat(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsNode> {
        if path.is_empty() {
            // 自身根：名字留空——provider 不知道自己的挂载名，由使用方回填
            return Ok(VdfsNode::dir("", LABEL, VdfsAccess::LIST));
        }
        let host = host_ctx(ctx)?;
        Ok(node_of(&self.summary_of(&host, &id_of(path)).await?))
    }

    async fn read(&self, ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsContent> {
        if path.is_empty() {
            return Err(VdfsError::invalid(format!(
                "该路径是目录，不可读取内容：{path}"
            )));
        }
        let host = host_ctx(ctx)?;
        let item = self.summary_of(&host, &id_of(path)).await?;
        let value = item
            .extra
            .get("config")
            .cloned()
            .unwrap_or_else(|| item.extra.clone());
        let text = serde_json::to_string_pretty(&value)
            .map_err(|e| VdfsError::internal(format!("配置序列化失败：{e}")))?;
        Ok(VdfsContent::text("", text).with_mime("application/json"))
    }

    async fn write(
        &self,
        ctx: &VdfsContext,
        path: &str,
        content: &VdfsContent,
    ) -> VdfsResult<VdfsWriteResponse> {
        let host = host_ctx(ctx)?;
        if path.is_empty() {
            return Err(VdfsError::invalid(format!(
                "{LABEL}不支持在挂载根上写入：{path}"
            )));
        }
        if content.binary {
            return Err(VdfsError::invalid(format!("{LABEL}不支持整包导入（zip）")));
        }
        let id = id_of(path);
        let manifest = if content.create {
            // 新建：使用方只给了路径名，最小配置由本插件自持
            self.new_manifest(&id, &id)
        } else {
            serde_json::from_str::<Value>(content.as_text().unwrap_or_default())
                .map_err(|e| VdfsError::invalid(format!("manifest 不是合法 JSON：{e}")))?
        };
        let normalized = self
            .validate_manifest(&id, &manifest)
            .await
            .map_err(from_plugin_error)?;
        let resp = entities::write_entity_manifest(
            &host,
            ENTITY_MODEL,
            categories::MODEL,
            manifests::PROVIDER,
            &id,
            &normalized,
        )
        .await
        .map_err(from_plugin_error)?;
        self.on_uploaded(&host, &id).await.map_err(from_plugin_error)?;
        Ok(VdfsWriteResponse {
            path: id,
            created: resp.created,
            etag: None,
        })
    }

    async fn delete(&self, ctx: &VdfsContext, path: &str, _recursive: bool) -> VdfsResult<()> {
        if path.is_empty() {
            return Err(VdfsError::Forbidden(format!("不可删除挂载点：{path}")));
        }
        let host = host_ctx(ctx)?;
        let id = id_of(path);
        // 存在性校验：删除不存在的条目应报 NotFound 而非静默成功
        self.summary_of(&host, &id).await?;
        entities::delete_entity_dir(&host, ENTITY_MODEL, categories::MODEL, &id)
            .await
            .map_err(from_plugin_error)?;
        self.on_deleted(&id).await;
        Ok(())
    }

    async fn action(
        &self,
        ctx: &VdfsContext,
        path: &str,
        action: &str,
        _payload: Option<&Value>,
    ) -> VdfsResult<VdfsActionResult> {
        match action {
            VFDS_ACTION_TEST => {
                if path.is_empty() {
                    return Err(VdfsError::invalid(format!(
                        "「测试连接」只对{LABEL}条目可用：{path}"
                    )));
                }
                let host = host_ctx(ctx)?;
                let id = id_of(path);
                // 存在性校验：测试不存在的条目应报 NotFound 而非成功
                self.summary_of(&host, &id).await?;
                let (ok, detail) = self.test_of(&id).await.map_err(from_plugin_error)?;
                Ok(VdfsActionResult {
                    action: VFDS_ACTION_TEST.to_string(),
                    ok,
                    message: detail,
                    data: None,
                })
            }
            _ => Err(VdfsError::NotImplemented),
        }
    }

    async fn watch(&self, _ctx: &VdfsContext, path: &str, sink: VdfsChangeSink) -> VdfsResult<()> {
        watch_changes(ENTITY_MODEL, path, sink).await
    }

    async fn unwatch(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<()> {
        unwatch_changes(ENTITY_MODEL, path).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 路径末段才是 id：`.vdfs/model/<id>.model` 与裸 `<id>` 同解
    #[test]
    fn id_of_strips_presentation_extension() {
        assert_eq!(id_of("openai-1"), "openai-1");
        assert_eq!(id_of("openai-1.model"), "openai-1");
        assert_eq!(id_of("a/b/openai-1.model"), "openai-1");
    }

    /// 新建的最小清单必须带 `skip_validation`：此时用户还没填 key / base，
    /// 走连接校验必然失败（新建态不该被校验挡住）。
    #[test]
    fn new_manifest_skips_validation() {
        let v = ModelPlugin::default().new_manifest("p1", "p1");
        assert_eq!(v.get("id").and_then(|x| x.as_str()), Some("p1"));
        assert_eq!(v.get("skip_validation").and_then(|x| x.as_bool()), Some(true));
    }

    /// 回归：详情 `read` 经 `summarize` 必须产出 `extra.config`（完整配置）。
    /// 否则详情表单拿不到字段值、标题回退成「新建 Provider」空表单。
    #[test]
    fn summary_extra_carries_full_config() {
        let cfg = ModelProviderConfig {
            id: "openai-1".into(),
            name: "我的 OpenAI".into(),
            provider: "openai".into(),
            api_base: "https://api.openai.com/v1".into(),
            model: "gpt-4o".into(),
            ..Default::default()
        };
        let extra = model_summary_extra(&cfg, true);
        assert_eq!(
            extra.get("provider").and_then(|v| v.as_str()),
            Some("openai")
        );
        assert_eq!(extra.get("model").and_then(|v| v.as_str()), Some("gpt-4o"));
        assert_eq!(
            extra.get("is_default").and_then(|v| v.as_bool()),
            Some(true)
        );
        // 关键断言：完整 config 必须随摘要下发，详情读才能拿到字段值
        let config = extra.get("config").expect("extra.config 必须存在");
        assert_eq!(config.get("id").and_then(|v| v.as_str()), Some("openai-1"));
        assert_eq!(config.get("model").and_then(|v| v.as_str()), Some("gpt-4o"));
        assert_eq!(
            config.get("provider").and_then(|v| v.as_str()),
            Some("openai")
        );
    }

    #[test]
    fn summary_extra_config_matches_serialized_model() {
        let cfg = ModelProviderConfig {
            id: "x".into(),
            name: "X".into(),
            provider: "anthropic".into(),
            model: "claude".into(),
            ..Default::default()
        };
        let extra = model_summary_extra(&cfg, false);
        let full = serde_json::to_value(&cfg).unwrap();
        assert_eq!(extra.get("config"), Some(&full));
        assert_eq!(
            extra.get("is_default").and_then(|v| v.as_bool()),
            Some(false)
        );
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
            // `config/schema` / `status` / `chat_sync` 已下线：
            // - 字段定义改由 `detail_definition`（随 VDFS 节点 `schema` 下发）；
            // - 连通性自检改由节点动作 `vdfs/action { action: "test" }`；
            // - `chat_sync` 本就是 NotImplemented 占位，chat 族归 session 插件。
            _ => Err(PluginError::NotFound(format!("未知路径: {path}"))),
        }
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        // 模型能力纳入统一注册收集机制：与 local/web/mcp 插件注册工具同构。
        // 命中 TRAVERSE_AVAILABLE_TOOLS 时，按上下文解析出**唯一生效** Provider
        // （ctx[PROVIDER_ID] > default_provider_id > 首个 enabled），构造运行期
        // `ModelProvider` 注册进 CAPABILITY_VISITOR；其系统提示词同时注册在
        // provider_id 键与 "default" 键（消费侧两键均兜底）。
        let sub_path = ctx.get(crate::symbio_core::PATH).unwrap_or_default();
        match sub_path.as_str() {
            // 选项收集（与能力收集同一广播机制的第二通道）：贡献「Model」选择项
            crate::symbio_core::TRAVERSE_AVAILABLE_OPTIONS => {
                self.contribute_options(&ctx).await;
                return Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()));
            }
            crate::symbio_core::TRAVERSE_AVAILABLE_TOOLS => {}
            other => {
                return Err(PluginError::NotFound(format!("未知遍历路径: {other}")));
            }
        }

        if let Some(tool_visitor) = ctx.get(crate::symbio_core::CAPABILITY_VISITOR) {
            // VDFS 挂载点：本插件自身就是 provider（`.vdfs/model`）——
            // 列 / 读 / 写 / 删 / 动作直接由 `impl VdfsProvider for ModelPlugin` 承载
            let vdfs_provider: Arc<dyn VdfsProvider> = self.clone();
            tool_visitor
                .register_vdfs_provider(PLUGIN_MODEL, vdfs_provider)
                .await;

            let providers = self.providers.read().await;
            // 解析唯一生效 Provider：请求显式指定 > 默认 > 首个启用
            let requested = ctx
                .get(crate::symbio_core::PROVIDER_ID)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            match providers.resolve(requested.as_deref()).cloned() {
                Some(p) => {
                    let protocol_id = resolve_protocol_id(&p.api_protocol);
                    match create_object::<dyn super::protocols::ModelProtocol>(
                        protocol_id,
                        ctx.clone(),
                    ) {
                        Some(protocol) => {
                            // 系统提示词双键注册（provider_id 键 + "default" 键兜底）
                            // 在移动 p 之前读取
                            let system_prompt = p.system_prompt.clone();
                            let provider_id = p.id.clone();
                            let provider = Arc::new(BoundProvider::new(p, protocol));
                            tool_visitor.register_model_provider(provider).await;
                            if let Some(sp) = &system_prompt {
                                tool_visitor
                                    .register_system_prompt(&provider_id, sp.clone())
                                    .await;
                                // "default" 键兜底：消费侧提示词解析链的首选键
                                tool_visitor
                                    .register_system_prompt("default", sp.clone())
                                    .await;
                            }
                        }
                        None => {
                            // 协议工厂不可用：软故障（不进致命错误桶），消费侧报"未找到可用的 Model Provider"
                            plugin_warn!(
                                "model",
                                "traverse: provider '{}' 协议工厂不可用（protocol_id={protocol_id}），跳过注册",
                                p.id
                            );
                        }
                    }
                }
                None => {
                    plugin_warn!(
                        "model",
                        "traverse: 无可用 Model Provider（requested={requested:?}），跳过注册"
                    );
                }
            }
        }

        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}
