//! OpenAI Compatible 类型定义

use serde::{Deserialize, Serialize};

/// 工具调用
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

impl ToolCall {
    pub fn new(id: impl Into<String>, name: impl Into<String>, arguments: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            arguments: arguments.into(),
        }
    }

    pub fn parse_arguments<T: for<'de> Deserialize<'de>>(&self) -> Result<T, serde_json::Error> {
        serde_json::from_str(&self.arguments)
    }
}

/// Token 使用信息
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

impl TokenUsage {
    pub fn new(input: u64, output: u64) -> Self {
        Self {
            input_tokens: Some(input),
            output_tokens: Some(output),
        }
    }
}

/// LLM 响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmResponse {
    pub text: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    pub usage: Option<TokenUsage>,
    pub reasoning_content: Option<String>,
}

impl LlmResponse {
    pub fn text(content: impl Into<String>) -> Self {
        Self {
            text: Some(content.into()),
            tool_calls: Vec::new(),
            usage: None,
            reasoning_content: None,
        }
    }

    pub fn has_tool_calls(&self) -> bool {
        !self.tool_calls.is_empty()
    }

    pub fn effective_content(&self) -> Option<&str> {
        match &self.text {
            Some(t) if !t.is_empty() => Some(t),
            _ => self.reasoning_content.as_deref(),
        }
    }
}

/// OpenAI 配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAiConfig {
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
}

fn default_temperature() -> f32 { 0.7 }
fn default_max_context_tokens() -> u32 { 128_000 }
fn default_reserved_tokens() -> u32 { 4_096 }

impl Default for OpenAiConfig {
    fn default() -> Self {
        Self {
            api_base: "https://api.openai.com/v1".to_string(),
            api_key: None,
            model: "gpt-3.5-turbo".to_string(),
            temperature: 0.7,
            max_tokens: None,
            system_prompt: None,
            max_context_tokens: default_max_context_tokens(),
            reserved_tokens: default_reserved_tokens(),
        }
    }
}

/// 聊天消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String,
    pub content: String,
}

impl ChatMessage {
    pub fn user(content: impl Into<String>) -> Self {
        Self { role: "user".to_string(), content: content.into() }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self { role: "assistant".to_string(), content: content.into() }
    }

    pub fn system(content: impl Into<String>) -> Self {
        Self { role: "system".to_string(), content: content.into() }
    }
}

/// 原生消息格式（支持 tool calls）
#[derive(Debug, Clone, Serialize)]
pub struct NativeMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<NativeToolCall>>,
}

/// 原生工具调用
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeToolCall {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub function: NativeFunctionCall,
}

/// 原生函数调用
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeFunctionCall {
    pub name: String,
    pub arguments: String,
}

/// 原生工具规格
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeToolSpec {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: NativeToolFunctionSpec,
}

/// 原生工具函数规格
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NativeToolFunctionSpec {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}
