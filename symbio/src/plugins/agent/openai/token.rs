//! Token 计数和上下文管理

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

// ═══════════════════════════════════════════════════════════════════════════
// 模型配置
// ═══════════════════════════════════════════════════════════════════════════

/// Tokenizer 编码类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TokenizerEncoding {
    #[default]
    Cl100kBase,
    O200kBase,
}

/// 模型能力配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub name: String,
    pub max_context_tokens: u32,
    pub encoding: TokenizerEncoding,
    pub max_output_tokens: Option<u32>,
    pub supports_vision: bool,
    pub supports_tools: bool,
}

impl ModelConfig {
    pub fn new(name: impl Into<String>, max_context: u32, encoding: TokenizerEncoding) -> Self {
        Self {
            name: name.into(),
            max_context_tokens: max_context,
            encoding,
            max_output_tokens: None,
            supports_vision: false,
            supports_tools: true,
        }
    }

    pub fn with_max_output(mut self, max: u32) -> Self {
        self.max_output_tokens = Some(max);
        self
    }

    pub fn with_vision(mut self) -> Self {
        self.supports_vision = true;
        self
    }

    pub fn reserved_tokens(&self) -> u32 {
        if self.max_context_tokens > 100_000 { 8_192 } else { 4_096 }
    }
}

/// 获取已知模型配置
pub fn get_model_config(model_name: &str) -> ModelConfig {
    let models = get_known_models();
    
    // 尝试精确匹配
    if let Some(config) = models.get(model_name) {
        return config.clone();
    }
    
    // 模糊匹配
    let lower_name = model_name.to_lowercase();
    for (key, config) in models.iter() {
        if lower_name.starts_with(key) || key.starts_with(&lower_name) {
            return config.clone();
        }
    }
    
    // 默认配置
    ModelConfig::new(model_name, 128_000, TokenizerEncoding::Cl100kBase)
}

