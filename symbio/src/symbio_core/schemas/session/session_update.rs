// 前端**不再**消费本协议（原对应文件 tauri/src/schemas/session_update.ts 已删除）。
//
// 消费方只有 CLI（cli/src/client.rs::ensure_session）：它需要**客户端指定会话 id**
// 后 upsert，而 VDFS 新建会话是 provider 生成 id（id 是存储细节，不属于使用方的
// 知识）。前端改 metadata / 标题走 `vdfs/write(.vdfs/session/<id>)`。
//
// 两条路径的 metadata 浅合并是**同一份实现**（`Session::merge_metadata_object`），
// 因此不会漂移——回归锚点在 `plugins/session/handlers.test.rs`。
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
