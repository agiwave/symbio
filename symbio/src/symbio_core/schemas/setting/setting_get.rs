use serde::{Deserialize, Serialize};

/// 获取设置请求
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Request {
    pub category: String,
}

/// 获取设置响应
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Response {
    pub category: String,
    pub settings: serde_json::Value,
}
