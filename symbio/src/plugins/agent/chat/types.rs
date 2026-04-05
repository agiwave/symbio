//! AI 对话相关类型定义

use serde::{Deserialize, Serialize};

// /// 对话消息
// #[derive(Debug, Clone, Serialize, Deserialize)]
// pub struct ChatMessage {
//     pub role: String,
//     pub content: String,
// }

// /// 对话请求
// #[derive(Debug, Clone, Serialize, Deserialize)]
// pub struct ChatRequest {
//     pub messages: Vec<ChatMessage>,
//     #[serde(skip_serializing_if = "Option::is_none")]
//     pub model: Option<String>,
//     #[serde(skip_serializing_if = "Option::is_none")]
//     pub temperature: Option<f32>,
//     #[serde(skip_serializing_if = "Option::is_none")]
//     pub max_tokens: Option<u32>,
//     #[serde(skip_serializing_if = "Option::is_none")]
//     pub stream: Option<bool>,
// }
