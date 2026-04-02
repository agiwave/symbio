//! OpenAI Compatible 插件工厂

use crate::core::traits::{Plugin, PluginFactory};
use crate::core::types::PluginMeta;
use super::plugin::OpenAiPlugin;
use super::types::OpenAiConfig;
use serde_json::{Value, json};
use std::sync::{Arc, Weak};

pub struct OpenAiFactory;

impl OpenAiFactory {
    pub fn new() -> Self {
        Self
    }
}

impl Default for OpenAiFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginFactory for OpenAiFactory {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "openai".to_string(),
            description: "OpenAI 兼容 LLM API 集成".to_string(),
            version: "0.1.0".to_string(),
            input: Some(json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["chat", "status", "list_models", "configure", "get_config", "compress_info"]
                    }
                }
            })),
            output: None,
            author: Some("Symbio Team".to_string()),
        }
    }

    fn create(&self, parent: Option<Weak<dyn Plugin>>, config: Option<&serde_json::Value>) -> Arc<dyn Plugin> {
        let openai_config: OpenAiConfig = config
            .and_then(|v| normalize_config(v))
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();

        let parent_weak = parent;
        Arc::new(OpenAiPlugin::new(parent_weak, openai_config))
    }
}

/// 规范化配置字段名
fn normalize_config(config: &Value) -> Option<Value> {
    if let Value::Object(map) = config {
        let mut normalized = serde_json::Map::new();
        for (key, value) in map {
            let normalized_key = match key.as_str() {
                "baseUrl" => "api_base",
                "apiKey" => "api_key",
                k => k,
            };
            normalized.insert(normalized_key.to_string(), value.clone());
        }
        Some(Value::Object(normalized))
    } else {
        None
    }
}
