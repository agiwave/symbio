//! Anthropic 协议处理

use async_trait::async_trait;
// use async_trait::async_trait;

use reqwest::header::HeaderMap;
use serde_json::{json, Value};
use std::sync::Arc;
use std::sync::Mutex;

use super::super::context::get_http_client;
use super::super::types::{CapabilityMeta, ContentPart, MessageContent, MessageRole, ModelConfig};
use super::{spawn_orchestrator, ModelProtocol, ProtocolEvent};
use crate::symbio_core::{
    InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginError, PluginPayload,
    MODEL_PROTOCOL_ANTHROPIC_MESSAGES,
};
use tracing::warn;

pub struct AnthropicProtocol {
    // SAFETY (S-002 审计): 此 Mutex 仅在同步 trait 方法 `parse_response_line` 内部使用，
    // 该方法由 SSE 解析器在 reqwest_eventsource 的同步迭代上下文中调用（无 .await），
    // 因此 std::sync::Mutex 不会阻塞 tokio worker。若未来要把 parse 改成 async，
    // 必须同时把此 Mutex 换成 tokio::sync::Mutex 并配套 .lock().await。
    current_event_type: Mutex<Option<String>>,
}

impl AnthropicProtocol {
    pub fn new() -> Self {
        Self {
            current_event_type: Mutex::new(None),
        }
    }
}

impl Default for AnthropicProtocol {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ModelProtocol for AnthropicProtocol {
    fn get_api_url(&self, config: &ModelConfig) -> String {
        format!("{}/messages", config.api_base)
    }

