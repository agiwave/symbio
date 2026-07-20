//! OpenAI Chat (V1) 协议实现

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use serde_json::{json, Value};
use std::sync::Arc;

use super::super::context::get_http_client;
use super::super::types::{CapabilityMeta, ModelConfig};
use super::{spawn_orchestrator, ModelProtocol, ProtocolEvent};
use crate::symbio_core::{
    InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginError, PluginPayload,
    MODEL_PROTOCOL_OPENAI_CHAT,
};

pub struct OpenaiChatProtocol;

#[async_trait]
impl ModelProtocol for OpenaiChatProtocol {
    fn get_api_url(&self, config: &ModelConfig) -> String {
        format!("{}/chat/completions", config.api_base)
    }

    fn get_headers(&self, config: &ModelConfig) -> HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        if let Ok(v) = "application/json".parse() {
            h.insert("Content-Type", v);
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
        let flattened_messages =
            crate::plugins::model::message_builder::flatten_chat_messages(messages);
        let mut openai_msgs = vec![json!({"role": "system", "content": system})];
        for m in &flattened_messages {
            openai_msgs.push(m.to_api_value());
        }

        let mut req = json!({
            "model": config.model,
            "messages": openai_msgs,
            "temperature": config.temperature,
            "stream": true,
        });

        // 处理 OpenAI Reasoning (o1/o3 等)
        if let Some(ref reasoning) = config.reasoning {
            req["reasoning_effort"] = json!(reasoning.effort);
        }

        req["max_tokens"] = json!(config.max_tokens.unwrap_or(8192));
        if !tools.is_empty() {
            req["tools"] = json!(tools
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": t.name.replace("/", "__"),
                            "description": t.description_for_llm(),
                            "parameters": t.input_schema
                        }
                    })
                })
                .collect::<Vec<_>>());
            req["tool_choice"] = json!("auto");
        }
        req
    }

    fn parse_response_line(&self, line: &str) -> Vec<ProtocolEvent> {
        let mut evs = Vec::new();
        if line.is_empty() {
            return evs;
        }

        if !line.starts_with("data: ") {
            // 记录非 data 行（可能是错误 JSON 或 Keep-alive）
            if line.trim().starts_with('{') {
                if let Ok(json) = serde_json::from_str::<Value>(line) {
                    if let Some(err) = json.get("error") {
                        evs.push(ProtocolEvent::Error(err.to_string()));
                    }
                }
            }
            return evs;
        }
        let data = &line[6..];
        if data == "[DONE]" {
            return evs;
        }
        if let Ok(json) = serde_json::from_str::<Value>(data) {
            if let Some(choices) = json.get("choices").and_then(|c| c.as_array()) {
                if let Some(delta) = choices.first().and_then(|c| c.get("delta")) {
                    if let Some(c) = delta.get("content").and_then(|v| v.as_str()) {
                        evs.push(ProtocolEvent::ContentDelta(c.to_string()));
                    }
                    if let Some(r) = delta.get("reasoning_content").and_then(|v| v.as_str()) {
                        evs.push(ProtocolEvent::ReasoningDelta(r.to_string()));
                    }
                    if let Some(tcs) = delta.get("tool_calls").and_then(|v| v.as_array()) {
                        for tc in tcs {
                            let idx =
                                tc.get("index").and_then(|i| i.as_u64()).unwrap_or(0) as usize;
                            let id = tc.get("id").and_then(|v| v.as_str()).map(|s| s.to_string());
                            let func = tc.get("function");
                            let name = func
                                .and_then(|f| f.get("name"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            let args = func
                                .and_then(|f| f.get("arguments"))
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string());
                            evs.push(ProtocolEvent::ToolCallDelta(idx, id, name, args));
                        }
                    }
                }
            }
            if let Some(err) = json.get("error") {
                evs.push(ProtocolEvent::Error(err.to_string()));
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
        spawn_orchestrator(Box::new(OpenaiChatProtocol), config, parent, ctx).await
    }
}

/// Ping 测试 API 可用性
pub async fn handle_ping(
    config: &ModelConfig,
    api_url: &str,
) -> Result<PluginPayload, PluginError> {
    let api_key = config.api_key.clone().unwrap_or_default();
    let request = json!({
        "model": config.model,
        "messages": [{"role": "user", "content": "ping"}],
        "max_tokens": 1,
    });

    let response = get_http_client()
        .post(api_url)
        .header("Authorization", format!("Bearer {api_key}"))
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
    Arc::new(OpenaiChatProtocol)
}

crate::submit_object_creator!(MODEL_PROTOCOL_OPENAI_CHAT, build, dyn ModelProtocol);
