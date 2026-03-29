//! Echo Plugin

use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginResult};
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Clone)]
pub struct EchoPlugin {
    meta: PluginMeta,
}

impl EchoPlugin {
    pub fn new() -> Self {
        EchoPlugin {
            meta: PluginMeta {
                name: "echo".to_string(),
                description: "回显输入内容，用于测试".to_string(),
                version: "0.1.0".to_string(),
                input: Some(json!({
                    "type": "object",
                    "properties": {
                        "message": {
                            "type": "string",
                            "description": "要回显的消息",
                            "default": "Hello, World!"
                        },
                        "uppercase": {
                            "type": "boolean",
                            "description": "是否转换为大写",
                            "default": false
                        }
                    },
                    "required": ["message"]
                })),
                output: Some(json!({
                    "type": "object",
                    "properties": {
                        "original": {
                            "type": "string",
                            "description": "原始输入"
                        },
                        "echoed": {
                            "type": "string",
                            "description": "回显输出"
                        }
                    },
                    "required": ["original", "echoed"]
                })),
                author: Some("Symbio Team".to_string()),
            },
        }
    }
}

#[async_trait::async_trait]
impl Plugin for EchoPlugin {
    fn meta(&self) -> PluginMeta {
        self.meta.clone()
    }
    
    async fn invoke(&self, input: Value) -> PluginResult<Value> {
        let obj = input.as_object()
            .ok_or_else(|| crate::core::types::PluginError::ValidationError("输入必须是对象".to_string()))?;
        
        let message = obj.get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Hello, World!");
        
        let uppercase = obj.get("uppercase")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        
        let echoed = if uppercase {
            message.to_uppercase()
        } else {
            message.to_string()
        };
        
        Ok(Value::Object(serde_json::Map::from_iter([
            ("original".to_string(), Value::String(message.to_string())),
            ("echoed".to_string(), Value::String(echoed)),
        ])))
    }
    
    fn plugin(&self, _path: &[String]) -> Option<Arc<dyn Plugin>> {
        None
    }
}

impl Default for EchoPlugin {
    fn default() -> Self {
        Self::new()
    }
}
