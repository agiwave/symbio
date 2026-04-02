//! AI 对话插件工厂

use crate::core::traits::{Plugin, PluginFactory};
use crate::core::types::PluginMeta;
use super::plugin::ChatPlugin;
use serde_json::{json, Value};
use std::sync::{Arc, Weak};

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

    fn create(&self, parent: Option<Weak<dyn Plugin>>, _config: Option<&Value>) -> Arc<dyn Plugin> {
        // ChatPlugin 不需要配置，只需要父引用
        let parent_weak = parent;
        Arc::new(ChatPlugin::new(parent_weak))
    }
}
