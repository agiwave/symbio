// Corresponding Frontend: tauri/src/schemas/common.ts
use serde::{Deserialize, Serialize};

/// Generic success response - 保持向后兼容
pub type SuccessResponse = String;

/// **配置切片** —— `save_config` 的载荷（写配置者推给宿主的那一份）
///
/// 写配置的插件只带**自己那一份**：`plugin` 是插件名（= 工厂 id，宿主据此在
/// `symbio.plugins` 下定位既有条目），`config` 是该插件的配置对象。
/// 宿主据此合并落盘，**不反向拉取**任何插件的配置——「读配置」不再同时承担
/// 「给 UI 显示」与「给宿主落盘」两个不相干的用途。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigSlice {
    /// 插件名（工厂 id，如 `session` / `gateway`）
    pub plugin: String,
    /// 该插件的配置对象（其字段即配置文档的字段）
    pub config: serde_json::Value,
}

impl ConfigSlice {
    pub fn new(plugin: impl Into<String>, config: serde_json::Value) -> Self {
        Self {
            plugin: plugin.into(),
            config,
        }
    }
}

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
