use super::types::*;
use serde_json::Value;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub status: String,
    pub model: String,
    pub api_base: String,
    pub has_api_key: bool,
}

pub fn handle_status(config: &ModelConfig) -> Value {
    serde_json::to_value(Response {
        status: "ready".to_string(),
        model: config.model.clone(),
        api_base: config.api_base.clone(),
        has_api_key: config.api_key.is_some(),
    })
    .unwrap_or_default()
}
