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
use crate::providers::vdfs_service::{MemoryVdfs, SingleFileVdfs};
use crate::symbio_core::schemas::detail::{DetailField, DetailOption};
use crate::symbio_core::{
    create_object, dir_from_ctx, InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin,
    PluginDir, PluginError, PluginMeta, PluginPayload, SimpleRequest, PLUGIN_MODEL,
};
use crate::{plugin_error, plugin_info, plugin_warn};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::RwLock;

/// Provider 主文件名（磁盘布局：`<本插件目录>/<id>/provider.json`）
const MANIFEST: &str = "provider.json";

/// 跨条目的插件配置（`<本插件目录>/PLUGIN.yml`）—— **写入形态**
///
/// 只写**跨条目的状态**：单个 Provider 的明细是**资源**，落在
/// `<本插件目录>/<id>/provider.json`，不进配置文件。
///
/// 读取比写入**宽**：`build` 读的是 `ModelProvidersConfig`。旧形态曾把 Provider
/// 明细整包存在这里（`providers` 键），读宽一次让既有的一次性迁移能照常把它们
/// 搬成资源；搬完 `persist()` 把文件归一为只有跨条目状态——同一个事实不留两份。
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
    /// （[`sync_mirror`](Self::sync_mirror)），因此不存在第二个写入者。
    entries: MemoryVdfs,
}

