//! Memory 插件工厂

use crate::core::traits::{Plugin, PluginFactory};
use crate::core::types::PluginMeta;
use super::plugin::MemoryPlugin;
use serde_json::json;

pub struct MemoryFactory;

impl MemoryFactory {
    pub fn new() -> Self {
        Self
    }
}

impl Default for MemoryFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginFactory for MemoryFactory {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "memory".to_string(),
            description: "持久化记忆存储".to_string(),
            version: "0.1.0".to_string(),
            input: Some(json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["store", "recall", "forget", "list", "search"]
                    }
                }
            })),
            output: None,
            author: Some("Symbio Team".to_string()),
        }
    }

    fn create(&self, _parent: Option<&dyn Plugin>, _config: Option<&serde_json::Value>) -> std::sync::Arc<dyn Plugin> {
        std::sync::Arc::new(MemoryPlugin::default())
    }
}
