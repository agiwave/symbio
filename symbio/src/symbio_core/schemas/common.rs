// Corresponding Frontend: tauri/src/schemas/common.ts
use serde::{Deserialize, Serialize};

/// Generic success response - 保持向后兼容
pub type SuccessResponse = String;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaResponse {
    pub schema: serde_json::Value,
}

/// 通用成功响应（带状态和消息）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimpleResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl SimpleResponse {
    pub fn success() -> Self {
        Self {
            status: "success".to_string(),
            message: None,
        }
    }

    pub fn success_with_message(message: impl Into<String>) -> Self {
        Self {
            status: "success".to_string(),
            message: Some(message.into()),
        }
    }

    pub fn ok() -> Self {
        Self {
            status: "ok".to_string(),
            message: None,
        }
    }
}
