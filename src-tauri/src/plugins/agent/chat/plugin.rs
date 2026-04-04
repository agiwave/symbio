//! AI 对话插件实现
//!
//! 职责：
//! - 前端对话接口
//! - 通过 @llm 能力路由调用实际的 LLM 插件 (openai)
//!
//! 注意：配置管理由 openai 插件负责，前端应直接调用 agent/@llm 或 agent/openai

use crate::core::traits::{Plugin, CAPABILITY_LLM};
use crate::core::types::{PluginMeta, PluginResult, InvokeStream, StreamChunk};
use serde_json::{Value, json};
use std::sync::{Arc, Weak};

/// AI 对话插件
pub struct ChatPlugin {
    meta: PluginMeta,
    /// 父插件引用（用于能力路由）
    parent: Option<Weak<dyn Plugin>>,
}

impl ChatPlugin {
    fn create_meta() -> PluginMeta {
        PluginMeta {
            name: "chat".to_string(),
            description: "AI 对话插件 - 通过 @llm 调用 LLM".to_string(),
            version: "0.1.0".to_string(),
            input: Some(json!({
                "type": "object",
                "properties": {
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
        }
    }

    /// 主构造函数（Factory 机制使用）
    pub fn new(parent: Option<Weak<dyn Plugin>>) -> Self {
        Self {
            meta: Self::create_meta(),
            parent,
        }
    }

    /// 获取父插件引用
    fn get_parent(&self) -> Option<Arc<dyn Plugin>> {
        self.parent.as_ref().and_then(|w| w.upgrade())
    }
}

impl Default for ChatPlugin {
    fn default() -> Self {
        Self::new(None)
    }
}

#[async_trait::async_trait]
impl Plugin for ChatPlugin {
    fn meta(&self, _path: &str) -> PluginResult<PluginMeta> {
        Ok(self.meta.clone())
    }

    fn invoke(&self, _path: &str, input: Value) -> PluginResult<InvokeStream> {
        let parent = self.get_parent();
        let stream = async_stream::stream! {
            // 获取 session_id
            let session_id = input.get("session_id")
                .and_then(|v| v.as_str())
                .unwrap_or("default");

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
                            "message": msg,
                            "session_id": session_id
                        });

                        match p.invoke(&format!("@{}", CAPABILITY_LLM), llm_input) {
                            Ok(llm_stream) => {
                                // 真正流式：实时转发每个 chunk
                                use futures::StreamExt;
                                let mut stream = match llm_stream {
                                    InvokeStream::Single(chunk) => {
                                        // 如果是单次返回，直接 yield
                                        yield chunk;
                                        return;
                                    }
                                    InvokeStream::Stream(s) => s,
                                };
                                
                                while let Some(chunk) = stream.next().await {
                                    yield chunk;
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
        };

        Ok(InvokeStream::Stream(Box::pin(stream)))
    }
}