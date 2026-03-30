//! AI 对话插件实现
//!
//! 通过 @llm 能力路由调用实际的 LLM 插件

use crate::core::traits::Plugin;
use crate::core::types::{PluginMeta, PluginResult, PluginError, InvokeStream, StreamChunk};
use super::types::*;
use super::client::LlmClient;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::sync::{Arc, Weak};
use tokio::sync::RwLock;
use futures::stream::StreamExt;

/// AI 对话插件
/// 
/// 职责：
/// - 管理 API 配置
/// - 通过 @llm 能力路由调用实际 LLM 插件
pub struct ChatPlugin {
    meta: PluginMeta,
    client: Arc<RwLock<Option<LlmClient>>>,
    config: Arc<RwLock<ProviderConfig>>,
    /// 父插件引用（用于能力路由）
    parent: Option<Weak<dyn Plugin>>,
}

impl ChatPlugin {
    pub fn new() -> Self {
        Self {
            meta: PluginMeta {
                name: "chat".to_string(),
                description: "AI 对话插件，支持流式响应".to_string(),
                version: "0.1.0".to_string(),
                input: Some(json!({
                    "type": "object",
                    "properties": {
                        "action": {
                            "type": "string",
                            "enum": ["send", "stream", "configure", "get_config"],
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
                        "event_type": { "type": "string" },
                        "config": { "type": "object" }
                    }
                })),
                author: Some("Symbio Team".to_string()),
            },
            client: Arc::new(RwLock::new(None)),
            config: Arc::new(RwLock::new(ProviderConfig::default())),
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

#[async_trait]
impl Plugin for ChatPlugin {
    fn meta(&self, _path: &str) -> PluginResult<PluginMeta> {
        Ok(self.meta.clone())
    }

    fn invoke(&self, _path: &str, input: Value) -> PluginResult<InvokeStream> {
        let action = input.get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::ValidationError("缺少 action 参数".to_string()))?
            .to_string();

        let client = Arc::clone(&self.client);
        let config = Arc::clone(&self.config);

        let stream = async_stream::stream! {
            match action.as_str() {
                "send" => {
                    let messages_value = input.get("messages");
                    
                    match messages_value {
                        Some(msgs) => {
                            let messages: Result<Vec<ChatMessage>, _> = serde_json::from_value(msgs.clone());
                            match messages {
                                Ok(msgs) => {
                                    let client_guard = client.read().await;
                                    match client_guard.as_ref() {
                                        Some(c) => match c.chat(msgs).await {
                                            Ok(content) => {
                                                yield StreamChunk {
                                                    data: json!({ "content": content }),
                                                    done: true,
                                                    error: None,
                                                };
                                            }
                                            Err(e) => {
                                                yield StreamChunk {
                                                    data: json!({}),
                                                    done: true,
                                                    error: Some(e.to_string()),
                                                };
                                            }
                                        },
                                        None => {
                                            yield StreamChunk {
                                                data: json!({}),
                                                done: true,
                                                error: Some("AI 客户端未初始化，请先配置 API Key".to_string()),
                                            };
                                        }
                                    }
                                }
                                Err(e) => {
                                    yield StreamChunk {
                                        data: json!({}),
                                        done: true,
                                        error: Some(format!("消息格式错误: {}", e)),
                                    };
                                }
                            }
                        }
                        None => {
                            yield StreamChunk {
                                data: json!({}),
                                done: true,
                                error: Some("缺少 messages 参数".to_string()),
                            };
                        }
                    }
                }
                "configure" => {
                    let config_value = input.get("config");
                    
                    match config_value {
                        Some(cfg) => {
                            let new_config: Result<ProviderConfig, _> = serde_json::from_value(cfg.clone());
                            match new_config {
                                Ok(new_config) => {
                                    // 更新配置
                                    {
                                        let mut config_guard = config.write().await;
                                        *config_guard = new_config.clone();
                                    }

                                    // 重新创建客户端
                                    {
                                        let mut client_guard = client.write().await;
                                        *client_guard = Some(LlmClient::new(new_config.clone()));
                                    }

                                    yield StreamChunk {
                                        data: json!({
                                            "message": "配置已更新",
                                            "config": {
                                                "name": new_config.name,
                                                "api_base": new_config.api_base,
                                                "model": new_config.model,
                                            }
                                        }),
                                        done: true,
                                        error: None,
                                    };
                                }
                                Err(e) => {
                                    yield StreamChunk {
                                        data: json!({}),
                                        done: true,
                                        error: Some(format!("配置格式错误: {}", e)),
                                    };
                                }
                            }
                        }
                        None => {
                            yield StreamChunk {
                                data: json!({}),
                                done: true,
                                error: Some("缺少 config 参数".to_string()),
                            };
                        }
                    }
                }
                "get_config" => {
                    let config_guard = config.read().await;
                    yield StreamChunk {
                        data: json!({
                            "name": config_guard.name,
                            "api_base": config_guard.api_base,
                            "model": config_guard.model,
                            "temperature": config_guard.temperature,
                            "max_tokens": config_guard.max_tokens,
                            "has_api_key": !config_guard.api_key.is_empty(),
                        }),
                        done: true,
                        error: None,
                    };
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