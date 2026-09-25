//! Model Provider 多实例配置 Schema
//!
//! 设计与定位：
//! - Model Provider 是"供应商 + 模型 + 协议 + 调用参数 + 限流设置"的完整可复用单元
//! - 用户可以同时维护多个 Model Provider（例如 OpenAI、Anthropic、本地 Ollama 等），
//!   每次对话可以选择其中一个使用
//! - 每个 Provider 以 `id` 为键落一份 `<本插件目录>/<id>/provider.json`；
//!   `ModelProvidersConfig` 是它们的**运行期内存视图**（同一批配置 + 一个默认指向）
//! - 运行期契约为 core 的纯 `ModelProvider` trait，由 `bound_provider::BoundProvider`
//!   绑定本配置与协议实现后实现；管理字段（id/name/enabled 等）仅在本层存在

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

/// 推理配置（serde 形态冻结，保持用户配置兼容）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningConfig {
    pub effort: String,
}

/// 单个 Model Provider 配置
///
/// 管理字段（`id` / `name` / `rate_limit_ms` / `enabled`）之外的全部字段
/// 均为模型参数，由协议钩子（`ModelProtocol`，收 `&ModelProviderConfig`）
/// 与 `BoundProvider`（core `ModelProvider` trait 实现）直接读取。
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
    pub temperature: f64,
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

fn default_temperature() -> f64 {
    0.7
}
fn default_max_context_tokens() -> u32 {
    // 默认 256k：多数主流模型上下文窗口的保守公共值
    262_144
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

/// Model Providers 注册表（多 Provider 容器，运行期内存视图）
#[derive(Debug, Clone, Default)]
pub struct ModelProvidersConfig {
    /// 所有 Provider，key 为 `ModelProviderConfig.id`
    pub providers: HashMap<String, ModelProviderConfig>,
    /// 当前默认 Provider ID
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
