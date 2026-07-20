use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct MemoryListItem {
    pub key: String,
    pub category: Option<String>,
    pub created_at: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub memories: Vec<MemoryListItem>,
    pub count: usize,
}
