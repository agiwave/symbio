use super::types::*;
use crate::symbio_core::schemas::model::model_status;
use serde_json::Value;

pub fn handle_status(config: &ModelConfig) -> Value {
    serde_json::to_value(model_status::Response {
        status: "ready".to_string(),
        model: config.model.clone(),
        api_base: config.api_base.clone(),
        has_api_key: config.api_key.is_some(),
    })
    .unwrap_or_default()
}
