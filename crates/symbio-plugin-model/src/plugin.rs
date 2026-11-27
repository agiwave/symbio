//! MODEL Plugin - Core implementation
//!
//! 负责：
//! - 多 Model Provider 注册表（`ModelProvidersConfig`，运行期内存视图）
//! - Provider 注册：traverse 时按上下文解析出唯一生效 Provider
//!   （`ctx[PROVIDER_ID]` > 默认 > 首个启用），绑定配置与协议实现为
//!   `BoundProvider`（本插件 `ModelProvider` 契约的唯一实现）注册进
//!   CAPABILITY_VISITOR。本插件只做无状态 LLM 网关：chat 编排与限流属于
//!   session 插件

use super::bound_provider::BoundProvider;
use super::model_providers::{ModelProviderConfig, ModelProvidersConfig};
use super::protocols::resolve_protocol_id;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use symbio_plugin_api::PLUGIN_ID_MODEL;
use symbio_plugin_api::{
    capability_report_error, creator_create_object, plugin_dir_from_ctx, CapabilityRow, Plugin,
    PluginDir, PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PluginInvokeResponse,
    PluginMeta, PluginPayload, PluginSimpleRequest,
};
use symbio_plugin_api::{DetailField, DetailOption};
use symbio_providers::{MemoryVdfs, SingleFileVdfs};
use tokio::sync::RwLock;
use {
    symbio_plugin_api::plugin_error, symbio_plugin_api::plugin_info, symbio_plugin_api::plugin_warn,
};

/// Provider 主文件名（磁盘布局：`<本插件目录>/<id>/provider.json`）
const MANIFEST: &str = "provider.json";

/// 跨条目的插件配置（`<本插件目录>/PLUGIN.yml`）
///
/// 只存**跨条目的状态**：单个 Provider 的明细是**资源**，落在
/// `<本插件目录>/<id>/provider.json`，不进配置文件。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ModelConfig {
    #[serde(default)]
    pub default_provider_id: Option<String>,
}

/// Universal MODEL Agent Plugin
#[derive(Clone)]
pub struct ModelPlugin {
    /// 多 Model Provider 注册表（chat 侧解析唯一生效 Provider 用）
    providers: Arc<RwLock<ModelProvidersConfig>>,
    /// 本插件的目录（`<本插件目录>`）——配置文件 `PLUGIN.yml` 就在这里
    dir: PluginDir,
    /// VDFS 侧条目清单（内存镜像：`id → provider.json` 原文）
    ///
    /// 规范 §13.4：model 的列表来自内存。镜像与注册表**同一处更新**
    /// （`publish_uploaded`），因此不存在第二个写入者。
    entries: MemoryVdfs,
}

impl ModelPlugin {
    /// 静态工厂：从 PluginInvokeRequest 构造 Plugin 实例
    ///
    /// 加载策略（按优先级）：
    /// 1. **存储**：从 `<本插件目录>/<id>/provider.json` 加载所有 Provider
    /// 2. **跨条目配置**：`<本插件目录>/PLUGIN.yml` 里的 `default_provider_id`
    pub fn build(ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        let dir = plugin_dir_from_ctx(&*ctx, PLUGIN_ID_MODEL);
        let providers_config: ModelProvidersConfig = match dir.load::<ModelConfig>() {
            Ok(Some(cfg)) => ModelProvidersConfig {
                default_provider_id: cfg.default_provider_id,
                ..Default::default()
            },
            Ok(None) => ModelProvidersConfig::default(),
            Err(e) => {
                plugin_warn!("model", "读取自身配置失败，改用默认值：{e}");
                ModelProvidersConfig::default()
            }
        };

        let plugin = Arc::new(Self::new(providers_config, dir));

        // 启动后异步触发：从存储加载 Provider 明细
        let plugin_weak = Arc::downgrade(&plugin);
        let ctx_clone = ctx.clone();
        tokio::spawn(async move {
            if let Some(ai) = plugin_weak.upgrade() {
                ai.load_from_storage(&ctx_clone).await;
            }
        });

        plugin as Arc<dyn Plugin>
    }

    /// 磁盘底座（每次现取，跟随 homedir 切换）
    ///
    /// 根 = **本插件自己的目录**（构造时由父插件经 `PLUGIN_DIR` 告知）——
    /// 这里不按插件名反推落位，插件不知道、也不该知道自己被放在哪。
    fn store(&self) -> SingleFileVdfs {
        SingleFileVdfs::at(self.dir.dir(), PLUGIN_ID_MODEL, MANIFEST).with_label(LABEL)
    }

    /// 异步加载：从存储拉取所有 Provider
    ///
    /// 注册表与 VDFS 镜像一次性同构填充——两份视图同源，任一时刻不会各说各话。
    pub async fn load_from_storage(&self, _ctx: &Arc<dyn PluginInvokeRequest>) {
        let store = self.store();
        let _guard = store.mutation_guard().await;
        let entries = match store.entries_locked().await {
            Ok(v) => v,
            Err(e) => {
                plugin_warn!("model", "list models 失败: {e}");
                return;
            }
        };

        let mut new_providers = std::collections::HashMap::new();
        let mut mirror = Vec::with_capacity(entries.len());
        for e in &entries {
            let Some(text) = e.raw.as_deref() else {
                plugin_warn!("model", "读取 provider {} 失败：主文件不可用", e.id);
                continue;
            };
            // 地址段是身份权威；旧文件里的 id 不改变资源落点。
            let parsed = serde_json::from_str::<Value>(text).and_then(|v| {
                serde_json::from_value::<ModelProviderConfig>(with_id(&v, &e.id)).map(|p| (v, p))
            });
            match parsed {
                Ok((_, mut p)) => {
                    if p.name.is_empty() {
                        p.name = e.id.clone();
                    }
                    new_providers.insert(e.id.clone(), p);
                    mirror.push((e.id.clone(), text.to_string()));
                }
                Err(err) => plugin_warn!("model", "解析 provider {} 失败: {err}", e.id),
            }
        }

        // PLUGIN.yml 只保存显式指针；失效指针不落盘改写，由启用项按 id 顺序计算有效默认。
        let mut cfg = self.providers.write().await;
        cfg.providers = new_providers;
        self.entries.replace_all(mirror);
        drop(cfg);
        plugin_info!(
            "model",
            "从 <本插件目录>/ 加载了 {} 个 Model Provider",
            self.entries.ids().len()
        );
    }

