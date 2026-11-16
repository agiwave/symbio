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

use crate::round::{ModelSpend, ModelUsage};

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
    ///
    /// 缺席 / `null` / 空串都按 0 读：详情页的计价是**可选子表单**，用户不填时
    /// 前端只会提交一个空的 `pricing` 对象；把「没填」变成 `missing field` 错误，
    /// 等于让可选参数阻塞整条保存链路。规范化的「空币种 + 零单价 ⇒ 未配置」在
    /// `ModelPlugin::normalize_pricing` 一处收口。
    #[serde(default, deserialize_with = "de_f64_or_zero")]
    pub input_per_million: f64,
    /// 每 100 万输出 token 的单价（`currency` 计）。
    #[serde(default, deserialize_with = "de_f64_or_zero")]
    pub output_per_million: f64,
    /// 币种代码（如 `USD` / `CNY`）。只作账本分组标签，不做汇率换算。
    #[serde(default, deserialize_with = "de_string_or_empty")]
    pub currency: String,
}

/// 可选数字输入的统一读法：缺席 / `null` / 空串 = 0，数字或数字字符串照读。
///
/// 表单的 `number` widget 在编辑态用空串表示「没填」；这条边界与
/// [`de_opt_u32`] 同源——外部输入（磁盘配置 / 前端表单）的「空」不是类型错误。
fn de_f64_or_zero<'de, D>(deserializer: D) -> Result<f64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    match Option::<serde_json::Value>::deserialize(deserializer)? {
        None | Some(serde_json::Value::Null) => Ok(0.0),
        Some(serde_json::Value::Number(n)) => n
            .as_f64()
            .ok_or_else(|| Error::custom("单价超出 f64 可表示范围")),
        Some(serde_json::Value::String(s)) if s.trim().is_empty() => Ok(0.0),
        Some(serde_json::Value::String(s)) => s.trim().parse::<f64>().map_err(Error::custom),
        Some(other) => Err(Error::custom(format!("单价必须是数字，得到 {other}"))),
    }
}

/// 文本输入的宽容读法：`null` = 空串（`currency` 的空值语义就是「未填写」）。
fn de_string_or_empty<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    match Option::<serde_json::Value>::deserialize(deserializer)? {
        None | Some(serde_json::Value::Null) => Ok(String::new()),
        Some(serde_json::Value::String(s)) => Ok(s),
        Some(other) => Err(Error::custom(format!("币种必须是字符串，得到 {other}"))),
    }
}

/// `max_tokens` 的宽容读法：缺席 / `null` / 空串 = 不限制（`None`）。
///
/// 「不限制」是**缺键**而不是一个哨兵数值：协议层据此决定「省略字段，交给
/// 供应商默认」；只有 Anthropic 这类必填协议才回落到协议档案的缺省值。
fn de_opt_u32<'de, D>(deserializer: D) -> Result<Option<u32>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    match Option::<serde_json::Value>::deserialize(deserializer)? {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::Number(n)) => n
            .as_u64()
            .and_then(|v| u32::try_from(v).ok())
            .map(Some)
            .ok_or_else(|| Error::custom("max_tokens 必须是 u32 范围内的整数")),
        Some(serde_json::Value::String(s)) if s.trim().is_empty() => Ok(None),
        Some(serde_json::Value::String(s)) => {
            s.trim().parse::<u32>().map(Some).map_err(Error::custom)
        }
        Some(other) => Err(Error::custom(format!(
            "max_tokens 必须是整数，得到 {other}"
        ))),
    }
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
    #[serde(default, deserialize_with = "de_opt_u32")]
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

    /// 最小请求间隔（毫秒）；0 表示不限制。
    ///
    /// **当前没有执行方**：这个字段被解析、被详情页展示、被示例配置带着，但
    /// 请求路径上没有任何一处读它——此前 `ModelProvider::rate_limit_ms` 是它
    /// 唯一的出口，而那个出口没有调用方（`D-437` 删掉了那个 getter，让缺口留在
    /// **字段**上而不是靠一个没人调的 getter 假装有执行面）。限流真正的落地位置
    /// 应当是 `http::execute_post_with_abort` 之前的一道闸（每分钟 x 次 / 最小间隔），
    /// 那是独立的一笔，不在 `D-437` 里顺手做。
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
