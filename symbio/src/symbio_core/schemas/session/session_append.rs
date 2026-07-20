// Corresponding Frontend: tauri/src/protocols/session_append.ts
use super::chat_message::ChatMessage;
use serde::{Deserialize, Serialize};

/// 追加消息请求
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Request {
    pub session_id: String,
    pub messages: Vec<ChatMessage>,
}

/// 追加消息响应
#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub message_count: usize,
}
