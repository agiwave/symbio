//! AI 对话插件实现
//!
//! 通过 @llm 能力路由调用实际的 LLM 插件 (openai)

use crate::core::traits::{Plugin, CAPABILITY_LLM};
use crate::core::types::{PluginMeta, PluginResult, PluginError, InvokeStream, StreamChunk};
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::{Arc, Weak};

/// AI 对话插件
/// 
/// 职责：
/// - 前端对话接口
/// - 通过 @llm 能力路由调用实际 LLM 插件
pub struct ChatPlugin {
    meta: PluginMeta,
    /// 父插件引用（用于能力路由）
    parent: Option<Weak<dyn Plugin>>,
}

impl ChatPlugin {
    pub fn new() -> Self {
        Self {
            meta: PluginMeta {
                name: "chat".to_string(),
                description: "AI 对话插件".to_string(),
                version: "0.1.0".to_string(),
                input: Some(json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["send", "configure", "get_config"],
                            "description": "操作类型"
                        },
                        "messages": {
                            "type": "array",
                            "items": {
                                "type": "object",
                                "properties": {
                                    "role": { "type": "string" },
                                    "content": { "type": "string" }
                                }
                            },
                            "description": "对话消息列表"
                        },
                        "config": {
                            "type": "object",
                            "properties": {
                                "api_base": { "type": "string" },
                                "api_key": { "type": "string" },
                                "model": { "type": "string" },
                                "temperature": { "type": "number" },
                                "max_tokens": { "type": "integer" }
                            },
                            "description": "AI 提供商配置"
                        }
                    },
                    "required": ["action"]
                })),
                output: Some(json!({
                    "type": "object",
                    "properties": {
                        "content": { "type": "string" },
                        "error": { "type": "string" }
                    }
                })),
                author: Some("Symbio Team".to_string()),
            },
            parent: None,
        }
    }

    /// 创建带父引用的实例
    pub fn with_parent(parent: Option<Arc<dyn Plugin>>) -> Arc<dyn Plugin> {
        let plugin = Self::new();
        let mut plugin = plugin;
        plugin.parent = parent.map(|p| Arc::downgrade(&p));
        Arc::new(plugin)
    }

    /// 获取父插件引用
    fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        self.parent.as_ref().and_then(|w| w.upgrade())
    }
}

impl Default for ChatPlugin {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl Plugin for ChatPlugin {
    fn meta(&self, _path: &str) -> PluginResult<PluginMeta> {
        Ok(self.meta.clone())
    }

    fn invoke(&self, _path: &str, input: Value) -> PluginResult<InvokeStream> {
        let action = input.get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 action 参数".to_string()))?
            .to_string();

        let parent = self.get_parent();

        let stream = async_stream::stream! {
            match action.as_str() {
                "send" => {
                    // 获取最后一条用户消息
                    let message = input.get("messages")
                        .and_then(|msgs| msgs.as_array())
                        .and_then(|arr| arr.last())
                        .and_then(|msg| msg.get("content"))
                        .and_then(|c| c.as_str());

                    match message {
                        Some(msg) => {
                            // 通过 @llm 能力路由调用 openai 插件
                            if let Some(ref p) = parent {
                                let llm_input = json!({
                                    "action": "chat",
                                    "message": msg
                                });
                                
                                match p.invoke(&format!("@{}", CAPABILITY_LLM), llm_input) {
                                    Ok(stream) => {
                                        use futures::StreamExt;
                                        let chunks = stream.collect().await;
                                        if let Some(chunk) = chunks.into_iter().next() {
                                            if let Some(error) = chunk.error {
                                                yield StreamChunk {
                                                    data: json!({}),
                                                    done: true,
                                                    error: Some(error),
                                                };
                                            } else if let Some(content) = chunk.data.get("content").and_then(|c| c.as_str()) {
                                                yield StreamChunk {
                                                    data: json!({ "content": content }),
                                                    done: true,
                                                    error: None,
                                                };
                                            } else {
                                                yield StreamChunk {
                                                    data: json!({}),
                                                    done: true,
                                                    error: Some("LLM 响应格式错误".to_string()),
                                                };
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        yield StreamChunk {
                                            data: json!({}),
                                            done: true,
                                            error: Some(format!("调用 LLM 失败: {}", e)),
                                        };
                                    }
                                }
                            } else {
                                yield StreamChunk {
                                    data: json!({}),
                                    done: true,
                                    error: Some("父插件未设置".to_string()),
                                };
                            }
                        }
                        None => {
                            yield StreamChunk {
                                data: json!({}),
                                done: true,
                                error: Some("缺少消息内容".to_string()),
                            };
                        }
                    }
                }
                "configure" => {
                    // 配置 openai 插件
                    if let Some(ref p) = parent {
                        if let Some(config) = input.get("config") {
                            let config_input = json!({
                                "action": "configure",
                                "api_base": config.get("api_base"),
                                "api_key": config.get("api_key"),
                                "model": config.get("model"),
                                "temperature": config.get("temperature"),
                                "max_tokens": config.get("max_tokens"),
                            });
                            
                            match p.invoke("openai", config_input) {
                                Ok(stream) => {
                                    use futures::StreamExt;
                                    let chunks = stream.collect().await;
                                    if let Some(chunk) = chunks.into_iter().next() {
                                        yield StreamChunk {
                                            data: json!({ "message": "配置已更新" }),
                                            done: true,
                                            error: chunk.error,
                                        };
                                    }
                                }
                                Err(e) => {
                                    yield StreamChunk {
                                        data: json!({}),
                                        done: true,
                                        error: Some(format!("配置失败: {}", e)),
                                    };
                                }
                            }
                        } else {
                            yield StreamChunk {
                                data: json!({}),
                                done: true,
                                error: Some("缺少 config 参数".to_string()),
                            };
                        }
                    } else {
                        yield StreamChunk {
                            data: json!({}),
                            done: true,
                            error: Some("父插件未设置".to_string()),
                        };
                    }
                }
                "get_config" => {
                    // 获取 openai 插件配置
                    if let Some(ref p) = parent {
                        let config_input = json!({ "action": "get_config" });
                        
                        match p.invoke("openai", config_input) {
                            Ok(stream) => {
                                use futures::StreamExt;
                                let chunks = stream.collect().await;
                                if let Some(chunk) = chunks.into_iter().next() {
                                    let data = chunk.data.clone();
                                    yield StreamChunk {
                                        data: json!({
                                            "name": "openai",
                                            "api_base": data.get("api_base").unwrap_or(&json!("")),
                                            "model": data.get("model").unwrap_or(&json!("")),
                                            "has_api_key": data.get("api_key_set").unwrap_or(&json!(false)),
                                        }),
                                        done: true,
                                        error: chunk.error,
                                    };
                                }
                            }
                            Err(e) => {
                                yield StreamChunk {
                                    data: json!({}),
                                    done: true,
                                    error: Some(format!("获取配置失败: {}", e)),
                                };
                            }
                        }
                    } else {
                        yield StreamChunk {
                            data: json!({}),
                            done: true,
                            error: Some("父插件未设置".to_string()),
                        };
                    }
                }
                _ => {
                    yield StreamChunk {
                        data: json!({}),
                        done: true,
                        error: Some(format!("未知操作: {}", action)),
                    };
                }
            }
        };

        Ok(InvokeStream::Stream(Box::pin(stream)))
    }
}
