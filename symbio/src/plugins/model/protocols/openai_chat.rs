//! OpenAI Chat (V1) 协议实现

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use serde_json::{json, Value};
use std::sync::Arc;

use super::super::model_providers::ModelProviderConfig;
use super::super::types::CapabilityMeta;
use super::partial_json::{FieldPath, JsonLineExtractor, PartialJsonSink, StrAction};
use super::sse::{SseLineParser, SsePartialLineExtractor};
use super::ModelProtocolEvent;
use super::{description_for_llm, sse_data, ModelProtocol, MODEL_PROTOCOL_OPENAI_CHAT};
use crate::plugins::model::http::get_http_client;
use crate::symbio_core::capability_to_wire;
use crate::symbio_core::{ModelFinishReason, ModelUsage, PluginError, PluginInvokeRequest};

pub struct OpenaiChatProtocol;

#[async_trait]
impl ModelProtocol for OpenaiChatProtocol {
    fn get_api_url(&self, cfg: &ModelProviderConfig) -> String {
        format!("{}/chat/completions", cfg.api_base)
    }

    fn get_headers(&self, cfg: &ModelProviderConfig) -> HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        if let Ok(v) = "application/json".parse() {
            h.insert("Content-Type", v);
        }
        if let Some(k) = &cfg.api_key {
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
        cfg: &ModelProviderConfig,
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
            "model": cfg.model,
            "messages": openai_msgs,
            "temperature": cfg.temperature,
            "stream": true,
        });

        // 处理 OpenAI Reasoning (o1/o3 等)
        if let Some(ref reasoning) = cfg.reasoning {
            req["reasoning_effort"] = json!(reasoning.effort);
        }

        req["max_tokens"] = json!(cfg.max_tokens.unwrap_or(8192));
        if !tools.is_empty() {
            req["tools"] = json!(tools
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": capability_to_wire(&t.name),
                            "description": description_for_llm(t),
                            "parameters": t.input_schema
                        }
                    })
                })
                .collect::<Vec<_>>());
            req["tool_choice"] = json!("auto");
        }
        req
    }

    async fn ping(&self, cfg: &ModelProviderConfig) -> Result<(), PluginError> {
        let api_key = cfg.api_key.clone().unwrap_or_default();
        let request = json!({
            "model": cfg.model,
            "messages": [{"role": "user", "content": "ping"}],
            // 注意：部分 OpenAI 兼容网关（如 GLM）要求 max_tokens > 2，不能设为 1
            "max_tokens": 256,
        });

        let response = get_http_client()
            .post(self.get_api_url(cfg))
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

        Ok(())
    }

    /// 最大上下文探测：OpenAI 兼容网关（Ollama / LM Studio / vLLM 等）
    /// 的上报值，供 session 侧 `min(用户设置, 服务上报)` 收敛使用
    async fn query_context_limit(&self, cfg: &ModelProviderConfig) -> Option<u32> {
        super::context_probe::probe_openai_compat_context(cfg).await
    }
}

// === 行解析（core 契约） ===

