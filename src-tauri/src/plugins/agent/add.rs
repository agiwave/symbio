//! Add 插件：用于添加新的子插件到 Agent

use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginResult};
use crate::core::PluginFactoryRegistry;
use serde_json::{Value, json};
use std::sync::Arc;

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
    
    /// 获取所有工厂信息（用于前端构建表单）
    pub fn get_factories_info(&self) -> Vec<Value> {
        let registry = PluginFactoryRegistry::global();
        registry.list().iter().map(|f| {
            json!({
                "name": f.meta().name,
                "description": f.meta().description,
                "input_schema": f.meta().input,
                "output_schema": f.meta().output,
            })
        }).collect()
    }
}

#[async_trait::async_trait]
impl Plugin for AddPlugin {
    fn meta(&self) -> PluginMeta {
        self.meta.clone()
    }
    
    async fn invoke(&self, input: Value) -> PluginResult<Value> {
        let obj = input.as_object()
            .ok_or_else(|| crate::core::types::PluginError::ValidationError("输入必须是对象".to_string()))?;
        
        let plugin_name = obj.get("plugin_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| crate::core::types::PluginError::ValidationError("需要指定 plugin_name".to_string()))?;
        
        let config = obj.get("config").cloned();
        
        Ok(json!({
            "success": true,
            "message": format!("插件 '{}' 已添加", plugin_name),
            "plugin_name": plugin_name,
            "config": config
        }))
    }
    
    fn plugin(&self, _path: &[String]) -> Option<Arc<dyn Plugin>> {
        None
    }
}
