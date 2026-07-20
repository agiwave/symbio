// Corresponding Frontend: tauri/src/schemas/session_update_message.ts
use super::chat_message::ChatMessage;
use serde::{Deserialize, Serialize};

/// 更新单条会话消息请求（手工编辑 / 标错重试等场景）。
///
/// `message` 必须携带 `id`；其余字段为可选覆盖项，仅覆盖提供的字段
/// （content / status / error / meta 等）。后端按 id 定位并局部更新该消息。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Request {
    pub session_id: String,
    pub message: ChatMessage,
}

/// 更新结果
#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub updated: bool,
}
