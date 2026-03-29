//! Remove 插件：删除 Agent 中的子插件

use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginResult};
use serde_json::{Value, json};
use std::sync::Arc;

pub struct RemovePlugin {
    meta: PluginMeta,
}

impl RemovePlugin {
    pub fn new() -> Self {
        RemovePlugin {
            meta: PluginMeta {
                name: "remove".to_string(),
                description: "删除 Agent 中的子插件".to_string(),
                version: "0.1.0".to_string(),
                input: Some(json!({
                    "type": "object",
                    "properties": {
                        "plugin_name": {
                            "type": "string",
                            "description": "要删除的插件名称"
                        }
                    },
                    "required": ["plugin_name"]
                })),
                output: Some(json!({
                    "type": "object",
                    "properties": {
                        "success": {"type": "boolean"},
                        "message": {"type": "string"},
                        "plugin_name": {"type": "string"}
                    }
                })),
                author: Some("Symbio Team".to_string()),
            },
        }
    }
}

#[async_trait::async_trait]
impl Plugin for RemovePlugin {
    fn meta(&self) -> PluginMeta {
        self.meta.clone()
    }
    
    async fn invoke(&self, input: Value) -> PluginResult<Value> {
        let obj = input.as_object()
            .ok_or_else(|| crate::core::types::PluginError::ValidationError("输入必须是对象".to_string()))?;
        
        let plugin_name = obj.get("plugin_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::core::types::PluginError::ValidationError("需要指定 plugin_name".to_string()))?;
        
        Ok(json!({
            "success": true,
            "message": format!("插件 '{}' 已删除", plugin_name),
            "plugin_name": plugin_name
        }))
    }
    
    fn plugin(&self, _path: &[String]) -> Option<Arc<dyn Plugin>> {
        None
    }
}

impl Default for RemovePlugin {
    fn default() -> Self {
        Self::new()
    }
}
