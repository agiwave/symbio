//! Model Provider 多实例配置 Schema
//!
//! 设计与定位：
//! - Model Provider 是"供应商 + 模型 + 协议 + 调用参数 + 限流设置"的完整可复用单元
//! - 用户可以同时维护多个 Model Provider（例如 OpenAI、Anthropic、本地 Ollama 等），
//!   每次对话可以选择其中一个使用
//! - 配置以 `id` 为键保存在 `ModelProvidersConfig` 中，并提供 `default_provider_id` 标识默认
//! - 兼容老版本：单 Model 配置 (`ModelConfig`) 可视为"默认 Provider 的派生视图"

use super::model_config::{ModelConfig, ReasoningConfig};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn default_enabled() -> bool {
    true
}

fn default_rate_limit_ms() -> u64 {
    0
}

fn default_provider_name() -> String {
    "Default".to_string()
}

/// 单个 Model Provider 配置
///
/// 在 `ModelConfig` 之上扩展：
/// - `id` / `name`: 注册表内唯一标识与展示名
/// - `rate_limit_ms`: 最小请求间隔（毫秒），0 表示不限制
/// - `enabled`: 是否启用；禁用后无法被选为活动 Provider
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelProviderConfig {
    /// Provider 唯一 ID（注册表 key）
    pub id: String,
    /// 展示名称
    #[serde(default = "default_provider_name")]
    pub name: String,

    /// 供应商标识 (openMODEL, anthropic, lmstudio, ollama 等)
    pub provider: String,
    pub api_base: String,
    pub api_key: Option<String>,
    pub model: String,
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    pub max_tokens: Option<u32>,
    pub system_prompt: Option<String>,
    #[serde(default = "default_max_context_tokens")]
    pub max_context_tokens: u32,
    #[serde(default = "default_reserved_tokens")]
    pub reserved_tokens: u32,
    #[serde(default = "default_timeout_secs")]
    pub timeout_secs: u64,
    #[serde(default = "default_api_protocol")]
    pub api_protocol: String,
    #[serde(default = "default_store")]
    pub store: bool,
    pub reasoning: Option<ReasoningConfig>,

    /// 最小请求间隔（毫秒）；0 表示不限制
    #[serde(default = "default_rate_limit_ms")]
    pub rate_limit_ms: u64,

    /// 是否启用；禁用后无法被选为活动 Provider
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_temperature() -> f32 {
    0.7
}
fn default_max_context_tokens() -> u32 {
    128_000
}
fn default_reserved_tokens() -> u32 {
    4_096
}
fn default_timeout_secs() -> u64 {
    300
}
fn default_api_protocol() -> String {
    "openai_responses".to_string()
}
fn default_store() -> bool {
    false
}

impl Default for ModelProviderConfig {
    fn default() -> Self {
        Self {
            id: "default".to_string(),
            name: default_provider_name(),
            provider: String::new(),
            api_base: String::new(),
            api_key: None,
            model: String::new(),
            temperature: default_temperature(),
            max_tokens: None,
            system_prompt: None,
            max_context_tokens: default_max_context_tokens(),
            reserved_tokens: default_reserved_tokens(),
            timeout_secs: default_timeout_secs(),
            api_protocol: default_api_protocol(),
            store: default_store(),
            reasoning: None,
            rate_limit_ms: default_rate_limit_ms(),
            enabled: default_enabled(),
        }
    }
}

impl ModelProviderConfig {
    /// 转换为运行期使用的 `ModelConfig`（剥离 id/name/限流/启用位等管理字段）
    pub fn to_model_config(&self) -> ModelConfig {
        ModelConfig {
            provider: self.provider.clone(),
            api_base: self.api_base.clone(),
            api_key: self.api_key.clone(),
            model: self.model.clone(),
            temperature: self.temperature,
            max_tokens: self.max_tokens,
            system_prompt: self.system_prompt.clone(),
            max_context_tokens: self.max_context_tokens,
            reserved_tokens: self.reserved_tokens,
            timeout_secs: self.timeout_secs,
            api_protocol: self.api_protocol.clone(),
            store: self.store,
            reasoning: self.reasoning.clone(),
        }
    }

    /// 由 `ModelConfig` 构造 `ModelProviderConfig`（用于兼容旧配置升级）
    pub fn from_model_config(id: &str, name: &str, cfg: &ModelConfig) -> Self {
        Self {
            id: id.to_string(),
            name: name.to_string(),
            provider: cfg.provider.clone(),
            api_base: cfg.api_base.clone(),
            api_key: cfg.api_key.clone(),
            model: cfg.model.clone(),
            temperature: cfg.temperature,
            max_tokens: cfg.max_tokens,
            system_prompt: cfg.system_prompt.clone(),
            max_context_tokens: cfg.max_context_tokens,
            reserved_tokens: cfg.reserved_tokens,
            timeout_secs: cfg.timeout_secs,
            api_protocol: cfg.api_protocol.clone(),
            store: cfg.store,
            reasoning: cfg.reasoning.clone(),
            rate_limit_ms: 0,
            enabled: true,
        }
    }
}

/// Model Providers 注册表（多 Provider 容器）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModelProvidersConfig {
    /// 所有 Provider，key 为 `ModelProviderConfig.id`
    #[serde(default)]
    pub providers: HashMap<String, ModelProviderConfig>,
    /// 当前默认 Provider ID
    #[serde(default)]
    pub default_provider_id: Option<String>,
}