    /// 主构造函数（Factory 机制使用）
    pub fn new(providers: ModelProvidersConfig, dir: PluginDir) -> Self {
        let entries = MemoryVdfs::new(PLUGIN_ID_MODEL).with_label(LABEL);
        Self {
            providers: Arc::new(RwLock::new(providers)),
            dir,
            entries,
        }
    }

    /// 默认指针的唯一持久权威是插件自己的 `PLUGIN.yml`。
    fn persist_default(&self, id: &str) -> Result<(), PluginError> {
        let cfg = ModelConfig {
            default_provider_id: Some(id.to_string()),
        };
        self.dir.save(&cfg).map_err(PluginError::InternalError)
    }

    /// 计价配置的规范化与校验。
    ///
    /// 表单的 `form` widget 总会提交一个子对象键，所以「没填」到达这里时是
    /// `{}`（空币种 + 零单价）。它必须被规范化回 **`None`**——否则「没定价」
    /// 会在台账里变成一次「明确免费」，两者靠币种是否为空分辨。
    ///
    /// 币种非空而单价为零是合法状态（免费端点）：账本记一次 `charges`、金额 0。
    fn normalize_pricing(provider: &mut ModelProviderConfig) -> Result<(), PluginError> {
        let Some(mut pricing) = provider.pricing.take() else {
            return Ok(());
        };
        let currency = pricing.currency.trim().to_uppercase();
        let zero = pricing.input_per_million == 0.0 && pricing.output_per_million == 0.0;
        if currency.is_empty() {
            if zero {
                return Ok(());
            }
            return Err(PluginError::ValidationError(
                "计价填了单价就必须填币种（不填则整笔不记账）".to_string(),
            ));
        }
        for (label, value) in [
            ("输入单价", pricing.input_per_million),
            ("输出单价", pricing.output_per_million),
        ] {
            if !value.is_finite() || value < 0.0 {
                return Err(PluginError::ValidationError(format!(
                    "{label}必须是非负的有限数"
                )));
            }
        }
        pricing.currency = currency;
        provider.pricing = Some(pricing);
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
        let Some(protocol_id) = resolve_protocol_id(&config.api_protocol) else {
            // 保存前就报：写错的 api_protocol 不得落盘成一条「看起来能用」的配置。
            return Some(unsupported_protocol_message(&config.api_protocol));
        };
        let ctx = Arc::new(PluginSimpleRequest::new(None, None));
        let Some(protocol) =
            creator_create_object::<dyn super::protocols::ModelProtocol>(protocol_id, ctx)
        else {
            return Some(format!("协议实现未注册：{protocol_id}"));
        };

        match protocol.ping(config).await {
            Ok(()) => None,
            Err(e) => Some(e.to_string()),
        }
    }
}

/// 未识别 `api_protocol` 的用户可读消息（配置校验与能力收集共用一句话）。
///
/// 把「支持哪些值」写在这里而不是让调用方各自拼：错误文案也是判据的一部分，
/// 两处各写一份就会一边列全、一边漏项。
fn unsupported_protocol_message(api_protocol: &str) -> String {
    format!(
        "未识别的 api_protocol \"{api_protocol}\"：支持 openai_chat / openai_responses / \
         anthropic_messages / gemini_api（历史别名 chat / responses / anthropic / gemini）"
    )
}