fn get_known_models() -> &'static HashMap<&'static str, ModelConfig> {
    use std::sync::OnceLock;
    static KNOWN_MODELS: OnceLock<HashMap<&'static str, ModelConfig>> = OnceLock::new();
    
    KNOWN_MODELS.get_or_init(|| {
        let mut models = HashMap::new();
        
        // GPT-4o 系列
        models.insert("gpt-4o", ModelConfig::new("gpt-4o", 128_000, TokenizerEncoding::O200kBase).with_max_output(16_384).with_vision());
        models.insert("gpt-4o-mini", ModelConfig::new("gpt-4o-mini", 128_000, TokenizerEncoding::O200kBase).with_max_output(16_384).with_vision());
        models.insert("gpt-4o-2024-11-20", ModelConfig::new("gpt-4o-2024-11-20", 128_000, TokenizerEncoding::O200kBase).with_max_output(16_384).with_vision());
        models.insert("gpt-4o-2024-08-06", ModelConfig::new("gpt-4o-2024-08-06", 128_000, TokenizerEncoding::O200kBase).with_max_output(16_384).with_vision());
        
        // o1 系列
        models.insert("o1", ModelConfig::new("o1", 200_000, TokenizerEncoding::O200kBase).with_max_output(100_000));
        models.insert("o1-preview", ModelConfig::new("o1-preview", 128_000, TokenizerEncoding::O200kBase).with_max_output(32_768));
        models.insert("o1-mini", ModelConfig::new("o1-mini", 128_000, TokenizerEncoding::O200kBase).with_max_output(65_536));
        models.insert("o3-mini", ModelConfig::new("o3-mini", 200_000, TokenizerEncoding::O200kBase).with_max_output(100_000));
        
        // GPT-4 系列
        models.insert("gpt-4-turbo", ModelConfig::new("gpt-4-turbo", 128_000, TokenizerEncoding::Cl100kBase).with_max_output(4_096).with_vision());
        models.insert("gpt-4-turbo-preview", ModelConfig::new("gpt-4-turbo-preview", 128_000, TokenizerEncoding::Cl100kBase).with_max_output(4_096));
        models.insert("gpt-4", ModelConfig::new("gpt-4", 8_192, TokenizerEncoding::Cl100kBase));
        models.insert("gpt-4-32k", ModelConfig::new("gpt-4-32k", 32_768, TokenizerEncoding::Cl100kBase));
        
        // GPT-3.5 系列
        models.insert("gpt-3.5-turbo", ModelConfig::new("gpt-3.5-turbo", 16_385, TokenizerEncoding::Cl100kBase).with_max_output(4_096));
        models.insert("gpt-3.5-turbo-16k", ModelConfig::new("gpt-3.5-turbo-16k", 16_385, TokenizerEncoding::Cl100kBase).with_max_output(4_096));
        
        // Claude 系列
        models.insert("claude-3-opus", ModelConfig::new("claude-3-opus", 200_000, TokenizerEncoding::Cl100kBase).with_max_output(4_096).with_vision());
        models.insert("claude-3-sonnet", ModelConfig::new("claude-3-sonnet", 200_000, TokenizerEncoding::Cl100kBase).with_max_output(4_096).with_vision());
        models.insert("claude-3-haiku", ModelConfig::new("claude-3-haiku", 200_000, TokenizerEncoding::Cl100kBase).with_max_output(4_096).with_vision());
        models.insert("claude-3-5-sonnet", ModelConfig::new("claude-3-5-sonnet", 200_000, TokenizerEncoding::Cl100kBase).with_max_output(8_192).with_vision());
        
        // Kimi (Moonshot)
        models.insert("moonshot-v1-8k", ModelConfig::new("moonshot-v1-8k", 8_192, TokenizerEncoding::Cl100kBase));
        models.insert("moonshot-v1-32k", ModelConfig::new("moonshot-v1-32k", 32_768, TokenizerEncoding::Cl100kBase));
        models.insert("moonshot-v1-128k", ModelConfig::new("moonshot-v1-128k", 128_000, TokenizerEncoding::Cl100kBase));
        
        // DeepSeek
        models.insert("deepseek-chat", ModelConfig::new("deepseek-chat", 64_000, TokenizerEncoding::Cl100kBase));
        models.insert("deepseek-coder", ModelConfig::new("deepseek-coder", 64_000, TokenizerEncoding::Cl100kBase));
        
        // Qwen
        models.insert("qwen-turbo", ModelConfig::new("qwen-turbo", 32_000, TokenizerEncoding::Cl100kBase));
        models.insert("qwen-plus", ModelConfig::new("qwen-plus", 32_000, TokenizerEncoding::Cl100kBase));
        models.insert("qwen-max", ModelConfig::new("qwen-max", 32_000, TokenizerEncoding::Cl100kBase));
        
        // GLM
        models.insert("glm-4", ModelConfig::new("glm-4", 128_000, TokenizerEncoding::Cl100kBase));
        models.insert("glm-4-plus", ModelConfig::new("glm-4-plus", 128_000, TokenizerEncoding::Cl100kBase));
        
        models
    })
}

// ═══════════════════════════════════════════════════════════════════════════
// Token 计数器
// ═══════════════════════════════════════════════════════════════════════════

/// Token 计数器
/// 使用快速估计算法：ASCII ~4 字符/token，CJK ~1.5 字符/token
pub struct TokenCounter {
    encoding: TokenizerEncoding,
}

impl TokenCounter {
    pub fn new(encoding: TokenizerEncoding) -> Self {
        Self { encoding }
    }

    pub fn for_model(model_name: &str) -> Self {
        let config = get_model_config(model_name);
        Self::new(config.encoding)
    }

    /// 计算文本中的 token 数量
    pub fn count_tokens(&self, text: &str) -> usize {
        if text.is_empty() {
            return 0;
        }
        
        // 快速估计算法
        let chars = text.chars().count();
        let ascii_chars = text.chars().filter(|c| c.is_ascii()).count();
        let non_ascii_chars = chars - ascii_chars;
        
        // ASCII: 4 字符/token，非 ASCII (CJK): 1.5 字符/token
        (ascii_chars / 4) + ((non_ascii_chars * 2) / 3) + 1
    }

    /// 计算消息的 token 数量
    pub fn count_message_tokens(&self, role: &str, content: &str) -> usize {
        4 + self.count_tokens(role) + self.count_tokens(content)
    }

    pub fn encoding(&self) -> TokenizerEncoding {
        self.encoding
    }
}

