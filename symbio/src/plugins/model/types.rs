//! MODEL Plugin - Unified type definitions

use serde::{Deserialize, Serialize};

pub use crate::symbio_core::schemas::model::model_config::ModelConfig;

pub use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, ContentPart, MessageContent, MessageRole,
};
pub use crate::symbio_core::{CapabilityMeta, ToolCall};

/// 原生消息格式（支持 tool calls 和多模态）
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NativeMessage {
    #[serde(default)]
    pub id: String,
    pub role: MessageRole,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<MessageContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default)]
    pub timestamp: i64,
    /// tool 调用的成功标志：None 视为成功；Some(false) 触发 Anthropic tool_result 的 `is_error=true`
    ///
    /// 调用方：`anthropic_messages::prepare_request` 在 tool 消息块根据此字段决定是否设置 `is_error`。
    #[serde(skip_serializing)]
    pub success: Option<bool>,
}

impl From<NativeMessage> for ChatMessage {
    fn from(msg: NativeMessage) -> Self {
        Self {
            id: if msg.id.is_empty() {
                uuid::Uuid::new_v4().to_string()
            } else {
                msg.id
            },
            parent_id: None,
            role: Some(msg.role),
            msg_type: None,

            name: None,
            content: msg.content,
            status: Some(
                crate::symbio_core::schemas::session::chat_message::MessageStatus::Completed,
            ),
            meta: None,
            timestamp: Some(msg.timestamp),
            response_id: msg.response_id,
            prompt: msg.prompt,
            error: None,
        }
    }
}

impl From<ChatMessage> for NativeMessage {
    fn from(msg: ChatMessage) -> Self {
        let role = msg.role.clone().unwrap_or(MessageRole::User);
        // For tool result (Tool) messages, parent_id is the tool_call_id
        let tool_call_id = if role == MessageRole::Tool {
            msg.parent_id.clone()
        } else {
            None
        };
        Self {
            id: msg.id,
            role,
            content: msg.content.clone(),
            tool_call_id,
            tool_calls: None,
            reasoning_content: None,
            response_id: None,
            prompt: msg.prompt,
            timestamp: msg.timestamp.unwrap_or(0),
            success: None,
        }
    }
}

impl NativeMessage {
    /// 转换为 API 请求格式
    pub fn to_api_value(&self) -> serde_json::Value {
        let mut obj = serde_json::Map::new();
        let role_str = match self.role {
            MessageRole::Tool => "tool",
            MessageRole::User => "user",
            MessageRole::Assistant => "assistant",
            MessageRole::System => "system",
        };
        obj.insert("role".to_string(), serde_json::json!(role_str));

        // 正确处理 content：如果是 Text，直接使用字符串；如果是 Parts，转换为对象数组
        let content_value = match self.content {
            Some(ref content) => match content {
                MessageContent::Text(text) => {
                    let final_text = if let Some(ref prompt) = self.prompt {
                        format!("{}\n\n{}", prompt, text)
                    } else {
                        text.clone()
                    };
                    serde_json::json!(final_text)
                },
                MessageContent::Parts(parts) => {
                    let mut converted_parts: Vec<serde_json::Value> = parts
                        .iter()
                        .map(|part| match part {
                            ContentPart::Text { text } => serde_json::json!(text),
                            ContentPart::ImageUrl { image_url } => serde_json::json!({
                                "type": "image_url",
                                "image_url": image_url
                            }),
                        })
                        .collect();

                    if let Some(ref prompt) = self.prompt {
                        converted_parts.insert(
                            0,
                            serde_json::json!({
                                "type": "text",
                                "text": prompt
                            }),
                        );
                    }

                    serde_json::json!(converted_parts)
                },
            },
            None => {
                if let Some(ref prompt) = self.prompt {
                    serde_json::json!([{
                        "type": "text",
                        "text": prompt
                    }])
                } else {
                    serde_json::Value::Null
                }
            },
        };
        obj.insert("content".to_string(), content_value);

        if let Some(ref reasoning) = self.reasoning_content {
            obj.insert(
                "reasoning_content".to_string(),
                serde_json::json!(reasoning),
            );
        }

        if let Some(ref tool_call_id) = self.tool_call_id {
            obj.insert("tool_call_id".to_string(), serde_json::json!(tool_call_id));
        }

        if let Some(ref tool_calls) = self.tool_calls {
            // 转换工具调用格式，将工具名称中的 / 替换为 __
            let api_tool_calls: Vec<serde_json::Value> = tool_calls
                .iter()
                .map(|tc| {
                    // arguments 需要转换为字符串，符合 OpenAI API 要求
                    let args_str = if tc.arguments.is_string() {
                        tc.arguments.as_str().unwrap_or("{}").to_string()
                    } else {
                        tc.arguments.to_string()
                    };
                    serde_json::json!({
                        "id": tc.id,
                        "type": tc.kind.as_ref().unwrap_or(&"function".to_string()),
                        "function": {
                            "name": tc.name.replace("/", "__"),
                            "arguments": args_str
                        }
                    })
                })
                .collect();
            obj.insert("tool_calls".to_string(), serde_json::json!(api_tool_calls));
        }

        serde_json::Value::Object(obj)
    }

