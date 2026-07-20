use serde::{Deserialize, Serialize};

/// Model provider configuration - Single Source of Truth
///
/// 历史名称 `AiConfig`：原 AI 插件的配置结构，现重命名为 `ModelConfig`
/// 以贴合行业惯例（"Model" 比 "AI" 更精确，且与项目其他命名风格一致）。
///
/// 注意：JSON 序列化字段保持不变（`provider` / `api_base` / `api_key` / `model` 等），
/// 已有的配置文件（含用户环境）兼容。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    /// 供应商标识 (openai, anthropic, lmstudio, ollama 等)
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

/// 推理配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningConfig {
    pub effort: String,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            provider: "".to_string(),
            api_base: "".to_string(),
            api_key: None,
            model: "".to_string(),
            temperature: 0.7,
            max_tokens: None,
            system_prompt: None,
            max_context_tokens: default_max_context_tokens(),
            reserved_tokens: default_reserved_tokens(),
            timeout_secs: default_timeout_secs(),
            api_protocol: default_api_protocol(),
            store: default_store(),
            reasoning: None,
            // previous_response_id: None,
        }
    }
}
