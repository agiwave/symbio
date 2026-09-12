use super::types::*;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub status: String,
    pub model: String,
    pub api_base: String,
    pub has_api_key: bool,
}

/// 状态查询：读取 Provider 的模型参数（model/api_base/api_key）
///
/// 入参为持久化层 `&ModelProviderConfig`（model 插件自持 schema）。
pub fn handle_status(config: &ModelProviderConfig) -> Value {
    serde_json::to_value(Response {
        status: "ready".to_string(),
        model: config.model.clone(),
        api_base: config.api_base.clone(),
        has_api_key: config.api_key.is_some(),
    })
    .unwrap_or_default()
}
