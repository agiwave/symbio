// Corresponding Frontend: tauri/src/protocols/session_list.ts
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 会话列表项
#[derive(Debug, Serialize, Deserialize)]
pub struct SessionListItem {
    pub id: String,
    pub message_count: usize,
    pub updated_at: i64,
    /// 实时运行状态（由 ActiveSessionManager 合并；持久化列表默认 false）
    #[serde(default)]
    pub is_working: bool,
    /// 会话元数据摘要（workdir / title / agent_id 等）
    #[serde(default)]
    pub metadata: Value,
}

/// 会话列表响应
#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub sessions: Vec<SessionListItem>,
}
