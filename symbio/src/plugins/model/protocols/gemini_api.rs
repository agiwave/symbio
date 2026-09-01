//! Google Gemini API 处理

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use serde_json::{json, Value};
use std::sync::Arc;

use super::super::context::get_http_client;
use super::super::types::{CapabilityMeta, ContentPart, MessageContent, MessageRole, ModelConfig};
use super::{spawn_orchestrator, ModelProtocol, ProtocolEvent};
use crate::symbio_core::{
    InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginError, PluginPayload,
    MODEL_PROTOCOL_GEMINI_API,
};

pub struct GeminiProtocol;

#[async_trait]
impl ModelProtocol for GeminiProtocol {
    fn get_api_url(&self, config: &ModelConfig) -> String {
        format!(
            "{}/models/{}:streamGenerateContent",
            config.api_base, config.model
        )
    }

    fn get_headers(&self, config: &ModelConfig) -> HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        if let Ok(v) = "application/json".parse() {
            h.insert("Content-Type", v);
        }
        if let Some(k) = &config.api_key {
            if let Ok(v) = k.parse() {
                h.insert("x-goog-api-key", v);
            } else {
                crate::plugin_warn!("model", "Invalid characters in Gemini API key");
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
        let mut contents: Vec<Value> = Vec::new();
        for m in &flattened_messages {
            if m.role == MessageRole::System {
                continue;
            }
            let mut parts = Vec::new();
            if let Some(ref content) = m.content {
                match content {
                    MessageContent::Text(t) => {
                        if !t.is_empty() {
                            parts.push(json!({"text": t}));
                        }
                    }
                    MessageContent::Parts(p) => {
                        for part in p {
                            match part {
                                ContentPart::Text { text } => {
                                    parts.push(json!({"text": text}));
                                }
                                ContentPart::ImageUrl { image_url } => {
                                    // Gemini expects: { "inlineData": { "mimeType": "image/jpeg", "data": "..." } }
                                    let (media_type, base64_data) =
                                        if image_url.url.starts_with("data:") {
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
                                        "inlineData": {
                                            "mimeType": media_type,
                                            "data": base64_data
                                        }
                                    }));
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
                        "functionCall": {
                            "name": tc.name.replace("/", "__"),
                            "args": args
                        }
                    }));
                }
            }
            if m.role == MessageRole::Tool {
                let text = match m.content {
                    Some(MessageContent::Text(ref t)) => t.clone(),
                    _ => "{}".into(),
                };
                parts.push(json!({
                    "functionResponse": {
                        "name": m.tool_call_id.as_ref().cloned().unwrap_or_default().replace("/", "__"), 
                        "response": {"result": text}
                    }
                }));
            }
            if parts.is_empty() {
                continue;
            }
            contents.push(json!({
                "role": if m.role == MessageRole::Assistant { "model" } else { "user" },
                "parts": parts
            }));
        }

        let mut req = json!({
            "contents": contents,
            "systemInstruction": {"parts": [{"text": system}]},
            "generationConfig": {
                "temperature": config.temperature,
                "maxOutputTokens": config.max_tokens.unwrap_or(8192)
            }
        });
        if !tools.is_empty() {
            req["tools"] = json!([{
                "functionDeclarations": tools.iter().map(|t| json!({
                    "name": t.name.replace("/", "__"),
                    "description": t.description_for_llm(),
                    "parameters": t.input_schema
                })).collect::<Vec<_>>()
            }]);
        }
        req
    }

    fn parse_response_line(&self, line: &str) -> Vec<ProtocolEvent> {
        let mut evs = Vec::new();
        let trimmed = line.trim();
        // 处理 Gemini 可能的数组包裹格式
        if trimmed.is_empty() || trimmed == "[" || trimmed == "]" || trimmed == "," {
            return evs;
        }
        let clean = trimmed.strip_prefix(',').unwrap_or(trimmed);

        if let Ok(json) = serde_json::from_str::<Value>(clean) {
            if let Some(candidates) = json.get("candidates").and_then(|c| c.as_array()) {
                if let Some(content) = candidates.first().and_then(|c| c.get("content")) {
                    if let Some(parts) = content.get("parts").and_then(|p| p.as_array()) {
                        for part in parts {
                            if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                                evs.push(ProtocolEvent::ContentDelta(text.into()));
                            }
                            if let Some(fc) = part.get("functionCall") {
                                let name =
                                    fc.get("name").and_then(|v| v.as_str()).map(|s| s.into());
                                let args = fc.get("args").map(|v| v.to_string());
                                // Gemini 每次返回完整调用，因此生成新 ID
                                evs.push(ProtocolEvent::ToolCallDelta(
                                    0,
                                    Some(uuid::Uuid::new_v4().to_string()),
                                    name,
                                    args,
                                ));
                            }
                        }
                    }
                }
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
        spawn_orchestrator(Box::new(GeminiProtocol), config, parent, ctx).await
    }
}

pub async fn handle_ping(
    config: &ModelConfig,
    api_url: &str,
) -> Result<PluginPayload, PluginError> {
    let api_key = config.api_key.clone().unwrap_or_default();
    let request = json!({
        "contents": [{"parts": [{"text": "ping"}]}],
        "generationConfig": {"maxOutputTokens": 16}
    });

    let response = get_http_client()
        .post(api_url)
        .header("x-goog-api-key", api_key)
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
    Arc::new(GeminiProtocol)
}

crate::submit_object_creator!(MODEL_PROTOCOL_GEMINI_API, build, dyn ModelProtocol);
