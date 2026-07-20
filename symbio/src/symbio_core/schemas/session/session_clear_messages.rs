// Corresponding Frontend: tauri/src/schemas/session_clear_messages.ts
use serde::{Deserialize, Serialize};

/// 清空会话消息请求（保留会话元数据 / 工作目录 / 标题等）。
///
/// 与 `session_clear`（删除整个会话）不同，本操作只清空 `session.messages`，
/// 会话本身（metadata）仍然保留。
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Request {
    pub session_id: String,
}

/// 清空结果
#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub cleared: bool,
}