impl ModelProvidersConfig {
    /// 解析一个目标 Provider：按 provider_id 查找，未找到或禁用则降级到 default
    pub fn resolve(&self, provider_id: Option<&str>) -> Option<&ModelProviderConfig> {
        if let Some(id) = provider_id {
            if let Some(p) = self.providers.get(id) {
                if p.enabled {
                    return Some(p);
                }
            }
        }
        if let Some(default_id) = &self.default_provider_id {
            if let Some(p) = self.providers.get(default_id) {
                if p.enabled {
                    return Some(p);
                }
            }
        }
        // 兜底：第一个 enabled 的 provider
        self.providers.values().find(|p| p.enabled)
    }
}

// ==================== CRUD 请求/响应 ====================

/// List - 列出全部 Model Provider
pub mod model_providers_list {
    use super::ModelProvidersConfig;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    pub struct Request {}

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    pub struct Response {
        pub config: ModelProvidersConfig,
    }
}

/// Get - 获取单个 Provider 配置
pub mod model_providers_get {
    use super::ModelProviderConfig;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Request {
        pub provider_id: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    pub struct Response {
        pub provider: ModelProviderConfig,
    }
}

/// Set - 创建或更新一个 Provider（按 id upsert）
pub mod model_providers_set {
    use super::ModelProviderConfig;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Request {
        pub provider: ModelProviderConfig,
        /// 是否跳过 API 校验；true 时不发起实际校验请求
        #[serde(default)]
        pub skip_validation: bool,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    pub struct Response {
        pub provider: ModelProviderConfig,
    }
}

/// Delete - 删除一个 Provider
pub mod model_providers_delete {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Request {
        pub provider_id: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    pub struct Response {}
}

/// SetDefault - 设置默认 Provider
pub mod model_providers_set_default {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Request {
        pub provider_id: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    pub struct Response {}
}

/// Test - 连接测试（无副作用：不写入注册表，不落盘）
///
/// 复用 `providers/set` 的验证逻辑（`validate_provider` → 按协议发起真实 API
/// 请求并等待流式响应），区别在于**测试路由不产生任何状态变更**——
/// 用于"保存前测试连接"场景（未保存的草稿配置也可以直接测试）。
pub mod model_providers_test {
    use super::ModelProviderConfig;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Request {
        pub provider: ModelProviderConfig,
        /// 预留：与 set 的 skip_validation 对齐；test 路由当前忽略此字段
        /// （测试连接的语义就是发起真实校验）
        #[serde(default)]
        pub skip_validation: bool,
    }

    /// 测试通过时返回；失败以 `PluginError::ValidationError` 报错
    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    pub struct Response {}
}