impl ModelPlugin {
    pub fn metadata() -> PluginMeta {
        PluginMeta::new("model", "模型")
            .with_description(
                "Model Provider 配置（每项一份 provider.json），是大模型接入的唯一来源。",
            )
            .with_version("0.3.0")
            .with_order(2)
            .with_icon(r#"<circle cx="12" cy="12" r="10"/><circle cx="12" cy="12" r="3"/><path d="M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z"/>"#)
        // 「根下可新建类型」由 provider 自持（根节点自述里的 `VdfsNode::new_type`，
        // 见下方 `impl VdfsProvider for ModelPlugin`）——它是挂载点的动态自述，容器
        // 合成根节点时向 provider 发一次 `Stat` 现场取，不进这份同步纯数据
    }

    /// 参与 `available_options` 收集：贡献「Model」字段。
    ///
    /// 候选 = 每个启用的 Provider（值 = `provider_id`）；`default` 取**生效
    /// Provider**（请求显式 > 默认）——这一步的解析链只有后端知道
    /// （`providers.default_provider_id`），故由后端把结论写进定义：会话 metadata
    /// 里没有 `provider_id` 时，前端按 `default` 显示生效的那个（见
    /// 设计文档 §6）。
    ///
    /// **不回填当前值**：值来自会话 `metadata.provider_id`，这里只声明候选与缺省。
    async fn contribute_options(&self, ctx: &Arc<dyn PluginInvokeRequest>) {
        let Some(visitor) = ctx.get(symbio_plugin_api::OPTION_VISITOR) else {
            return;
        };

        // 展示顺序号段约定：30 = Model（见 session::options 模块文档）
        const ORDER: i32 = 30;

        let providers = self.providers.read().await;
        let requested = ctx
            .get(symbio_plugin_api::PROVIDER_ID)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        // 生效 Provider：请求显式 > 默认（与后端解析链一致）
        let effective = requested.clone().or_else(|| {
            providers
                .resolve(providers.default_provider_id.as_deref())
                .map(|p| p.id.clone())
        });

        let mut enabled: Vec<&ModelProviderConfig> =
            providers.providers.values().filter(|p| p.enabled).collect();
        enabled.sort_by(|a, b| a.id.cmp(&b.id));

        visitor
            .put(CapabilityRow::option(
                ORDER,
                provider_field(&enabled, effective.as_deref()),
            ))
            .await;
    }
}

/// 「Model」选项的字段声明（`node.schema` 用）。
///
/// 无可用 Provider 时下发**单个占位候选**「未配置」（说明指向设置页），而不是为它
/// 新增「无条件禁用」这种机制。
fn provider_field(enabled: &[&ModelProviderConfig], effective: Option<&str>) -> DetailField {
    let options = if enabled.is_empty() {
        vec![DetailOption {
            value: String::new(),
            label: "未配置".to_string(),
            description: Some("暂无可用 Model，请前往「设置 → 模型」添加".to_string()),
        }]
    } else {
        enabled
            .iter()
            .map(|p| DetailOption {
                value: p.id.clone(),
                label: if p.name.is_empty() {
                    p.id.clone()
                } else {
                    p.name.clone()
                },
                description: Some(format!(
                    "{} · {}",
                    p.provider,
                    if p.model.is_empty() {
                        "未设置模型"
                    } else {
                        p.model.as_str()
                    }
                )),
            })
            .collect()
    };

    DetailField {
        key: "provider_id".to_string(),
        label: "Model".to_string(),
        description: Some("选择本次会话使用的 Model Provider（含默认）".to_string()),
        widget: "select".to_string(),
        icon: Some("model".to_string()),
        default: effective.map(|id| json!(id)),
        options,
        ..Default::default()
    }
}

/// 无装配上下文的实例（仅测试）。目录给临时目录——**不读全局系统根**。
#[cfg(test)]
impl Default for ModelPlugin {
    fn default() -> Self {
        Self::new(
            ModelProvidersConfig::default(),
            PluginDir::at(
                std::env::temp_dir().join("symbio-test/model"),
                PLUGIN_ID_MODEL,
            ),
        )
    }
}

// ==================== VDFS 挂载点（`<根>/model`） ====================
//
// 本插件**直接实现 `VdfsProvider`**：VDFS 是唯一协议、唯一地址空间，列 / 读 /
// 写 / 删 / 动作的语义都在这里表达。
//
// 存储不自建抽象：落盘走 `providers::SingleFileVdfs`（一个条目 =
// 一份 `provider.json`），清单走 `MemoryVdfs`（内存镜像，规范 §13.4「model 的列表
// 来自内存」）。于是本模块只剩 **model 特有的三件事**：详情定义随节点下发、
// 写前的连接校验、写后对 chat 侧注册表与内存镜像的同步。
//
// 分工与旧写法一致：`list` 读内存镜像，`stat` / `read` / `delete` / `action`
// 读磁盘（真相源），避免镜像与磁盘在校验路径上出现分歧。

use symbio_plugin_api::{vdfs_from_plugin_error, vdfs_unwatch_changes, vdfs_watch_changes};
use symbio_plugin_api::{
    VdfsAccess, VdfsActionResult, VdfsContent, VdfsContext, VdfsError, VdfsNewType, VdfsNode,
    VdfsProvider, VdfsRequest, VdfsResponse, VdfsResult, VdfsWriteResponse, VDFS_ACTION_TEST,
    VDFS_EXT_FORM, VDFS_STATUS_ACTIVE, VDFS_STATUS_DISABLED,
};

const LABEL: &str = "模型";

/// 详情定义（JSON 形态）——**唯一出处**：节点 `schema` 与新建类型 `schema` 都读它，
/// 因此「点新建」的草稿表单与「选中一项」的详情表单是同一张（用户第 1 点）。
fn detail_definition() -> Value {
    serde_json::to_value(super::detail::model_detail_definition()).unwrap_or(Value::Null)
}

/// 配置 → VDFS 节点（`ext = form` + 详情定义随节点 `schema` 下发）
///
/// 标题取 `name`、副标题取 `model`、状态取 `enabled`——这三样是 model 的呈现
/// 差异，`vdfs_service` 不知道也不该知道。
fn node_of(p: &ModelProviderConfig, updated_at: Option<i64>) -> VdfsNode {
    let mut n = VdfsNode::file(&p.id, p.name.clone(), VdfsAccess::READ_WRITE);
    n.kind = PLUGIN_ID_MODEL.to_string();
    n.ext = Some(VDFS_EXT_FORM.to_string());
    n.schema = Some(detail_definition());
    n.status = if p.enabled {
        VDFS_STATUS_ACTIVE.to_string()
    } else {
        VDFS_STATUS_DISABLED.to_string()
    };
    n.description = Some(p.model.clone());
    n.updated_at = updated_at;
    n
}

fn with_default(mut node: VdfsNode, is_default: bool) -> VdfsNode {
    node.attributes
        .insert("is_default".to_string(), Value::Bool(is_default));
    node
}

/// 地址中的 id 是身份权威，表单省略 id 时也无需调用方另行补齐。
fn with_id(manifest: &Value, id: &str) -> Value {
    let mut m = manifest.clone();
    if let Value::Object(map) = &mut m {
        map.insert("id".to_string(), json!(id));
    }
    m
}

/// 既有资源的顶层补丁：省略保留、显式替换；敏感空值与旧标记有专门语义。
fn merge_manifest(id: &str, previous: Option<&str>, patch: &Value) -> Result<Value, PluginError> {
    let previous = previous
        .map(serde_json::from_str::<Value>)
        .transpose()
        .map_err(|e| PluginError::ValidationError(format!("既有 Provider 配置无效: {e}")))?
        .unwrap_or_else(|| json!({}));
    let previous = previous
        .as_object()
        .ok_or_else(|| PluginError::ValidationError("既有 Provider 配置必须是对象".into()))?;
    let patch = patch
        .as_object()
        .ok_or_else(|| PluginError::ValidationError("Provider 补丁必须是对象".into()))?;
    let mut merged = previous.clone();
    for (key, value) in patch {
        if key != "is_default" {
            merged.insert(key.clone(), value.clone());
        }
    }
    let blank_key = patch.get("api_key").is_none_or(|value| {
        value.is_null() || value.as_str().is_some_and(|key| key.trim().is_empty())
    });
    if blank_key {
        match previous.get("api_key") {
            Some(value) => {
                merged.insert("api_key".to_string(), value.clone());
            }
            None => {
                merged.remove("api_key");
            }
        }
    }
    merged.insert("id".to_string(), json!(id));
    Ok(Value::Object(merged))
}

/// 主文件原文 → 配置（标题回落磁盘段名）
fn config_of(id: &str, text: &str) -> Option<ModelProviderConfig> {
    let v = serde_json::from_str::<Value>(text).ok()?;
    let mut p = serde_json::from_value::<ModelProviderConfig>(with_id(&v, id)).ok()?;
    if p.name.is_empty() {
        p.name = id.to_string();
    }
    Some(p)
}

impl ModelPlugin {
    /// 内存镜像里的全部配置（列表的唯一来源）
    ///
    /// 解析失败的条目**列不出来**而不是列成空壳：宁可少一项，也不给前端一个
    /// 点开就是坏详情的入口。
    async fn mirrored_nodes(&self) -> Vec<VdfsNode> {
        let mut out = Vec::new();
        let providers = self.providers.read().await;
        let default_id = providers.resolve(None).map(|provider| provider.id.clone());
        for id in self.entries.ids() {
            let text = self.entries.get(&id).unwrap_or_default();
            if let Some(p) = config_of(&id, &text) {
                out.push(with_default(
                    node_of(&p, self.entries.updated_at(&id)),
                    default_id.as_deref() == Some(id.as_str()),
                ));
            }
        }
        out
    }

