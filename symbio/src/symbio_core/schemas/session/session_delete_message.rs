// Corresponding Frontend: tauri/src/schemas/session_delete_message.ts
use serde::{Deserialize, Serialize};

/// 删除单条会话消息请求。
///
/// 后端会从**已排序的消息列表**中定位目标消息，连同其**之后的所有消息**一并删除
/// （保证会话连续性，无需 parent_id 级联）。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Request {
    pub session_id: String,
    pub message_id: String,
}

/// 删除结果
#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    /// 被删除的消息总数（含目标消息及其之后的所有消息）
    pub deleted: usize,
    /// 被删除消息的 id 列表（前端据此精确移除本地状态）
    pub deleted_ids: Vec<String>,
}
