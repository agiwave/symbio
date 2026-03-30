//! AI 对话插件工厂

use crate::core::traits::{Plugin, PluginFactory};
use crate::core::types::PluginMeta;
use super::plugin::ChatPlugin;
use serde_json::json;
use std::sync::Arc;

pub struct ChatFactory;

impl ChatFactory {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ChatFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginFactory for ChatFactory {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "chat".to_string(),
            description: "AI 对话插件，支持 OpenAI 兼容 API".to_string(),
            version: "0.1.0".to_string(),
            input: Some(json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["send", "stream", "configure", "get_config"]
                    }
                }
            })),
            output: None,
            author: Some("Symbio Team".to_string()),
        }
    }

    fn create(&self, _parent: Option<Arc<dyn Plugin>>, _config: Option<&serde_json::Value>) -> Arc<dyn Plugin> {
        Arc::new(ChatPlugin::new())
    }
}
