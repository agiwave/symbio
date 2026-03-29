//! Formatter Plugin

use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginResult, PluginError, InvokeStream, StreamChunk};
use serde_json::{Value, json};
use futures::stream;

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
            _ => return Err(PluginError::ValidationError(format!("未知的格式化类型：{}", format))),
        };
        
        // 演示流式返回：逐字符输出
        let chars: Vec<char> = formatted.chars().collect();
        let len = chars.len();
        
        let chunks: Vec<StreamChunk> = chars
            .into_iter()
            .enumerate()
            .map(|(i, ch)| StreamChunk {
                data: Value::String(ch.to_string()),
                done: i == len - 1,
                error: None,
            })
            .collect();
        
        Ok(InvokeStream::stream(stream::iter(chunks)))
    }
}

impl Default for FormatterPlugin {
    fn default() -> Self {
        Self::new()
    }
}
