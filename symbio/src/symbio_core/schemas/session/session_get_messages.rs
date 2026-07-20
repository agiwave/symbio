// Corresponding Frontend: tauri/src/protocols/session_get_messages.ts
use super::chat_message::ChatMessage;
use serde::{Deserialize, Serialize};

/// 获取会话消息请求
///
/// 设计说明：
/// - 不分页获取完整会话历史
/// - AI插件和前端共用此接口
/// - 如果会话历史被压缩或剪裁，返回的是压缩/剪裁后的内容
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Request {
    pub session_id: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Response {
    pub messages: Vec<ChatMessage>,
}
