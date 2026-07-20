// Corresponding Frontend: tauri/src/protocols/work_get_workspace.ts
use serde::{Deserialize, Serialize};

/// 获取工作区响应
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Response {
    pub workdir: String,
    pub expanded_path: String,
    #[serde(default)]
    pub recent_workspaces: Vec<String>,
}
