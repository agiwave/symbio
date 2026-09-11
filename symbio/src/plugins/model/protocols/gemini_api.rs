//! Google Gemini API 处理

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use serde_json::{json, Value};
use std::sync::Arc;

use super::super::types::{CapabilityMeta, ContentPart, MessageContent, MessageRole};
use crate::symbio_core::model_provider::{FinishReason, ModelProtocol, ProtocolEvent, Usage};
use crate::symbio_core::{get_http_client, ModelProvider, InvokeRequest, PluginError, MODEL_PROTOCOL_GEMINI_API};

pub struct GeminiProtocol;

#[async_trait]
impl ModelProtocol for GeminiProtocol {
    fn get_api_url(&self, provider: &ModelProvider) -> String {
        format!(
            "{}/models/{}:streamGenerateContent",
            provider.api_base, provider.model
        )
    }

    fn get_headers(&self, provider: &ModelProvider) -> HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        if let Ok(v) = "application/json".parse() {
            h.insert("Content-Type", v);
        }
        if let Some(k) = &provider.api_key {
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
        provider: &ModelProvider,
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
                "temperature": provider.temperature,
                "maxOutputTokens": provider.max_tokens.unwrap_or(8192)
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

                // 流结束原因（Gemini 叫 finishReason，顶层 candidates[0]）
                if let Some(fr) = candidates
                    .first()
                    .and_then(|c| c.get("finishReason"))
                    .and_then(|v| v.as_str())
                {
                    evs.push(ProtocolEvent::Finish(FinishReason::from_provider(Some(fr))));
                }
            }

            // 用量（Gemini 顶层 usageMetadata）
            if let Some(um) = json.get("usageMetadata") {
                let input = um
                    .get("promptTokenCount")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32);
                let output = um
                    .get("candidatesTokenCount")
                    .and_then(|v| v.as_u64())
                    .map(|v| v as u32);
                if input.is_some() || output.is_some() {
                    evs.push(ProtocolEvent::Usage(Usage { input, output }));
                }
            }
        }
        evs
    }

    async fn ping(&self, provider: &ModelProvider) -> Result<(), PluginError> {
        let api_key = provider.api_key.clone().unwrap_or_default();
        let request = json!({
            "contents": [{"parts": [{"text": "ping"}]}],
            "generationConfig": {"maxOutputTokens": 16}
        });

        let response = get_http_client()
            .post(self.get_api_url(provider))
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

        Ok(())
    }

    /// 最大上下文探测：Gemini ListModels（`GET {api_base}/models`）为每个模型
    /// 返回 `inputTokenLimit`，供 session 侧 `min(用户设置, 服务上报)` 收敛使用
    async fn query_context_limit(&self, provider: &ModelProvider) -> Option<u32> {
        let api_base = provider.api_base.clone();
        let model = provider.model.clone();
        let api_key = provider.api_key.clone();
        if api_base.trim().is_empty() || model.trim().is_empty() {
            return None;
        }
        let key = format!("{}|{}", api_base, model);
        super::context_probe::cached_probe(&key, async move {
            let url = format!("{}/models", api_base.trim_end_matches('/'));
            let mut req = get_http_client()
                .get(&url)
                .timeout(std::time::Duration::from_secs(2));
            if let Some(k) = &api_key {
                req = req.header("x-goog-api-key", k);
            }
            let resp = req.send().await.ok()?;
            if !resp.status().is_success() {
                return None;
            }
            let v: serde_json::Value = resp.json().await.ok()?;
            // name 形如 "models/gemini-1.5-pro"（配置里的 model 通常不带前缀）
            let target = model.trim_start_matches("models/");
            v.get("models")?
                .as_array()?
                .iter()
                .find(|m| {
                    m.get("name")
                        .and_then(|n| n.as_str())
                        .map(|n| n.trim_start_matches("models/") == target)
                        .unwrap_or(false)
                })
                .and_then(|m| m.get("inputTokenLimit"))
                .and_then(|v| v.as_u64())
                .and_then(|n| u32::try_from(n).ok())
                .filter(|n| *n > 0)
        })
        .await
    }
}

// === 注册到通用对象创建机制 ===

fn build(_ctx: Arc<dyn InvokeRequest>) -> Arc<dyn ModelProtocol> {
    Arc::new(GeminiProtocol)
}

crate::submit_object_creator!(MODEL_PROTOCOL_GEMINI_API, build, dyn ModelProtocol);
