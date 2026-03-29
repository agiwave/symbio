//! Formatter Plugin

use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginResult, StreamChunk};
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Clone)]
pub struct FormatterPlugin {
    meta: PluginMeta,
}

impl FormatterPlugin {
    pub fn new() -> Self {
        FormatterPlugin {
            meta: PluginMeta {
                name: "formatter".to_string(),
                description: "文本格式化插件，支持多种格式输出".to_string(),
                version: "0.1.0".to_string(),
                input: Some(json!({
                    "type": "object",
                    "properties": {
                        "text": {
                            "type": "string",
                            "description": "要格式化的文本",
                            "default": "Hello, World!"
                        },
                        "format": {
                            "type": "string",
                            "description": "格式化类型",
                            "default": "uppercase",
                            "enum": ["uppercase", "lowercase", "reverse", "word_count"]
                        }
                    },
                    "required": ["text", "format"]
                })),
                output: Some(json!({
                    "type": "object",
                    "properties": {
                        "original": {
                            "type": "string",
                            "description": "原始文本"
                        },
                        "formatted": {
                            "type": "string",
                            "description": "格式化后的文本"
                        },
                        "format_type": {
                            "type": "string",
                            "description": "使用的格式化类型"
                        }
                    },
                    "required": ["original", "formatted", "format_type"]
                })),
                author: Some("Symbio Team".to_string()),
            },
        }
    }
}

#[async_trait::async_trait]
impl Plugin for FormatterPlugin {
    fn meta(&self) -> PluginMeta {
        self.meta.clone()
    }
    
    async fn invoke(&self, input: Value) -> PluginResult<Value> {
        let obj = input.as_object()
            .ok_or_else(|| crate::core::types::PluginError::ValidationError("输入必须是对象".to_string()))?;
        
        let text = obj.get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("Hello, World!");
        
        let format = obj.get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("uppercase");
        
        let formatted = match format {
            "uppercase" => text.to_uppercase(),
            "lowercase" => text.to_lowercase(),
            "reverse" => text.chars().rev().collect(),
            "word_count" => {
                let count = text.split_whitespace().count();
                format!("单词数：{}", count)
            }
            _ => return Err(crate::core::types::PluginError::ValidationError(format!("未知的格式化类型：{}", format))),
        };
        
        Ok(Value::Object(serde_json::Map::from_iter([
            ("original".to_string(), Value::String(text.to_string())),
            ("formatted".to_string(), Value::String(formatted)),
            ("format_type".to_string(), Value::String(format.to_string())),
        ])))
    }
    
    async fn sinvoke(&self, input: Value) -> PluginResult<Vec<StreamChunk>> {
        let obj = input.as_object()
            .ok_or_else(|| crate::core::types::PluginError::ValidationError("输入必须是对象".to_string()))?;
        
        let text = obj.get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("Hello, World!");
        
        let format = obj.get("format")
            .and_then(|v| v.as_str())
            .unwrap_or("uppercase");
        
        let formatted = match format {
            "uppercase" => text.to_uppercase(),
            "lowercase" => text.to_lowercase(),
            "reverse" => text.chars().rev().collect(),
            "word_count" => {
                let count = text.split_whitespace().count();
                format!("单词数：{}", count)
            }
            _ => return Err(crate::core::types::PluginError::ValidationError(format!("未知的格式化类型：{}", format))),
        };
        
        let mut chunks = Vec::new();
        let chars: Vec<char> = formatted.chars().collect();
        
        for (i, ch) in chars.iter().enumerate() {
            chunks.push(StreamChunk {
                data: Value::String(ch.to_string()),
                done: i == chars.len() - 1,
            });
        }
        
        Ok(chunks)
    }
    
    fn plugin(&self, _path: &[String]) -> Option<Arc<dyn Plugin>> {
        None
    }
}

impl Default for FormatterPlugin {
    fn default() -> Self {
        Self::new()
    }
}
