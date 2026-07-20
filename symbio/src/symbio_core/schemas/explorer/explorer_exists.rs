// Corresponding Frontend: tauri/src/protocols/explorer_exists.ts
use serde::{Deserialize, Serialize};

/// 检查文件是否存在请求
#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub path: String,
}

/// 检查文件是否存在响应
#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub exists: bool,
    pub is_dir: bool,
}
