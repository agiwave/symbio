// Corresponding Frontend: tauri/src/protocols/config_get.ts
use serde::{Deserialize, Serialize};

/// 获取插件配置响应
#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub config: serde_json::Value,
}