    /// 转换为 Responses API 格式
    pub fn to_responses_api_value(&self) -> serde_json::Value {
        // 如果是工具输出
        if self.role == MessageRole::Tool {
            let mut item = serde_json::Map::new();
            item.insert(
                "type".to_string(),
                serde_json::json!("function_call_output"),
            );
            item.insert(
                "call_id".to_string(),
                serde_json::json!(self.tool_call_id.as_ref().cloned().unwrap_or_default()),
            );

            let output = match self.content {
                Some(ref content) => match content {
                    MessageContent::Text(text) => text.clone(),
                    MessageContent::Parts(parts) => parts
                        .iter()
                        .filter_map(|p| match p {
                            ContentPart::Text { text } => Some(text.clone()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n"),
                },
                None => String::new(),
            };
            item.insert("output".to_string(), serde_json::json!(output));
            return serde_json::Value::Object(item);
        }

        // 如果包含工具调用（Assistant 发出的）
        if let Some(ref tool_calls) = self.tool_calls {
            if !tool_calls.is_empty() {
                // Responses API 这里的结构比较特殊，通常一个 function_call 是一个独立的项
                // 如果有多个工具调用，可能需要拆分或者使用特定的结构。
                // 按照当前 Beta 规范，function_call 项是独立的。
                let tc = &tool_calls[0]; // 简化处理：取第一个
                let mut item = serde_json::Map::new();
                item.insert("type".to_string(), serde_json::json!("function_call"));
                item.insert("status".to_string(), serde_json::json!("completed"));
                item.insert(
                    "name".to_string(),
                    serde_json::json!(tc.name.replace("/", "__")),
                );
                item.insert(
                    "call_id".to_string(),
                    serde_json::json!(tc.id.as_ref().cloned().unwrap_or_default()),
                );

                let args_str = if tc.arguments.is_string() {
                    tc.arguments.as_str().unwrap_or("{}").to_string()
                } else {
                    tc.arguments.to_string()
                };
                item.insert("arguments".to_string(), serde_json::json!(args_str));
                return serde_json::Value::Object(item);
            }
        }

        // 普通消息
        let mut item = serde_json::Map::new();
        item.insert("type".to_string(), serde_json::json!("message"));
        item.insert("role".to_string(), serde_json::json!(self.role));

        if let Some(ref content) = self.content {
            match content {
                MessageContent::Text(text) => {
                    let mut final_text = text.clone();
                    if let Some(ref p) = self.prompt {
                        final_text = format!("{}\n\n{}", p, final_text);
                    }
                    let content_part = if self.role == MessageRole::User {
                        serde_json::json!([{"type": "input_text", "text": final_text}])
                    } else {
                        serde_json::json!([{"type": "text", "text": final_text}])
                    };
                    item.insert("content".to_string(), content_part);
                },
                MessageContent::Parts(parts) => {
                    let mut converted_parts: Vec<serde_json::Value> = parts
                        .iter()
                        .map(|part| match part {
                            ContentPart::Text { text } => {
                                if self.role == MessageRole::User {
                                    serde_json::json!({"type": "input_text", "text": text})
                                } else {
                                    serde_json::json!({"type": "text", "text": text})
                                }
                            },
                            ContentPart::ImageUrl { image_url } => {
                                serde_json::json!({"type": "input_image", "image_url": image_url})
                            },
                        })
                        .collect();

                    if let Some(ref p) = self.prompt {
                        if self.role == MessageRole::User {
                            converted_parts
                                .insert(0, serde_json::json!({"type": "input_text", "text": p}));
                        } else {
                            converted_parts
                                .insert(0, serde_json::json!({"type": "text", "text": p}));
                        }
                    }

                    item.insert("content".to_string(), serde_json::json!(converted_parts));
                },
            }
        }

        serde_json::Value::Object(item)
    }
}
