//! Memory 类型定义

use serde::{Deserialize, Serialize};

/// 记忆条目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryEntry {
    pub key: String,
    pub content: String,
    pub category: String,
    pub created_at: i64,
    pub updated_at: i64,
}

impl MemoryEntry {
    pub fn new(key: String, content: String, category: Option<String>) -> Self {
        let now = chrono::Utc::now().timestamp();
        Self {
            key,
            content,
            category: category.unwrap_or_else(|| "default".to_string()),
            created_at: now,
            updated_at: now,
        }
    }
}
