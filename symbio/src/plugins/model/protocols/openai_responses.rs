//! OpenAI Responses (Beta) 协议实现

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use serde_json::{json, Value};
use std::sync::Arc;

use super::{
    spawn_orchestrator, CapabilityMeta, ContentPart, MessageContent, MessageRole, ModelConfig,
    ModelProtocol, ProtocolEvent,
};
use crate::symbio_core::{
    InvokeRequest, InvokeResponse, Plugin, PluginPayload, MODEL_PROTOCOL_OPENAI_RESPONSES,
};
use tracing::debug;

pub struct OpenaiResponsesProtocol;

#[async_trait]
impl ModelProtocol for OpenaiResponsesProtocol {
    fn get_api_url(&self, config: &ModelConfig) -> String {
        format!("{}/responses", config.api_base)
    }

    fn get_headers(&self, config: &ModelConfig) -> HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        if let Ok(v) = "application/json".parse() {
            h.insert("Content-Type", v);
        }
        if let Ok(v) = "realtime=v1".parse() {
            h.insert("OpenAI-Beta", v);
        }
        if let Some(k) = &config.api_key {
            if let Ok(v) = format!("Bearer {k}").parse() {
                h.insert("Authorization", v);
            } else {
                crate::plugin_warn!("model", "Invalid characters in OpenAI API key");
            }
        }
        h
    }

    fn prepare_request(
        &self,
        config: &ModelConfig,
        system: &str,
        messages: &[crate::symbio_core::schemas::session::chat_message::ChatMessage],
        tools: &[CapabilityMeta],
    ) -> Value {
        let mut input_items = Vec::new();

        // 从消息历史中获取上一次响应的 ID 并确定需要处理的消息范围
        // 有状态模式：查找最后一个有 response_id 的 Assistant 消息
        let (previous_response_id, messages_to_process) = messages
            .iter()
            .enumerate()
            .rfind(|(_, m)| m.role == Some(MessageRole::Assistant) && m.response_id.is_some())
            .map(|(idx, m)| {
                // 找到了有 response_id 的 Assistant 消息，只发送增量消息
                (m.response_id.clone(), messages[idx + 1..].to_vec())
            })
            .unwrap_or_else(|| {
                // 没有找到（首次请求），发送全部消息
                (None, messages.to_vec())
            });

        let flattened_messages =
            crate::plugins::model::message_builder::flatten_chat_messages(&messages_to_process);

        for m in flattened_messages {
            if m.role == MessageRole::System {
                continue;
            }

            match m.role {
                MessageRole::User => {
                    let text = match m.content {
                        Some(MessageContent::Text(ref t)) => t.clone(),
                        Some(MessageContent::Parts(ref p)) => p
                            .iter()
                            .filter_map(|part| {
                                if let ContentPart::Text { text } = part {
                                    Some(text.clone())
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n"),
                        _ => "".into(),
                    };
                    if !text.is_empty() {
                        input_items.push(json!({
                            "type": "message",
                            "role": "user",
                            "content": [{"type": "input_text", "text": text}]
                        }));
                    }
                },
                MessageRole::Assistant => {
                    // 1. 推送合并后的文本项（只包含实际输出内容，reasoning 是内部过程不回传）
                    let mut full_text = String::new();
                    if let Some(ref content) = m.content {
                        match content {
                            MessageContent::Text(t) => full_text.push_str(t),
                            MessageContent::Parts(p) => {
                                for part in p {
                                    if let ContentPart::Text { text } = part {
                                        full_text.push_str(text);
                                    }
                                }
                            },
                        }
                    }

                    if !full_text.is_empty() {
                        input_items.push(json!({
                            "type": "message",
                            "role": "assistant",
                            "content": [{"type": "output_text", "text": full_text}]
                        }));
                    }

                    // 2. 推送工具调用项 (移除 status 字段以提高兼容性)
                    if let Some(ref tcs) = m.tool_calls {
                        for tc in tcs {
                            input_items.push(json!({
                                "type": "function_call",
                                "call_id": tc.id.as_ref().cloned().unwrap_or_default(),
                                "name": tc.name.replace("/", "__"),
                                "arguments": if tc.arguments.is_string() {
                                    tc.arguments.as_str().unwrap_or("{}").to_string()
                                } else {
                                    tc.arguments.to_string()
                                }
                            }));
                        }
                    }
                },
                MessageRole::Tool => {
                    // 推送工具结果项
                    if let Some(ref call_id) = m.tool_call_id {
                        if !call_id.is_empty() {
                            let text = match m.content {
                                Some(MessageContent::Text(ref t)) => t.clone(),
                                Some(MessageContent::Parts(ref p)) => p
                                    .iter()
                                    .filter_map(|part| {
                                        if let ContentPart::Text { text } = part {
                                            Some(text.clone())
                                        } else {
                                            None
                                        }
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\n"),
                                _ => "".into(),
                            };
                            input_items.push(json!({
                                "type": "function_call_output",
                                "call_id": call_id,
                                "output": text
                            }));
                        }
                    }
                },
                _ => {},
            }
        }

        let mut req = json!({
            "model": config.model,
            "input": input_items,
            "instructions": system,
            "temperature": config.temperature,
            "stream": true,
            "store": config.store,
        });

        if let Some(ref pid) = previous_response_id {
            req["previous_response_id"] = json!(pid);
        }
        req["max_output_tokens"] = json!(config.max_tokens.unwrap_or(8192));
        if !tools.is_empty() {
            req["tools"] = json!(tools
                .iter()
                .map(|t| json!({
                    "type": "function",
                    "name": t.name.replace("/", "__"),
                    "description": t.description_for_llm(),
                    "parameters": t.input_schema
                }))
                .collect::<Vec<_>>());
            req["tool_choice"] = json!("auto");
        }

        debug!(
            "OpenAI Responses request: {}",
            serde_json::to_string(&req).unwrap_or_default()
        );
        req
    }

    fn parse_response_line(&self, line: &str) -> Vec<ProtocolEvent> {
        let mut evs = Vec::new();
        if !line.starts_with("data: ") {
            return evs;
        }
        let data = &line[6..];
        if data == "[DONE]" {
            return evs;
        }

        if let Ok(json) = serde_json::from_str::<Value>(data) {
            // 捕捉 Response ID
            if let Some(id) = json
                .get("response")
                .and_then(|r| r.get("id"))
                .and_then(|v| v.as_str())
            {
                evs.push(ProtocolEvent::ResponseId(id.to_string()));
            }

            match json.get("type").and_then(|t| t.as_str()).unwrap_or("") {
                // 文本增量 (兼容多种 delta 命名)
                "response.text.delta" | "response.output_text.delta" => {
                    if let Some(d) = json.get("delta").and_then(|v| v.as_str()) {
                        evs.push(ProtocolEvent::ContentDelta(d.to_string()));
                    }
                },
                // 推理增量
                "response.reasoning_text.delta" => {
                    if let Some(d) = json.get("delta").and_then(|v| v.as_str()) {
                        evs.push(ProtocolEvent::ReasoningDelta(d.to_string()));
                    }
                },
                // 工具调用增量参数
                "response.function_call_arguments.delta" => {
                    let idx = json
                        .get("output_index")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as usize;
                    let id = json
                        .get("call_id")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    let args = json
                        .get("delta")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    evs.push(ProtocolEvent::ToolCallDelta(idx, id, None, args));
                },
                // 工具调用完成 (有些模型直接在这里返回完整参数)
                "response.function_call_arguments.done" => {
                    let idx = json
                        .get("output_index")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as usize;
                    let args = json
                        .get("arguments")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    evs.push(ProtocolEvent::ToolCallDelta(idx, None, None, args));
                },
                // 项目添加 (用于提取工具名称和 ID)
                "response.output_item.added" => {
                    if let Some(item) = json.get("item") {
                        if item["type"] == "function_call" {
                            let idx = json
                                .get("output_index")
                                .and_then(|v| v.as_u64())
                                .unwrap_or(0) as usize;
                            let id = item
                                .get("call_id")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            let name = item
                                .get("name")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            evs.push(ProtocolEvent::ToolCallDelta(idx, id, name, None));
                        }
                    }
                },
                "error" => {
                    if let Some(err) = json
                        .get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|v| v.as_str())
                    {
                        evs.push(ProtocolEvent::Error(err.to_string()));
                    }
                },
                _ => {},
            }
        }
        evs
    }

    async fn handle_chat_stream(
        &self,
        config: &ModelConfig,
        parent: &Option<Arc<dyn Plugin>>,
        ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        spawn_orchestrator(Box::new(OpenaiResponsesProtocol), config, parent, ctx).await
    }
}

// === 注册到通用对象创建机制 ===

fn build(_ctx: Arc<dyn InvokeRequest>) -> Arc<dyn ModelProtocol> {
    Arc::new(OpenaiResponsesProtocol)
}

crate::submit_object_creator!(MODEL_PROTOCOL_OPENAI_RESPONSES, build, dyn ModelProtocol);