    /// 读盘 → 配置（`stat` / `read` / `delete` / `action` 的存在性校验都走这里）
    async fn config_on_disk(&self, id: &str) -> VdfsResult<ModelProviderConfig> {
        let store = self.store();
        let entry = store.entry(id).await?;
        config_of(id, entry.raw.as_deref().unwrap_or_default())
            .ok_or_else(|| VdfsError::internal(format!("Provider「{id}」的清单不是合法配置")))
    }

    /// 表单 manifest → 规范化配置（写盘前的校验：补 id + 字段缺省 + 连接校验）
    async fn validate_manifest(&self, id: &str, manifest: &Value) -> Result<Value, PluginError> {
        let mut provider: ModelProviderConfig = serde_json::from_value(with_id(manifest, id))
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
        Self::normalize_pricing(&mut provider)?;

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
        // 强类型规范化不吞掉未来扩展键；控制键只属于本次调用。
        if let (Some(normalized), Some(input)) = (v.as_object_mut(), manifest.as_object()) {
            for (key, value) in input {
                if key != "skip_validation" && key != "is_default" && !normalized.contains_key(key)
                {
                    normalized.insert(key.clone(), value.clone());
                }
            }
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

    /// 在存储 mutation lock 内同步两份内存视图（chat 侧注册表 + VDFS 镜像）。
    ///
    /// 普通保存不再顺带写 PLUGIN.yml：默认指针只由显式动作改变，避免两个文件部分提交。
    async fn publish_uploaded(&self, id: &str, provider: &ModelProviderConfig, text: &str) {
        let mut providers = self.providers.write().await;
        providers.providers.insert(id.to_string(), provider.clone());
        self.entries.set(id, text.to_string());
    }

    async fn save_manifest(&self, id: &str, manifest: &Value) -> VdfsResult<(bool, Value)> {
        let id = id.to_string();
        let manifest = manifest.clone();
        let update_id = id.clone();
        let update_manifest = manifest.clone();
        let update_plugin = self.clone();
        let publish_id = id.clone();
        let publish_plugin = self.clone();
        let (created, (normalized, _, _)) = self
            .store()
            .update_json(
                &id,
                move |previous| async move {
                    let merged = merge_manifest(&update_id, previous.as_deref(), &update_manifest)
                        .map_err(vdfs_from_plugin_error)?;
                    let normalized = update_plugin
                        .validate_manifest(&update_id, &merged)
                        .await
                        .map_err(vdfs_from_plugin_error)?;
                    let provider = serde_json::from_value::<ModelProviderConfig>(
                        normalized.clone(),
                    )
                    .map_err(|e| {
                        vdfs_from_plugin_error(PluginError::ParseError(format!(
                            "规范化结果回读失败: {e}"
                        )))
                    })?;
                    let text = serde_json::to_string_pretty(&normalized).map_err(|e| {
                        vdfs_from_plugin_error(PluginError::ParseError(e.to_string()))
                    })?;
                    Ok((normalized.clone(), (normalized, provider, text)))
                },
                move |(_, provider, text)| async move {
                    publish_plugin
                        .publish_uploaded(&publish_id, &provider, &text)
                        .await;
                    symbio_plugin_api::vdfs_notify_change(PLUGIN_ID_MODEL, &publish_id);
                },
            )
            .await?;
        Ok((created, normalized))
    }

    /// 删除后在同一存储串行边界内清理两份内存视图。
    async fn after_deleted(&self, id: &str) {
        let mut providers = self.providers.write().await;
        providers.providers.remove(id);
        self.entries.remove(id);
    }

    /// 路径末段 → 条目 id（去掉 `.<kind>` 呈现扩展名）
    fn id_of(path: &str) -> String {
        symbio_providers::vdfs_id_of(path, PLUGIN_ID_MODEL)
    }

    /// 目标地址 → 条目 id（**唯一**判据，`write` 与测试共用）。
    ///
    /// 两种目标形态见 [`VdfsRequest::Write`]：
    /// 地址末段非空 ⇒ 名字由使用方给（`id_of` 按呈现扩展名剥后缀）；地址为空
    /// ⇒ **使用方没给名字**（写挂载点目录自身），id 由本插件生成——这正是
    /// 「点新建，直接进详情页填，保存时一次写入」的机制形态。
    /// 目录自身没有可覆盖的目标，所以必须带 `create` 意图。
    fn resolve_id(path: &str, create: bool) -> VdfsResult<String> {
        if !path.trim_matches('/').is_empty() {
            return Ok(Self::id_of(path));
        }
        if !create {
            return Err(VdfsError::invalid(format!(
                "写{LABEL}挂载根需要 create 意图：目录自身没有可覆盖的目标"
            )));
        }
        Ok(symbio_providers::vdfs_auto_id(PLUGIN_ID_MODEL))
    }

    /// 连接测试：草稿按补丁合并，只验证不落盘。
    async fn test_of(
        &self,
        id: &str,
        previous: Option<String>,
        draft: Option<Value>,
    ) -> Result<(bool, String, Option<Value>), PluginError> {
        let mut manifest = match (previous, draft) {
            (Some(raw), Some(draft)) => merge_manifest(id, Some(&raw), &draft)?,
            (Some(raw), None) => serde_json::from_str(&raw)
                .map_err(|e| PluginError::ValidationError(format!("Provider 配置无效: {e}")))?,
            (None, Some(draft)) => merge_manifest(id, None, &draft)?,
            (None, None) => {
                return Err(PluginError::ValidationError(
                    "测试草稿缺少字段配置".to_string(),
                ));
            }
        };
        if let Some(map) = manifest.as_object_mut() {
            // 连接探测统一留给下面一次，草稿不能借控制键跳过测试。
            map.insert("skip_validation".to_string(), Value::Bool(true));
        }
        let normalized = match self.validate_manifest(id, &manifest).await {
            Ok(normalized) => normalized,
            Err(error) => return Ok((false, error.to_string(), None)),
        };
        let provider = serde_json::from_value::<ModelProviderConfig>(normalized)
            .map_err(|e| PluginError::ParseError(format!("规范化结果回读失败: {e}")))?;
        let started = std::time::Instant::now();
        Ok(match Self::validate_provider(&provider).await {
            None => {
                let elapsed_ms = started.elapsed().as_millis();
                (
                    true,
                    format!(
                        "连接测试通过：{} · {} · {elapsed_ms} ms",
                        provider.model, provider.api_protocol
                    ),
                    Some(json!({
                        "model": provider.model,
                        "api_protocol": provider.api_protocol,
                        "elapsed_ms": elapsed_ms
                    })),
                )
            }
            Some(e) => (false, e, None),
        })
    }
}

#[async_trait]
impl VdfsProvider for ModelPlugin {
    async fn dispatch(
        &self,
        _ctx: &VdfsContext,
        path: &str,
        req: VdfsRequest,
    ) -> VdfsResult<VdfsResponse> {
        match req {
            VdfsRequest::List { .. } => {
                if !path.is_empty() {
                    return Err(VdfsError::not_found(format!(
                        "{LABEL}是叶子资源，没有子项：{path}"
                    )));
                }
                // 清单来自内存镜像（启动时自磁盘灌入，写 / 删后同步）
                Ok(VdfsResponse::list(self.mirrored_nodes().await))
            }
            VdfsRequest::Stat => {
                if path.is_empty() {
                    // 自身根：名字留空——provider 不知道自己的挂载名，由使用方回填。
                    //
                    // 根的自述里带着「可新建类型」：根下只能新建「模型」条目
                    // （model 不支持整包导入，故无导入入口）。`ext = model` 是**呈现
                    // 扩展名**（`id_of` 按它剥地址后缀），落成后的节点 `ext = form`
                    // ——两者不同，故显式声明 `node_ext` 与详情定义：使用方据此在
                    // 「还没创建」时就能渲染出与落成后同一张表单（草稿详情页）。
                    return Ok(VdfsResponse::Stat(
                        VdfsNode::dir("", LABEL, VdfsAccess::LIST).with_new_type(Some(
                            VdfsNewType::new(PLUGIN_ID_MODEL, LABEL)
                                .with_description(format!(
                                    "新建{LABEL}（在详情页里填好，保存时一次写入）"
                                ))
                                .with_node_ext(VDFS_EXT_FORM)
                                .with_schema(detail_definition()),
                        )),
                    ));
                }
                let id = Self::id_of(path);
                let entry = self.store().entry(&id).await?;
                let p =
                    config_of(&id, entry.raw.as_deref().unwrap_or_default()).ok_or_else(|| {
                        VdfsError::internal(format!("Provider「{id}」的清单不是合法配置"))
                    })?;
                let providers = self.providers.read().await;
                let is_default = providers
                    .resolve(None)
                    .is_some_and(|provider| provider.id == id);
                Ok(VdfsResponse::Stat(with_default(
                    node_of(&p, entry.updated_at),
                    is_default,
                )))
            }
            VdfsRequest::Read => {
                if path.is_empty() {
                    return Err(VdfsError::invalid(format!(
                        "该路径是目录，不可读取内容：{path}"
                    )));
                }
                // 通用读出口不暴露凭据；密码控件只是呈现，不能作为安全边界。
                let text = self.store().read_text(&Self::id_of(path)).await?;
                let mut value: Value = serde_json::from_str(&text).map_err(|e| {
                    VdfsError::internal(format!(
                        "Provider「{}」的清单不是合法 JSON：{e}",
                        Self::id_of(path)
                    ))
                })?;
                value
                    .as_object_mut()
                    .ok_or_else(|| {
                        VdfsError::internal(format!(
                            "Provider「{}」的清单必须是对象",
                            Self::id_of(path)
                        ))
                    })?
                    .insert("api_key".to_string(), Value::Null);
                let projected = serde_json::to_string_pretty(&value)
                    .map_err(|e| VdfsError::internal(format!("Provider 读取投影失败：{e}")))?;
                Ok(VdfsResponse::Read(
                    VdfsContent::text(projected).with_mime("application/json"),
                ))
            }
            VdfsRequest::Write { content } => {
                if content.binary {
                    return Err(VdfsError::invalid(format!("{LABEL}不支持整包导入（zip）")));
                }
                let id = Self::resolve_id(path, content.create)?;
                let text = content.as_text().unwrap_or_default();
                // `create` 只管「不存在时怎么办」，**不改变内容的处理方式**：草稿详情页
                // 填好的字段必须原样落盘（否则「填完再保存」等于白填）。唯一例外是
                // **内容为空**——「先建一个，随后再填」是合法形态，此时落一份最小配置。
                let manifest = if content.create && text.trim().is_empty() {
                    // 使用方只给了地址（或连名字都没有），最小配置由本插件自持
                    self.new_manifest(&id, &id)
                } else {
                    serde_json::from_str::<Value>(text)
                        .map_err(|e| VdfsError::invalid(format!("manifest 不是合法 JSON：{e}")))?
                };
                let (created, _) = self.save_manifest(&id, &manifest).await?;
                // 名字只在**匿名写**（打在挂载根上）时才需要交回：具名写的名字是
                // 调用方自己给的（见 [`VdfsWriteResponse::name`]）。
                let generated = path.trim_matches('/').is_empty().then_some(id);
                Ok(VdfsResponse::Write(VdfsWriteResponse {
                    name: generated,
                    created,
                }))
            }
            VdfsRequest::Delete { .. } => {
                if path.is_empty() {
                    return Err(VdfsError::Forbidden(format!("不可删除挂载点：{path}")));
                }
                let id = Self::id_of(path);
                // 存在性校验：删除不存在的条目应报 NotFound 而非静默成功
                self.config_on_disk(&id).await?;
                let plugin = self.clone();
                let removed_id = id.clone();
                self.store()
                    .remove_with(&id, move || async move {
                        plugin.after_deleted(&removed_id).await;
                    })
                    .await?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Action { action, payload } => match action.as_str() {
                "save" => {
                    let mut manifest = payload
                        .as_ref()
                        .and_then(|value| value.get("fields"))
                        .cloned()
                        .ok_or_else(|| VdfsError::invalid("保存动作缺少 fields 对象"))?;
                    if payload.as_ref().is_some_and(|value| {
                        value.get("skip_validation").and_then(Value::as_bool) == Some(true)
                    }) {
                        manifest
                            .as_object_mut()
                            .ok_or_else(|| VdfsError::invalid("保存动作的 fields 必须是对象"))?
                            .insert("skip_validation".to_string(), Value::Bool(true));
                    }
                    let id = Self::resolve_id(path, true)?;
                    self.save_manifest(&id, &manifest).await?;
                    Ok(VdfsResponse::Action(VdfsActionResult {
                        action,
                        ok: true,
                        message: "Provider 配置已保存".into(),
                        data: Some(json!({ "id": id })),
                    }))
                }
                VDFS_ACTION_TEST => {
                    let id = Self::resolve_id(path, true)?;
                    let previous = if path.trim_matches('/').is_empty() {
                        None
                    } else {
                        Some(self.store().read_text(&id).await?)
                    };
                    let (ok, detail, data) = self
                        .test_of(&id, previous, payload)
                        .await
                        .map_err(vdfs_from_plugin_error)?;
                    Ok(VdfsResponse::Action(VdfsActionResult {
                        action: action.clone(),
                        ok,
                        message: detail,
                        data,
                    }))
                }
                "set-default" => {
                    if path.is_empty() {
                        return Err(VdfsError::invalid("「设为默认」只对 Provider 条目可用"));
                    }
                    let id = Self::id_of(path);
                    let store = self.store();
                    let _guard = store.mutation_guard().await;
                    let entry = store.entry_locked(&id).await?;
                    let provider = config_of(&id, entry.raw.as_deref().unwrap_or_default())
                        .ok_or_else(|| {
                            VdfsError::internal(format!("Provider「{id}」的清单不是合法配置"))
                        })?;
                    if !provider.enabled {
                        return Err(VdfsError::invalid(format!(
                            "Provider「{id}」已停用，不能设为默认"
                        )));
                    }
                    let mut providers = self.providers.write().await;
                    let previous_default =
                        providers.resolve(None).map(|provider| provider.id.clone());
                    self.persist_default(&id).map_err(vdfs_from_plugin_error)?;
                    providers.default_provider_id = Some(id.clone());
                    drop(providers);
                    if let Some(previous) = previous_default.filter(|previous| *previous != id) {
                        symbio_plugin_api::vdfs_notify_change(PLUGIN_ID_MODEL, &previous);
                    }
                    symbio_plugin_api::vdfs_notify_change(PLUGIN_ID_MODEL, &id);
                    Ok(VdfsResponse::Action(VdfsActionResult {
                        action,
                        ok: true,
                        message: format!("已将「{}」设为默认 Provider", provider.name),
                        data: None,
                    }))
                }
                _ => Err(VdfsError::NotImplemented),
            },
            VdfsRequest::Watch { sink } => {
                vdfs_watch_changes(PLUGIN_ID_MODEL, path, sink).await?;
                Ok(VdfsResponse::Unit)
            }
            VdfsRequest::Unwatch => {
                vdfs_unwatch_changes(PLUGIN_ID_MODEL, path).await?;
                Ok(VdfsResponse::Unit)
            }
            _ => Err(VdfsError::NotImplemented),
        }
    }
}

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;

symbio_plugin_api::submit_object_creator!(PLUGIN_ID_MODEL, ModelPlugin::build, dyn Plugin);

#[async_trait]
impl Plugin for ModelPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    fn get_vfs_provider(self: Arc<Self>) -> Option<Arc<dyn symbio_plugin_api::VdfsProvider>> {
        Some(self)
    }

    /// `project_tools`本轮工具名的协议投影。
    ///
    /// 配置的读写在 VDFS 上（`<根>/model/<id>` 的详情表单，以及节点动作
    /// `set-default`），跨条目状态写自己的 `<本插件目录>/PLUGIN.yml`；本路由只回答
    /// 「本轮哪些工具能见、各自叫什么」，不碰配置。
    ///
    /// 输入 `{ "names": [<候选身份，按注册序>] }`，输出
    /// `{ exposed, aliases, collisions }`（见 [`super::tool_projection`]）。纯计算：
    /// 不读事实、不落事实——协议字符集只在本插件换算，会话只转发结果。
    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(symbio_plugin_api::PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);
        match path {
            // 本轮工具名的协议投影：哪些能见、各自叫什么、谁因碰撞退出
            "project_tools" => {
                let request: Value = ctx.payload()?;
                let names = request
                    .get("names")
                    .and_then(Value::as_array)
                    .ok_or_else(|| PluginError::ValidationError("缺少 names 清单".to_string()))?;
                let names = names
                    .iter()
                    .map(|name| {
                        name.as_str().map(str::to_string).ok_or_else(|| {
                            PluginError::ValidationError("names 必须是字符串清单".to_string())
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(PluginPayload::new(&super::tool_projection::project(&names)))
            }
            _ => Err(PluginError::NotFound(format!(
                "{PLUGIN_ID_MODEL} 无此路由：{path}"
            ))),
        }
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        // 模型能力纳入统一注册收集机制：与 local/web/mcp 插件注册工具同构。
        // 命中 TRAVERSE_AVAILABLE_TOOLS 时，按上下文解析出**唯一生效** Provider
        // （ctx[PROVIDER_ID] > default_provider_id > 首个 enabled），逐行写进
        // CAPABILITY_VISITOR 的**能力表**（生效位标在那一行上，见
        // `capability/model.rs`）；系统提示词注册在本插件名下（`PLUGIN_ID_MODEL`）。
        let sub_path = ctx.get(symbio_plugin_api::PATH).unwrap_or_default();
        match sub_path.as_str() {
            // 选项收集（与能力收集同一广播机制的第二通道）：贡献「Model」选择项
            symbio_plugin_api::TRAVERSE_AVAILABLE_OPTIONS => {
                self.contribute_options(&ctx).await;
                return Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()));
            }
            symbio_plugin_api::TRAVERSE_AVAILABLE_TOOLS => {}
            other => {
                return Err(PluginError::NotFound(format!("未知遍历路径: {other}")));
            }
        }

        if let Some(tool_visitor) = ctx.get(symbio_plugin_api::CAPABILITY_VISITOR) {
            let providers = self.providers.read().await;
            // 解析唯一生效 Provider：请求显式指定 > 默认 > 首个启用
            let requested = ctx
                .get(symbio_plugin_api::PROVIDER_ID)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty());
            let active = providers.resolve(requested.as_deref()).cloned();

            // 目录 = **全部已启用**的 provider（按 id 索引）。插件用它选自己的模型
            // （判决用便宜快的分类模型、措辞用更好的），会话引擎不看它。
            //
            // 协议钩子的构造要用 `ctx`（协议工厂按 `ctx` 装配），因此这一步只能在这里
            // 做——插件拿不到 model 插件的内部注册表，也不该拿到（E-007 / E-009）。
            //
            // 单个 provider 的协议工厂不可用 ⇒ 它**不在目录里**，而不是让整次注册失败：
            // 一个配坏的条目不该把其余可用的模型一起带走。
            let mut available: Vec<Arc<dyn crate::provider::ModelProvider>> = Vec::new();
            let mut active_in_catalog = false;
            for p in providers.providers.values().filter(|p| p.enabled) {
                let Some(protocol_id) = resolve_protocol_id(&p.api_protocol) else {
                    // 写错的协议**不替换成 openai_chat**：那会让一条 anthropic 配置
                    // 在本进程里发 OpenAI 请求体，故障推迟到运行期且指不到这里。
                    // 非生效者按软故障跳过；生效者是会话启动的硬前提——报进能力错误桶，
                    // 由编排方在收集结束后中止并展示原因（而不是只说「没有可用模型」）。
                    let message = format!(
                        "provider '{}'：{}；该条目不进目录（不会回退 openai_chat）",
                        p.id,
                        unsupported_protocol_message(&p.api_protocol)
                    );
                    plugin_warn!("model", "traverse: {message}");
                    if active.as_ref().is_some_and(|a| a.id == p.id) {
                        capability_report_error(&ctx, PLUGIN_ID_MODEL, message).await;
                    }
                    continue;
                };
                match creator_create_object::<dyn super::protocols::ModelProtocol>(
                    protocol_id,
                    ctx.clone(),
                ) {
                    Some(protocol) => {
                        active_in_catalog |= active.as_ref().is_some_and(|a| a.id == p.id);
                        available.push(Arc::new(BoundProvider::new(p.clone(), protocol)));
                    }
                    None => {
                        // 软故障（不进致命错误桶），消费侧报"未找到可用的 Model Provider"
                        plugin_warn!(
                            "model",
                            "traverse: provider '{}' 协议工厂不可用（protocol_id={protocol_id}），不进目录",
                            p.id
                        );
                    }
                }
            }

            let active_id = active.as_ref().map(|p| p.id.clone());
            if active_id.is_some() && !active_in_catalog {
                // 生效者配坏了：**不**替会话挑一个别的（静默换模型比没有模型更难查）
                plugin_warn!(
                    "model",
                    "traverse: 生效 provider 未进目录，会话将报「未找到可用的 Model Provider」"
                );
            }
            if available.is_empty() {
                plugin_warn!(
                    "model",
                    "traverse: 无可用 Model Provider（requested={requested:?}），跳过注册"
                );
            }
            // 入表：**一行一个已启用 provider**，当次生效者带**生效位**（DE-025 §3）。
            // 「生效者必然是目录里的一员」由此是结构而不是纪律——生效位长在目录行上：
            // 生效 provider 不在目录里（未启用 / 协议工厂不可用）⇒ 没有任何行带位
            // ⇒ 会话报「未找到可用的 Model Provider」，而不是替它挑一个别的。
            // 行里只剩**目录事实**（id + 生效位）：执行对象留在下面的
            // `ModelPort` 手里。行不再是「一个能跑的 provider」，因此表的读者
            // 不会顺手拿到一次调用能力。
            for provider in &available {
                let is_active = active_id.as_deref() == Some(provider.provider_id());
                tool_visitor
                    .put(CapabilityRow::model(provider.provider_id(), is_active))
                    .await;
            }

            // 世界端口：生效者在目录里 ⇒ 注册 `port.model` 提供者行（解析与
            // 适配器构造收进 `ModelPort`，环只按端口名取）。生效者缺席 ⇒ 不注册——
            // 环在调用点拿到缺席读数，落失败事实并中止，不在这里猜一个别的。
            if let Some(active) = available
                .iter()
                .find(|p| active_id.as_deref() == Some(p.provider_id()))
                .cloned()
            {
                let port = Arc::new(super::port::ModelPort::new(
                    active.clone(),
                    available.clone(),
                ));
                symbio_plugin_api::port::register_ports(&ctx, port.as_ref()).await;
                // 一次性问答：同一生效者，独立端口成员 `model.assist`；
                // 提问方的指令随行载荷进，本插件不认识它的世界语义。偏好 provider
                // 从行载荷取，全集随端口带过去供按 id 解析。
                let assist = Arc::new(super::assist::AssistPort::new(active, available));
                symbio_plugin_api::port::register_ports(&ctx, assist.as_ref()).await;
            }

            // 人格在**注册期**就已选定（上面按 `PROVIDER_ID` 解析出生效 provider），
            // 因此这里只注册一个条目。历史上曾同时注册 `provider_id` 与 `"default"`
            // 两个键——值完全相同，而消费侧早已不再按 key 挑选，属于纯粹冗余
            // （见 `capability/prompt.rs` 的模块文档）。
            if let Some(sp) = active.as_ref().and_then(|p| p.system_prompt.as_ref()) {
                tool_visitor
                    // 段名是覆盖键，**不得与挂载名同键**（4d 后同一张表：跨族同名会顶掉
                    // 本插件的挂载行 `model`）。
                    .put(CapabilityRow::prompt("model-persona", sp.clone()))
                    .await;
            }

            // 债表（`debt` 族，既有能力收集机制）：模型欠哪一轮由本插件自报，
            // 宿主收集、环只读。生效者缺席不影响申报——缺席在调用点响亮失败。
            for row in super::debt::model_debt_rows() {
                tool_visitor.put(row).await;
            }
        }

        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

// ── panic 面登记（PN-001…003）─────────────────────────────────────────
// 本文件每一处 `unwrap` / `expect` / `panic!` / `unreachable!` 的理由。登记放在
// 文件内而不是集中一张表：理由与它解释的那段代码会一起被 review、一起被删。
// 判据见 `scripts/audits/panic-audit.mjs`。**加一处 panic 必须同时加一行登记，理由非空。**
