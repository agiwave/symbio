//! Tools 插件工厂

use crate::core::traits::{Plugin, PluginFactory};
use crate::core::types::PluginMeta;
use super::plugin::ToolsPlugin;
use serde_json::json;
use std::sync::Arc;

pub struct ToolsFactory;

impl ToolsFactory {
    pub fn new() -> Self {
        Self
    }
}

impl Default for ToolsFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginFactory for ToolsFactory {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "tools".to_string(),
            description: "文件操作和 Shell 命令工具".to_string(),
            version: "0.1.0".to_string(),
            input: Some(json!({
                "type": "object",
                "properties": {
                    "tool": {"type": "string"},
                    "params": {"type": "object"}
                }
            })),
            output: None,
            author: Some("Symbio Team".to_string()),
        }
    }

    fn create(&self, _parent: Option<Arc<dyn Plugin>>, _config: Option<&serde_json::Value>) -> Arc<dyn Plugin> {
        Arc::new(ToolsPlugin::default())
    }
}