impl ModelPlugin {
    /// 静态工厂：从 InvokeRequest 构造 Plugin 实例
    ///
    /// 加载策略（按优先级）：
    /// 1. **存储**：从 `<本插件目录>/<id>/provider.json` 加载所有 Provider
    /// 2. **跨条目配置**：`<本插件目录>/PLUGIN.yml` 里的 `default_provider_id`
    ///    （旧形态里可能还带着 `providers` 明细，由 `load_from_storage` 搬成资源）
    pub fn build(ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Plugin> {
        let dir = dir_from_ctx(&*ctx, PLUGIN_MODEL);
        // 读宽：兼容旧形态里整包存在配置中的 Provider 明细
        let providers_config: ModelProvidersConfig = match dir.load::<ModelProvidersConfig>() {
            Ok(Some(c)) => c,
            Ok(None) => ModelProvidersConfig::default(),
            Err(e) => {
                plugin_warn!("model", "读取自身配置失败，改用默认值：{e}");
                ModelProvidersConfig::default()
            }
        };

        let plugin = Arc::new(Self::new(providers_config, dir));

        // 启动后异步触发：从存储加载（并触发首启动数据迁移）
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
        SingleFileVdfs::at(self.dir.dir(), PLUGIN_MODEL, MANIFEST).with_label(LABEL)
    }

    /// 异步加载：从存储拉取所有 Provider
    ///
    /// 启动时调用此方法，**会**触发首启动数据迁移（从配置文件里的旧明细迁到新存储）。
    /// `ctx` 保留给调用方兼容；迁移不依赖请求上下文。
    pub async fn load_from_storage(&self, _ctx: &Arc<dyn InvokeRequest>) {
        // 仅兼容迁移认识旧分类的历史落位。
        let legacy = SingleFileVdfs::for_category("ai", MANIFEST);
        self.load_with_legacy(&legacy).await;
    }

    async fn load_with_legacy(&self, legacy: &SingleFileVdfs) {
        let store = self.store();
        // 无法枚举目标时不做迁移或清理。
        if let Err(e) = store.entries().await {
            plugin_warn!("model", "list models 失败: {e}");
            return;
        }
        self.migrate_legacy_category(&store, legacy).await;
        self.migrate_from_legacy_config(&store).await;
        // 迁移后重新读取，不递归重入，也不以“目标为空”作为续迁条件。
        let entries = match store.entries().await {
            Ok(v) => v,
            Err(e) => {
                plugin_warn!("model", "list models 失败: {e}");
                return;
            }
        };

        // 3. 加载存储内容（注册表 + VDFS 镜像一次性同构填充）
        let mut new_providers = std::collections::HashMap::new();
        let mut mirror = Vec::with_capacity(entries.len());
        let mut marked_default: Option<String> = None;
        for e in &entries {
            let Some(text) = e.raw.as_deref() else {
                plugin_warn!("model", "读取 provider {} 失败：主文件不可用", e.id);
                continue;
            };
            // 先解析为 Value 提取 is_default 标记，再解析为强类型配置
            let parsed = serde_json::from_str::<Value>(text).and_then(|v| {
                serde_json::from_value::<ModelProviderConfig>(v.clone()).map(|p| (v, p))
            });
            match parsed {
                Ok((raw, mut p)) => {
                    if p.id.is_empty() {
                        p.id = e.id.clone();
                    }
                    if p.name.is_empty() {
                        p.name = e.id.clone();
                    }
                    if raw
                        .get("is_default")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        marked_default = Some(e.id.clone());
                    }
                    new_providers.insert(e.id.clone(), p);
                    mirror.push((e.id.clone(), text.to_string()));
                }
                Err(err) => plugin_warn!("model", "解析 provider {} 失败: {err}", e.id),
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
        drop(cfg);
        self.entries.replace_all(mirror);
        plugin_info!(
            "model",
            "从 <本插件目录>/ 加载了 {} 个 Model Provider",
            self.entries.ids().len()
        );
    }

    /// 已有可解析目标是权威版本；坏文件/读失败不是“缺失”，不能覆盖。
    /// 新写入必须回读一致，才算本次迁移成功。
    async fn migrate_provider(store: &SingleFileVdfs, id: &str, text: &str) -> bool {
        if crate::providers::vdfs_service::entry::safe_segment(id) != id
            || serde_json::from_str::<ModelProviderConfig>(text).is_err()
        {
            plugin_warn!(
                "model",
                "迁移 provider {id} 失败：源格式或 id 不可用，保留源"
            );
            return false;
        }
        match store.read_text(id).await {
            Ok(existing) => {
                let valid = serde_json::from_str::<ModelProviderConfig>(&existing).is_ok();
                if !valid {
                    plugin_warn!(
                        "model",
                        "迁移 provider {id} 失败：目标不可解析，不覆盖并保留源"
                    );
                }
                return valid;
            }
            Err(VdfsError::NotFound(_)) => {}
            Err(e) => {
                plugin_warn!("model", "迁移 provider {id} 读取目标失败：{e}，保留源");
                return false;
            }
        }
        if let Err(e) = store.write_text(id, text).await {
            plugin_warn!("model", "迁移 provider {id} 写入失败：{e}，保留源");
            return false;
        }
        match store.read_text(id).await {
            Ok(written) if written == text => true,
            _ => {
                plugin_warn!("model", "迁移 provider {id} 回读确认失败，保留源");
                false
            }
        }
    }

    /// 整批确认后才清理旧分类；失败保留全部源，下一次只补缺失项。
    async fn migrate_legacy_category(&self, store: &SingleFileVdfs, legacy: &SingleFileVdfs) {
        let entries = match legacy.entries().await {
            Ok(entries) => entries,
            Err(e) => {
                plugin_warn!("model", "读取旧分类失败：{e}，保留源");
                return;
            }
        };
        let mut complete = true;
        for e in &entries {
            match e.raw.as_deref() {
                Some(text) => complete &= Self::migrate_provider(store, &e.id, text).await,
                None => {
                    plugin_warn!("model", "读取旧 provider {} 失败，保留整批源", e.id);
                    complete = false;
                }
            }
        }
        if complete {
            for e in &entries {
                if let Err(err) = legacy.remove(&e.id).await {
                    plugin_warn!("model", "清理旧 provider {} 失败：{err}，下次重试", e.id);
                }
            }
        }
    }

    /// 每次从磁盘旧配置续迁，不把运行期注册表误当成待迁移源。
    async fn migrate_from_legacy_config(&self, store: &SingleFileVdfs) {
        let manifest = match self.dir.read_manifest() {
            Ok(Some(m)) if m.contains_key("providers") => m,
            Ok(_) => return,
            Err(e) => {
                plugin_warn!("model", "读取旧配置失败：{e}，保留源");
                return;
            }
        };
        let current: ModelProvidersConfig = match serde_json::from_value(Value::Object(manifest)) {
            Ok(c) => c,
            Err(e) => {
                plugin_warn!("model", "解析旧配置失败：{e}，保留源");
                return;
            }
        };
        let mut complete = true;
        for (id, p) in &current.providers {
            match serde_json::to_string_pretty(p) {
                Ok(text) => complete &= Self::migrate_provider(store, id, &text).await,
                Err(e) => {
                    plugin_error!("model", format!("序列化 provider {id} 失败：{e}，保留源"));
                    complete = false;
                }
            }
        }
        if complete {
            // 所有明细均已确认落盘才允许清掉内嵌源。save 的原子写失败仍保留旧文件。
            let cfg = ModelConfig {
                default_provider_id: current.default_provider_id,
            };
            if let Err(e) = self.dir.save(&cfg) {
                plugin_warn!("model", "迁移配置归一失败：{e}，保留源待重试");
            }
        }
    }

    /// 主构造函数（Factory 机制使用）
    pub fn new(providers: ModelProvidersConfig, dir: PluginDir) -> Self {
        let entries = MemoryVdfs::new(PLUGIN_MODEL).with_label(LABEL);
        Self {
            providers: Arc::new(RwLock::new(providers)),
            dir,
            entries,
        }
    }

    /// 把自己的跨条目配置写回**自己的** `PLUGIN.yml`（不再经父插件）
    ///
    /// 只写 `default_provider_id`：单个 Provider 的明细是资源，落在
    /// `<本插件目录>/<id>/provider.json`，不进配置文件。
    async fn persist(&self) -> InvokeResponse<()> {
        let cfg = ModelConfig {
            default_provider_id: self.providers.read().await.default_provider_id.clone(),
        };
        // 迁移未完成时，普通编辑 / set-default 也不能间接抹掉内嵌源。
        let mut manifest = self
            .dir
            .read_manifest()
            .map_err(PluginError::InternalError)?
            .unwrap_or_default();
        let result = if manifest.contains_key("providers") {
            manifest.insert("default_provider_id".into(), json!(cfg.default_provider_id));
            self.dir.save(&manifest)
        } else {
            self.dir.save(&cfg)
        };
        result.map_err(PluginError::InternalError)
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

    /// 参与 `available_options` 收集：贡献「Model」字段。
    ///
    /// 候选 = 每个启用的 Provider（值 = `provider_id`）；`default` 取**生效
    /// Provider**（请求显式 > 默认）——这一步的解析链只有后端知道
    /// （`providers.default_provider_id`），故由后端把结论写进定义：会话 metadata
    /// 里没有 `provider_id` 时，前端按 `default` 显示生效的那个（见
    /// `docs/design/session-options-unification.md` §6）。
    ///
    /// **不回填当前值**：值来自会话 `metadata.provider_id`，这里只声明候选与缺省。
    async fn contribute_options(&self, ctx: &Arc<dyn InvokeRequest>) {
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
            .register_option_field(ORDER, provider_field(&enabled, effective.as_deref()))
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

impl Default for ModelPlugin {
    fn default() -> Self {
        Self::new(ModelProvidersConfig::default(), PluginDir::of(PLUGIN_MODEL))
    }
}

// ==================== VDFS 挂载点（`<根>/model`） ====================
//
// 本插件**直接实现 `VdfsProvider`**：VDFS 是唯一协议、唯一地址空间，列 / 读 /
// 写 / 删 / 动作的语义都在这里表达。
//
// 存储不自建抽象：落盘走 `providers::vdfs_service::SingleFileVdfs`（一个条目 =
// 一份 `provider.json`），清单走 `MemoryVdfs`（内存镜像，规范 §13.4「model 的列表
// 来自内存」）。于是本模块只剩 **model 特有的三件事**：详情定义随节点下发、
// 写前的连接校验、写后对 chat 侧注册表与内存镜像的同步。
//
// 分工与旧写法一致：`list` 读内存镜像，`stat` / `read` / `delete` / `action`
// 读磁盘（真相源），避免镜像与磁盘在校验路径上出现分歧。

use crate::symbio_core::vdfs::{from_plugin_error, unwatch_changes, watch_changes};
use crate::symbio_core::vdfs_provider::{
    VdfsAccess, VdfsActionResult, VdfsChangeSink, VdfsContent, VdfsContext, VdfsError, VdfsNewType,
    VdfsNode, VdfsProvider, VdfsResult, VdfsWriteResponse, VDFS_ACTION_TEST, VDFS_EXT_FORM,
    VDFS_STATUS_ACTIVE, VDFS_STATUS_DISABLED,
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
    n.kind = PLUGIN_MODEL.to_string();
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

/// manifest 缺 `id`（或空串）时以路径段补全，已有值原样保留
///
/// 这是**读写两条路径共同的不变量**：`DetailForm` 的 option 绑定只回纯字段值
/// （id 不在表单字段里），而 `ModelProviderConfig.id` 无 serde 缺省 ⇒ 不补就会
/// 报「missing field `id`」（编辑已有 Provider 保存失败即为该症状）。
fn with_id(manifest: &Value, id: &str) -> Value {
    let mut m = manifest.clone();
    let missing = m
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .is_empty();
    if missing {
        if let Value::Object(map) = &mut m {
            map.insert("id".to_string(), json!(id));
        }
    }
    m
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
        for id in self.entries.ids() {
            let text = self.entries.get(&id).unwrap_or_default();
            if let Some(p) = config_of(&id, &text) {
                out.push(node_of(&p, self.entries.updated_at(&id)));
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

    /// 落盘之后同步两份内存视图（chat 侧注册表 + VDFS 镜像）+ 写回自己的配置文件
    ///
    /// **唯一写入者**：注册表与镜像只在这里成对更新，因此二者不会各说各话。
    async fn after_uploaded(&self, id: &str, normalized: &Value) -> Result<(), PluginError> {
        // 「设为默认」标记（validate_manifest 保留、随 manifest 落盘）
        let is_default = normalized
            .get("is_default")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let provider: ModelProviderConfig = serde_json::from_value(normalized.clone())
            .map_err(|e| PluginError::ParseError(format!("规范化结果回读失败: {e}")))?;
        {
            let mut providers = self.providers.write().await;
            providers.providers.insert(id.to_string(), provider);
            if is_default || providers.default_provider_id.is_none() {
                providers.default_provider_id = Some(id.to_string());
            }
        }
        let text = serde_json::to_string_pretty(normalized)
            .map_err(|e| PluginError::ParseError(e.to_string()))?;
        self.entries.set(id, text);
        self.persist().await
    }

    /// 删除后清理两份内存视图
    async fn after_deleted(&self, id: &str) {
        let mut providers = self.providers.write().await;
        providers.providers.remove(id);
        if providers.default_provider_id.as_deref() == Some(id) {
            providers.default_provider_id = None;
        }
        drop(providers);
        self.entries.remove(id);
    }

    /// 路径末段 → 条目 id（去掉 `.<kind>` 呈现扩展名）
    fn id_of(path: &str) -> String {
        crate::providers::vdfs_service::entry::id_of(path, PLUGIN_MODEL)
    }

    /// 目标地址 → 条目 id（**唯一**判据，`write` 与测试共用）。
    ///
    /// 两种目标形态见 [`VdfsProvider::write`](crate::symbio_core::vdfs_provider::VdfsProvider::write)：
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
        Ok(crate::providers::vdfs_service::entry::auto_id(PLUGIN_MODEL))
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
        Some(PLUGIN_MODEL)
    }

    /// 根下只能新建「模型」条目（model 不支持整包导入）
    ///
    /// `ext = model` 是**呈现扩展名**（`id_of` 按它剥地址后缀），落成后的节点
    /// `ext = form`——两者不同，故显式声明 `node_ext` 与详情定义：使用方据此
    /// 在「还没创建」时就能渲染出与落成后同一张表单（草稿详情页）。
    async fn root_new_types(&self) -> Vec<VdfsNewType> {
        vec![VdfsNewType::new(PLUGIN_MODEL, LABEL)
            .with_description(format!("新建{LABEL}（在详情页里填好，保存时一次写入）"))
            .with_node_ext(VDFS_EXT_FORM)
            .with_schema(detail_definition())]
    }

    async fn list(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<Vec<VdfsNode>> {
        if !path.is_empty() {
            return Err(VdfsError::not_found(format!(
                "{LABEL}是叶子资源，没有子项：{path}"
            )));
        }
        // 清单来自内存镜像（启动时自磁盘灌入，写 / 删后同步）
        Ok(self.mirrored_nodes().await)
    }

    async fn stat(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsNode> {
        if path.is_empty() {
            // 自身根：名字留空——provider 不知道自己的挂载名，由使用方回填
            return Ok(VdfsNode::dir("", LABEL, VdfsAccess::LIST));
        }
        let id = Self::id_of(path);
        let entry = self.store().entry(&id).await?;
        let p = config_of(&id, entry.raw.as_deref().unwrap_or_default())
            .ok_or_else(|| VdfsError::internal(format!("Provider「{id}」的清单不是合法配置")))?;
        Ok(node_of(&p, entry.updated_at))
    }

    async fn read(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<VdfsContent> {
        if path.is_empty() {
            return Err(VdfsError::invalid(format!(
                "该路径是目录，不可读取内容：{path}"
            )));
        }
        // 读磁盘原文：DetailForm 以它作预填输入，`is_default` 这类落盘标记因此
        // 与磁盘严格一致（不在读取时重新推导）。
        let text = self.store().read_text(&Self::id_of(path)).await?;
        Ok(VdfsContent::text(path, text).with_mime("application/json"))
    }

    async fn write(
        &self,
        _ctx: &VdfsContext,
        path: &str,
        content: &VdfsContent,
    ) -> VdfsResult<VdfsWriteResponse> {
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
        let normalized = self
            .validate_manifest(&id, &manifest)
            .await
            .map_err(from_plugin_error)?;
        // 落盘（原子写 + 变更广播由 vdfs_service 承担）→ 再同步内存视图
        let created = self.store().write_json(&id, &normalized).await?;
        self.after_uploaded(&id, &normalized)
            .await
            .map_err(from_plugin_error)?;
        Ok(VdfsWriteResponse {
            path: id,
            created,
            etag: None,
        })
    }

    async fn delete(&self, _ctx: &VdfsContext, path: &str, _recursive: bool) -> VdfsResult<()> {
        if path.is_empty() {
            return Err(VdfsError::Forbidden(format!("不可删除挂载点：{path}")));
        }
        let id = Self::id_of(path);
        // 存在性校验：删除不存在的条目应报 NotFound 而非静默成功
        self.config_on_disk(&id).await?;
        self.store().remove(&id).await?;
        self.after_deleted(&id).await;
        Ok(())
    }

    async fn action(
        &self,
        _ctx: &VdfsContext,
        path: &str,
        action: &str,
        _payload: Option<&Value>,
    ) -> VdfsResult<VdfsActionResult> {
        match action {
            VDFS_ACTION_TEST => {
                if path.is_empty() {
                    return Err(VdfsError::invalid(format!(
                        "「测试连接」只对{LABEL}条目可用：{path}"
                    )));
                }
                let id = Self::id_of(path);
                // 存在性校验：测试不存在的条目应报 NotFound 而非成功
                self.config_on_disk(&id).await?;
                let (ok, detail) = self.test_of(&id).await.map_err(from_plugin_error)?;
                Ok(VdfsActionResult {
                    action: VDFS_ACTION_TEST.to_string(),
                    ok,
                    message: detail,
                    data: None,
                })
            }
            _ => Err(VdfsError::NotImplemented),
        }
    }

    async fn watch(&self, _ctx: &VdfsContext, path: &str, sink: VdfsChangeSink) -> VdfsResult<()> {
        watch_changes(PLUGIN_MODEL, path, sink).await
    }

    async fn unwatch(&self, _ctx: &VdfsContext, path: &str) -> VdfsResult<()> {
        unwatch_changes(PLUGIN_MODEL, path).await
    }
}

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;

crate::submit_object_creator!(PLUGIN_MODEL, ModelPlugin::build, dyn Plugin);

#[async_trait]
impl Plugin for ModelPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    fn get_vfs_provider(
        self: Arc<Self>,
    ) -> Option<Arc<dyn crate::symbio_core::vdfs_provider::VdfsProvider>> {
        Some(self)
    }

    /// model 已无自有路由：配置的读写在 VDFS 上（`<根>/model/<id>` 的详情表单，
    /// 以及节点动作 `set-default`），跨条目状态写自己的 `<本插件目录>/PLUGIN.yml`。
    async fn route(self: Arc<Self>, _ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        Err(PluginError::NotFound(format!(
            "{PLUGIN_MODEL} 已无自有路由，请改用 VDFS 地址"
        )))
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
            // VDFS 挂载点：本插件自身就是 provider（`<根>/model`）——
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
                            // 人格在**注册期**就已选定（上面 `providers.resolve` 按
                            // `PROVIDER_ID` 解析出唯一生效 provider），因此这里只注册
                            // 一个条目。历史上曾同时注册 `provider_id` 与 `"default"`
                            // 两个键——值完全相同，而消费侧早已不再按 key 挑选，
                            // 属于纯粹冗余（见 `CapabilityVisitor::register_system_prompt`）。
                            let system_prompt = p.system_prompt.clone();
                            let provider = Arc::new(BoundProvider::new(p, protocol));
                            tool_visitor.register_model_provider(provider).await;
                            if let Some(sp) = &system_prompt {
                                tool_visitor
                                    .register_system_prompt(PLUGIN_MODEL, sp.clone())
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