    fn get_headers(&self, config: &ModelConfig) -> HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        if let Ok(v) = "application/json".parse() {
            h.insert("Content-Type", v);
        }
        if let Ok(v) = "2023-06-01".parse() {
            h.insert("anthropic-version", v);
        }
        if let Some(k) = &config.api_key {
            if let Ok(v) = k.parse() {
                h.insert("x-api-key", v);
            } else {
                crate::plugin_warn!("model", "Invalid characters in Anthropic API key");
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
        let flattened_messages =
            crate::plugins::model::message_builder::flatten_chat_messages(messages);
        let mut anthropic_msgs: Vec<Value> = Vec::new();
        for m in &flattened_messages {
            if m.role == MessageRole::System {
                continue;
            }
            let mut parts = Vec::new();
            if m.role != MessageRole::Tool {
                if let Some(ref content) = m.content {
                    match content {
                        MessageContent::Text(t) => {
                            if !t.is_empty() {
                                parts.push(json!({"type": "text", "text": t}));
                            }
                        }
                        MessageContent::Parts(p) => {
                            for part in p {
                                match part {
                                    ContentPart::Text { text } => {
                                        parts.push(json!({"type": "text", "text": text}));
                                    }
                                    ContentPart::ImageUrl { image_url } => {
                                        // Anthropic expects: { "type": "image", "source": { "type": "base64", "media_type": "image/jpeg", "data": "..." } }
                                        // We assume the URL is already a data URI or base64 string
                                        let (media_type, base64_data) = if image_url
                                            .url
                                            .starts_with("data:")
                                        {
                                            let parts: Vec<&str> =
                                                image_url.url.split(',').collect();
                                            if parts.len() == 2 {
                                                let meta = parts[0];
                                                let data = parts[1];
                                                let media_type = meta
                                                    .strip_prefix("data:")
                                                    .and_then(|s| s.split(';').next())
                                                    .unwrap_or("image/jpeg");
                                                (media_type.to_string(), data.to_string())
                                            } else {
                                                ("image/jpeg".to_string(), image_url.url.clone())
                                            }
                                        } else {
                                            ("image/jpeg".to_string(), image_url.url.clone())
                                        };

                                        parts.push(json!({
                                            "type": "image",
                                            "source": {
                                                "type": "base64",
                                                "media_type": media_type,
                                                "data": base64_data
                                            }
                                        }));
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if let Some(ref tcs) = m.tool_calls {
                for tc in tcs {
                    let args = if tc.arguments.is_string() {
                        serde_json::from_str(tc.arguments.as_str().unwrap_or_default())
                            .unwrap_or(json!({}))
                    } else {
                        tc.arguments.clone()
                    };
                    parts.push(json!({
                        "type": "tool_use",
                        "id": tc.id,
                        "name": tc.name.replace("/", "__"),
                        "input": args
                    }));
                }
            }
            if m.role == MessageRole::Tool {
                let text = match m.content {
                    Some(MessageContent::Text(ref t)) => t.clone(),
                    _ => "{}".into(),
                };
                let mut res_obj = json!({
                    "type": "tool_result",
                    "tool_use_id": m.tool_call_id,
                    "content": text
                });
                if let Some(false) = m.success {
                    res_obj["is_error"] = json!(true);
                }
                parts.push(res_obj);
            }
            // 处理推理内容 (Reasoning / Thinking)
            if m.role == MessageRole::Assistant {
                if let Some(ref reasoning) = m.reasoning_content {
                    if !reasoning.is_empty() {
                        // 如果模型看起来是 Claude 3.7+，尝试使用 thinking 块
                        // 注意：官方 API 要求思考块必须有 signature，
                        // 但对于许多中转或 LMStudio，可能不需要或支持纯文本形式。
                        // 为了最大兼容性，我们暂时使用带标签的文本块，或者如果以后有了 signature 则使用 thinking 块。
                        parts.insert(
                            0,
                            json!({
                                "type": "text",
                                "text": format!("<thought>\n{}\n</thought>", reasoning)
                            }),
                        );
                    }
                }
            }

            if parts.is_empty() {
                continue;
            }

            let role = if m.role == MessageRole::Assistant {
                "assistant"
            } else {
                "user"
            };

            // 合并连续相同角色的消息
            if let Some(last) = anthropic_msgs.last_mut() {
                if last["role"] == role {
                    if let Some(arr) = last["content"].as_array_mut() {
                        // Anthropic 规定 tool_result 必须在 user 消息内容的最前面
                        if role == "user" {
                            let mut tool_results = Vec::new();
                            let mut others = Vec::new();
                            for p in parts {
                                if p["type"] == "tool_result" {
                                    tool_results.push(p);
                                } else {
                                    others.push(p);
                                }
                            }

                            // 寻找现有内容中第一个非 tool_result 的位置
                            let first_non_tool = arr
                                .iter()
                                .position(|p| p["type"] != "tool_result")
                                .unwrap_or(arr.len());

                            // 插入新的 tool_results 到该位置（即所有已有 tool_results 之后，text 之前）
                            for (i, tr) in tool_results.into_iter().enumerate() {
                                arr.insert(first_non_tool + i, tr);
                            }
                            // 其余内容追加到最后
                            arr.extend(others);
                        } else {
                            arr.extend(parts);
                        }
                    }
                } else {
                    anthropic_msgs.push(json!({"role": role, "content": parts}));
                }
            } else {
                anthropic_msgs.push(json!({"role": role, "content": parts}));
            }
        }

        let mut req = json!({
            "model": config.model,
            "system": system,
            "messages": anthropic_msgs,
            "temperature": config.temperature,
            "stream": true
        });

        // 处理 Anthropic Thinking (Claude 3.7+)
        if config.reasoning.is_some() {
            let budget = (config.max_tokens.unwrap_or(4096) / 2).max(1024);
            req["thinking"] = json!({
                "type": "enabled",
                "budget_tokens": budget
            });
            // Anthropic 规定开启 thinking 时 temperature 必须为 1.0
            req["temperature"] = json!(1.0);

            // 确保 max_tokens 大于 budget
            if config.max_tokens.unwrap_or(8192) <= budget {
                req["max_tokens"] = json!(budget + 1024);
            }
        }

        if let Some(m) = config.max_tokens {
            req["max_tokens"] = json!(m);
        } else if req.get("max_tokens").is_none() {
            req["max_tokens"] = json!(8192);
        }
        if !tools.is_empty() {
            req["tools"] = json!(tools
                .iter()
                .map(|t| json!({
                    "name": t.name.replace("/", "__"),
                    "description": t.description_for_llm(),
                    "input_schema": t.input_schema
                }))
                .collect::<Vec<_>>());
        }
        req
    }

    fn parse_response_line(&self, line: &str) -> Vec<ProtocolEvent> {
        let mut evs = Vec::new();
        if let Some(stripped) = line.strip_prefix("event: ") {
            let mut etype = self.current_event_type.lock().unwrap();
            let type_val = stripped.trim().to_string();
            *etype = Some(type_val);
            return evs;
        }

        if !line.starts_with("data: ") {
            return evs;
        }

        let data = &line[6..];
        if let Ok(json) = serde_json::from_str::<Value>(data.trim()) {
            let etype_raw = self.current_event_type.lock().unwrap();
            let mut etype = etype_raw.as_deref().unwrap_or("").to_string();

            // 兼容性逻辑：如果 SSE event 为空，尝试从 JSON 的 type 字段获取
            if etype.is_empty() {
                if let Some(t) = json.get("type").and_then(|v| v.as_str()) {
                    etype = t.to_string();
                }
            }

            match etype.as_str() {
                "content_block_start" => {
                    if let Some(block) = json.get("content_block") {
                        if block["type"] == "tool_use" {
                            evs.push(ProtocolEvent::ToolCallDelta(
                                json["index"].as_u64().unwrap_or(0) as usize,
                                block.get("id").and_then(|v| v.as_str()).map(|s| s.into()),
                                block.get("name").and_then(|v| v.as_str()).map(|s| s.into()),
                                None,
                            ));
                        }
                    }
                }
                "content_block_delta" => {
                    if let Some(delta) = json.get("delta") {
                        let idx = json["index"].as_u64().unwrap_or(0) as usize;
                        match delta["type"].as_str() {
                            Some("text_delta") => {
                                if let Some(t) = delta["text"].as_str() {
                                    evs.push(ProtocolEvent::ContentDelta(t.into()));
                                }
                                // 兼容性检查：某些提供商可能在 text_delta 中包含 reasoning_content
                                if let Some(r) =
                                    delta.get("reasoning_content").and_then(|v| v.as_str())
                                {
                                    evs.push(ProtocolEvent::ReasoningDelta(r.into()));
                                }
                            }
                            Some("thinking_delta")
                            | Some("thought_delta")
                            | Some("reasoning_delta") => {
                                if let Some(r) = delta
                                    .get("thinking")
                                    .or_else(|| delta.get("thought"))
                                    .or_else(|| delta.get("reasoning"))
                                    .and_then(|v| v.as_str())
                                {
                                    evs.push(ProtocolEvent::ReasoningDelta(r.into()));
                                }
                            }
                            Some("input_json_delta") => {
                                if let Some(p) = delta["partial_json"].as_str() {
                                    evs.push(ProtocolEvent::ToolCallDelta(
                                        idx,
                                        None,
                                        None,
                                        Some(p.into()),
                                    ));
                                }
                            }
                            _ => {
                                warn!(
                                    delta_type = ?delta["type"],
                                    delta = %delta,
                                    "Anthropic unknown delta type"
                                );
                            }
                        }
                    }
                }
                "error" => {
                    if let Some(m) = json
                        .get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|v| v.as_str())
                    {
                        evs.push(ProtocolEvent::Error(m.into()));
                    }
                }
                _ => {}
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
        let payload = ctx.payload::<serde_json::Value>().unwrap_or_default();
        if payload
            .get("ping")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            return handle_ping(config, &self.get_api_url(config)).await;
        }
        spawn_orchestrator(Box::new(AnthropicProtocol::new()), config, parent, ctx).await
    }
}

pub async fn handle_ping(
    config: &ModelConfig,
    api_url: &str,
) -> Result<PluginPayload, PluginError> {
    let api_key = config.api_key.clone().unwrap_or_default();
    let request = json!({
        "model": config.model,
        "messages": [{"role": "user", "content": "ping"}],
        // Anthropic 协议要求 max_tokens >= 1，部分兼容网关要求更大，统一用安全值
        "max_tokens": 16,
    });

    let response = get_http_client()
        .post(api_url)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("Content-Type", "application/json")
        .json(&request)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| PluginError::InternalError(format!("Network error: {e}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let error_text = response.text().await.unwrap_or_default();
        return Err(PluginError::InternalError(format!(
            "API Error ({status}): {error_text}"
        )));
    }

    Ok(PluginPayload::new(
        &crate::symbio_core::schemas::common::SuccessResponse::default(),
    ))
}

// === 注册到通用对象创建机制 ===

fn build(_ctx: Arc<dyn InvokeRequest>) -> Arc<dyn ModelProtocol> {
    Arc::new(AnthropicProtocol::new())
}

crate::submit_object_creator!(MODEL_PROTOCOL_ANTHROPIC_MESSAGES, build, dyn ModelProtocol);
