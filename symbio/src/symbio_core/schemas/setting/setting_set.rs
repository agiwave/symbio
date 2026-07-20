use serde::{Deserialize, Serialize};

/// 更新设置请求
#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub category: String,
    pub key: String,
    pub value: serde_json::Value,
}

/// 更新设置响应
#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub category: String,
    pub key: String,
    pub message: String,
}
