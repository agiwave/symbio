// Corresponding Frontend: tauri/src/schemas/session_update.ts
//
// 用于 PATCH 会话元数据（写 workdir / title 等）。
// 通过 `session/update` 路径调用，合并写入 Session.metadata。
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
pub struct Request {
    pub session_id: String,
    /// 要合并写入的 metadata 字段（浅合并）
    pub metadata: Value,
    /// 可选：直接覆盖标题（会写 metadata.title）
    #[serde(default)]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Response {
    pub success: bool,
    pub session: Value,
}
