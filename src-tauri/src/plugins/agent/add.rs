//! Add 插件：用于添加新的子插件到 Agent

use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginResult, PluginError, InvokeStream};
use crate::core::PluginFactoryRegistry;
use serde_json::{Value, json};

pub struct AddPlugin {
    meta: PluginMeta,
}

impl AddPlugin {
    pub fn new() -> Self {
        let registry = PluginFactoryRegistry::global();
        let plugin_names: Vec<String> = registry.list()
            .iter()
            .map(|f| f.meta().name.clone())
            .collect();
        
        AddPlugin {
            meta: PluginMeta {
                name: "add".to_string(),
                description: "添加新的子插件到 Agent".to_string(),
                version: "0.1.0".to_string(),
                input: Some(json!({
                    "type": "object",
                    "properties": {
                        "plugin_name": {
                            "type": "string",
                            "description": "要添加的插件名称",
                            "enum": plugin_names
                        },
                        "config": {
                            "type": "object",
                            "description": "插件的配置参数"
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
impl Plugin for AddPlugin {
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
        
        let config = obj.get("config").cloned();
        
        Ok(InvokeStream::single(json!({
            "success": true,
            "message": format!("插件 '{}' 已添加", plugin_name),
            "plugin_name": plugin_name,
            "config": config
        })))
    }
}