impl SseLineParser for OpenaiChatProtocol {
    fn parse_line(&self, line: &str) -> Vec<ModelProtocolEvent> {
        let mut evs = Vec::new();
        if line.is_empty() {
            return evs;
        }

        let Some(data) = sse_data(line) else {
            // 记录非 data 行（可能是错误 JSON 或 Keep-alive）
            if line.trim().starts_with('{') {
                if let Ok(json) = serde_json::from_str::<Value>(line) {
                    if let Some(err) = json.get("error") {
                        evs.push(ModelProtocolEvent::Error(err.to_string()));
                    }
                }
            }
            return evs;
        };
        if data == "[DONE]" {
            return evs;
        }
        if let Ok(json) = serde_json::from_str::<Value>(data) {
            if let Some(choices) = json.get("choices").and_then(|c| c.as_array()) {
                if let Some(delta) = choices.first().and_then(|c| c.get("delta")) {
                    if let Some(c) = delta.get("content").and_then(|v| v.as_str()) {
                        evs.push(ModelProtocolEvent::ContentDelta(c.to_string()));
                    }
                    if let Some(r) = delta.get("reasoning_content").and_then(|v| v.as_str()) {
                        evs.push(ModelProtocolEvent::ReasoningDelta(r.to_string()));
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
                            evs.push(ModelProtocolEvent::ToolCallDelta(idx, id, name, args));
                        }
                    }
                }
            }
            // 结束原因：`finish_reason` 为 null 的中间帧不 emit（用 as_str 过滤），
            // 只有真正带上字符串时才产出 Finish。
            if let Some(fr) = json
                .get("choices")
                .and_then(|c| c.as_array())
                .and_then(|c| c.first())
                .and_then(|c| c.get("finish_reason"))
                .and_then(|v| v.as_str())
            {
                evs.push(ModelProtocolEvent::Finish(
                    ModelFinishReason::from_provider(Some(fr)),
                ));
            }
            // 用量：通常只在最后的 chunk 出现（需 stream_options.include_usage），
            // 拿不到也没关系——估算器照样工作，只是失去校准机会。
            if let Some(u) = json.get("usage") {
                evs.push(ModelProtocolEvent::Usage(ModelUsage {
                    input: u
                        .get("prompt_tokens")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as u32),
                    output: u
                        .get("completion_tokens")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as u32),
                }));
            }
            if let Some(err) = json.get("error") {
                evs.push(ModelProtocolEvent::Error(err.to_string()));
            }
        }
        evs
    }

    fn open_partial_line(&self, _head: &str) -> Option<Box<dyn SsePartialLineExtractor>> {
        Some(Box::new(JsonLineExtractor::new(ChatPartial::default())))
    }
}

/// 未结束行的增量提取：只认 `choices[0].delta.*` 三个字段。
///
/// **路径必须与上面 `parse_line` 读的字段一一对应**：两边不一致的后果不是报错，
/// 而是「增量吐出的文本不是完整行文本的前缀」——core 按前缀截断时会重复或吃字。
#[derive(Default)]
struct ChatPartial {
    kind: Option<ChatField>,
}

enum ChatField {
    Content,
    Reasoning,
    ToolArgs(usize),
}

impl PartialJsonSink for ChatPartial {
    fn begin_string(&mut self, path: &FieldPath<'_>) -> StrAction {
        self.kind = if path.is(&["choices", "delta", "content"]) {
            Some(ChatField::Content)
        } else if path.is(&["choices", "delta", "reasoning_content"]) {
            Some(ChatField::Reasoning)
        } else if path.is(&["choices", "delta", "tool_calls", "function", "arguments"]) {
            // 属于第几个工具调用由「最内层数组下标」给出（`tool_calls[N]`）
            Some(ChatField::ToolArgs(path.array_index.unwrap_or(0)))
        } else {
            None
        };
        if self.kind.is_some() {
            StrAction::Emit
        } else {
            StrAction::Skip
        }
    }

    fn text(&mut self, t: &str, out: &mut Vec<ModelProtocolEvent>) {
        match self.kind {
            Some(ChatField::Content) => out.push(ModelProtocolEvent::ContentDelta(t.to_string())),
            Some(ChatField::Reasoning) => {
                out.push(ModelProtocolEvent::ReasoningDelta(t.to_string()))
            }
            Some(ChatField::ToolArgs(i)) => out.push(ModelProtocolEvent::ToolCallDelta(
                i,
                None,
                None,
                Some(t.to_string()),
            )),
            None => {}
        }
    }
}

// === 注册到通用对象创建机制 ===

fn build(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn ModelProtocol> {
    Arc::new(OpenaiChatProtocol)
}

crate::submit_object_creator!(MODEL_PROTOCOL_OPENAI_CHAT, build, dyn ModelProtocol);

#[cfg(test)]
#[path = "openai_chat.test.rs"]
mod tests;
