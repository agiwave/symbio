// Corresponding Frontend: tauri/src/protocols/explorer_list.ts
use serde::{Deserialize, Serialize};

/// 文件/目录项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileItem {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<FileItem>>,
}

/// 列出目录请求
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Request {
    pub path: Option<String>,
    #[serde(default)]
    pub recursive: bool,
}

/// 列出目录响应
#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub path: String,
    pub items: Vec<FileItem>,
}
