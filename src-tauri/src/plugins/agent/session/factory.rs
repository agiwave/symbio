//! Session 插件工厂

use crate::core::traits::{Plugin, PluginFactory};
use crate::core::types::PluginMeta;
use super::plugin::SessionPlugin;
use serde_json::json;
use std::sync::Arc;

pub struct SessionFactory;

impl SessionFactory {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SessionFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginFactory for SessionFactory {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "session".to_string(),
            description: "会话历史和上下文管理".to_string(),
            version: "0.1.0".to_string(),
            input: Some(json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["get", "append", "clear", "list", "get_context"]
                    }
                }
            })),
            output: None,
            author: Some("Symbio Team".to_string()),
        }
    }

    fn create(&self, _parent: Option<Arc<dyn Plugin>>, _config: Option<&serde_json::Value>) -> Arc<dyn Plugin> {
        Arc::new(SessionPlugin::default())
    }
}