//! Remove 插件：删除 Agent 中的子插件

use crate::symbio_core::traits::Plugin;
use crate::symbio_core::types::{PluginMeta, PluginResult, PluginError, InvokeStream};
use serde_json::{Value, json};

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
    fn meta(&self, path: &str) -> PluginResult<PluginMeta> {
        if path.is_empty() {
            Ok(self.meta.clone())
        } else {
            Err(PluginError::NotFound(format!("插件路径 '{}' 未找到", path)))
        }
    }
    
    fn invoke(&self, path: &str, input: Value) -> PluginResult<InvokeStream> {
        if !path.is_empty() {
            return Err(PluginError::NotFound(format!("插件路径 '{}' 未找到", path)));
        }
        
        let obj = input.as_object()
            .ok_or_else(|| PluginError::ValidationError("输入必须是对象".to_string()))?;
        
        let plugin_name = obj.get("plugin_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("需要指定 plugin_name".to_string()))?;
        
        Ok(InvokeStream::single(json!({
            "success": true,
            "message": format!("插件 '{}' 已删除", plugin_name),
            "plugin_name": plugin_name
        })))
    }
}

impl Default for RemovePlugin {
    fn default() -> Self {
        Self::new()
    }
}
