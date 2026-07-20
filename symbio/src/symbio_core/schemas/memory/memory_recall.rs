use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub key: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub entry: serde_json::Value, // 这里保留 Value 动态解析 MemoryEntry
}
