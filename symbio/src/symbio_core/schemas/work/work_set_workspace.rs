// Corresponding Frontend: tauri/src/protocols/work_set_workspace.ts
use serde::{Deserialize, Serialize};

/// 设置工作区请求
#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub path: String,
}

/// 设置工作区响应
#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub workdir: String,
    pub expanded_path: String,
    #[serde(default)]
    pub recent_workspaces: Vec<String>,
    pub status: String,
}
