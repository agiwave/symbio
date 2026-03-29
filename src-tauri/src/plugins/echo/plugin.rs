//! Echo Plugin

use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginResult, PluginError, InvokeStream};
use serde_json::{Value, json};

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
        
        let result = Value::Object(serde_json::Map::from_iter([
            ("original".to_string(), Value::String(message.to_string())),
            ("echoed".to_string(), Value::String(echoed)),
        ]));
        
        Ok(InvokeStream::single(result))
    }
}

impl Default for EchoPlugin {
    fn default() -> Self {
        Self::new()
    }
}
