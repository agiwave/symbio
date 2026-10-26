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

use crate::symbio_core::{ModelSpend, ModelUsage};

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

/// Token 单价（每个 Provider 自己的价格，**不按模型名猜**）。
///
/// ## 为什么价格是配置，且必须整组出现
///
/// 同一个模型名在不同端点 / 套餐下价格可以完全不同（官方 API、代理、自建、
/// 免费额度）；按名字内置一张价目表，会在用户换端点的那一天静默算错。价格因此
/// 跟着 Provider 条目走，由用户填。三项同属一个对象：输入价、输出价、币种缺一
/// 不可——缺币种的金额没有意义，缺一侧的单价会把该侧用量记成免费。
///
/// ## 缺省 = 不折算，零 = 明确免费
///
/// `ModelProviderConfig.pricing` 缺席 ⇒ 这份配置不产生金额读数（token 照记）；
/// 三项都填 0 ⇒ 每次往返记一条 `amount_nano: 0` 的账。两者在账本上由
/// 「有没有这条记录」分辨，不靠补零冒充（与 `usage` 的「没测量 vs 测到零」同源）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ModelPricing {
    /// 每 100 万输入 token 的单价（`currency` 计）。
    pub input_per_million: f64,
    /// 每 100 万输出 token 的单价（`currency` 计）。
    pub output_per_million: f64,
    /// 币种代码（如 `USD` / `CNY`）。只作账本分组标签，不做汇率换算。
    pub currency: String,
}

impl ModelPricing {
    /// 把一次实测用量折成整数纳单位的金额。
    ///
    /// `None` = 这一次两侧都没测到（没有可折算的用量）：与「价格没配」是两件事，
    /// 但两者都不该写一条零账。只测到一侧时另一侧按 0 token 计（不是按未知计）。
    ///
    /// 浮点只在**这一处**出现：`token × 单价 / 1e6 × 1e9` 一次算完再四舍五入，
    /// 落格之后账是整数。配置来自磁盘（外部输入）：负数 / 非有限值按 0 计，
    /// 不让一笔脏配置产生负账或 `NaN`。
    pub fn charge(&self, usage: ModelUsage) -> Option<ModelSpend> {
        if usage.input.is_none() && usage.output.is_none() {
            return None;
        }
        let input = usage.input.unwrap_or(0) as f64;
        let output = usage.output.unwrap_or(0) as f64;
        let nano =
            input * self.input_per_million * 1000.0 + output * self.output_per_million * 1000.0;
        let amount_nano = if nano.is_finite() && nano > 0.0 {
            nano.round().min(u64::MAX as f64) as u64
        } else {
            0
        };
        Some(ModelSpend {
            amount_nano,
            currency: self.currency.trim().to_uppercase(),
        })
    }
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

    /// Token 单价（可选）。缺席 = 不折算金额；见 [`ModelPricing`] 的「缺省 vs 零」。
    pub pricing: Option<ModelPricing>,

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
            pricing: None,
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
