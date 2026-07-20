// Corresponding Frontend: tauri/src/protocols/explorer_read.ts
use serde::{Deserialize, Serialize};

/// 读取文件请求
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Request {
    pub path: String,
}

/// 读取文件响应数据
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ReadData {
    pub path: String,
    pub content: String,
    pub file_type: String,
    pub size: Option<u64>,
}

/// 读取文件响应
pub type Response = ReadData;