impl Default for TokenCounter {
    fn default() -> Self {
        Self::new(TokenizerEncoding::Cl100kBase)
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 上下文管理策略
// ═══════════════════════════════════════════════════════════════════════════

/// 上下文管理策略
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ContextStrategy {
    #[default]
    TruncateOldest,
    SmartSelect,
}

/// 上下文配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextConfig {
    pub max_tokens: usize,
    pub reserved_tokens: usize,
    pub strategy: ContextStrategy,
    pub compression_threshold: f32,
    pub min_messages_to_keep: usize,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            max_tokens: 128_000,
            reserved_tokens: 4_096,
            strategy: ContextStrategy::SmartSelect,
            compression_threshold: 0.8,
            min_messages_to_keep: 4,
        }
    }
}

impl ContextConfig {
    pub fn for_model(model_name: &str) -> Self {
        let model_config = get_model_config(model_name);
        Self {
            max_tokens: model_config.max_context_tokens as usize,
            reserved_tokens: model_config.reserved_tokens() as usize,
            ..Default::default()
        }
    }

    pub fn available_tokens(&self) -> usize {
        self.max_tokens.saturating_sub(self.reserved_tokens)
    }

    pub fn should_compress(&self, current_tokens: usize) -> bool {
        let threshold = (self.max_tokens as f32 * self.compression_threshold) as usize;
        current_tokens >= threshold
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// 消息优先级
// ═══════════════════════════════════════════════════════════════════════════

/// 消息优先级
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MessagePriority {
    Low = 0,
    Normal = 1,
    High = 2,
    Critical = 3,
}

// ═══════════════════════════════════════════════════════════════════════════
// API 错误处理
// ═══════════════════════════════════════════════════════════════════════════

/// API 错误类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApiErrorType {
    NetworkError,
    RateLimited,
    ServerError,
    ClientError,
    Timeout,
    AuthError,
    ContextLengthExceeded,
    Unknown,
}

/// 重试配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryConfig {
    pub max_retries: u32,
    pub initial_delay_ms: u64,
    pub max_delay_ms: u64,
    pub backoff_multiplier: f32,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay_ms: 1000,
            max_delay_ms: 60_000,
            backoff_multiplier: 2.0,
        }
    }
}

impl RetryConfig {
    pub fn delay_for_attempt(&self, attempt: u32) -> std::time::Duration {
        let delay = self.initial_delay_ms as f32 * self.backoff_multiplier.powi(attempt as i32);
        let delay = delay.min(self.max_delay_ms as f32);
        std::time::Duration::from_millis(delay as u64)
    }

    pub fn should_retry(&self, error_type: ApiErrorType, attempt: u32) -> bool {
        if attempt >= self.max_retries {
            return false;
        }
        matches!(
            error_type,
            ApiErrorType::NetworkError | ApiErrorType::RateLimited | ApiErrorType::ServerError | ApiErrorType::Timeout
        )
    }
}

/// 分类 API 错误
pub fn classify_api_error(status: Option<u16>, error_message: &str) -> ApiErrorType {
    let lower = error_message.to_lowercase();
    
    if lower.contains("timeout") || lower.contains("timed out") {
        return ApiErrorType::Timeout;
    }
    if lower.contains("rate limit") || lower.contains("too many requests") {
        return ApiErrorType::RateLimited;
    }
    if lower.contains("context length") || lower.contains("maximum context") {
        return ApiErrorType::ContextLengthExceeded;
    }
    if lower.contains("unauthorized") || lower.contains("forbidden") || lower.contains("invalid api key") {
        return ApiErrorType::AuthError;
    }
    if lower.contains("connection") || lower.contains("network") || lower.contains("dns") {
        return ApiErrorType::NetworkError;
    }
    
    if let Some(code) = status {
        match code {
            401 | 403 => ApiErrorType::AuthError,
            429 => ApiErrorType::RateLimited,
            400 | 404 | 405 | 422 => ApiErrorType::ClientError,
            500 | 502 | 503 | 504 => ApiErrorType::ServerError,
            _ => ApiErrorType::Unknown,
        }
    } else {
        ApiErrorType::Unknown
    }
}
