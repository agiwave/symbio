//! Anthropic 协议处理

use async_trait::async_trait;
// use async_trait::async_trait;

use reqwest::header::HeaderMap;
use serde_json::{json, Value};
use std::sync::Arc;
use std::sync::Mutex;

use super::super::model_providers::ModelProviderConfig;
use super::super::types::{CapabilityMeta, ContentPart, MessageContent, MessageRole};
use super::partial_json::{FieldPath, JsonLineExtractor, PartialJsonSink, StrAction};
use super::{sse_data, ModelProtocol, MODEL_PROTOCOL_ANTHROPIC_MESSAGES};
use crate::symbio_core::sse::PartialLineExtractor;
use crate::symbio_core::{
    get_http_client, FinishReason, InvokeRequest, PluginError, ProtocolEvent, SseLineParser, Usage,
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
    fn get_api_url(&self, cfg: &ModelProviderConfig) -> String {
        format!("{}/messages", cfg.api_base)
    }

    fn get_headers(&self, cfg: &ModelProviderConfig) -> HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        if let Ok(v) = "application/json".parse() {
            h.insert("Content-Type", v);
        }
        if let Ok(v) = "2023-06-01".parse() {
            h.insert("anthropic-version", v);
        }
        if let Some(k) = &cfg.api_key {
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
        cfg: &ModelProviderConfig,
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
            // 推理内容（Reasoning / Thinking）不回传：
            // 1) 我们保存的 reasoning 没有 Anthropic 官方 signature，无法构造合法的
            //    thinking 块；以 <thought> 文本块形式回传对官方 API 是无效内容，
            //    只会白白占用上下文窗口并干扰模型。
            // 2) OpenAI / Gemini 协议同样不回传历史 reasoning（它是模型内部过程），
            //    此处保持各协议行为一致。
            // 注意：本轮请求开启 thinking 时，模型会基于消息历史重新推理，
            // 历史思考内容并非必需输入。

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
            "model": cfg.model,
            "system": system,
            "messages": anthropic_msgs,
            "temperature": cfg.temperature,
            "stream": true
        });

        // 处理 Anthropic Thinking (Claude 3.7+)
        if cfg.reasoning.is_some() {
            let budget = (cfg.max_tokens.unwrap_or(4096) / 2).max(1024);
            req["thinking"] = json!({
                "type": "enabled",
                "budget_tokens": budget
            });
            // Anthropic 规定开启 thinking 时 temperature 必须为 1.0
            req["temperature"] = json!(1.0);

            // 确保 max_tokens 大于 budget
            if cfg.max_tokens.unwrap_or(8192) <= budget {
                req["max_tokens"] = json!(budget + 1024);
            }
        }

        if let Some(m) = cfg.max_tokens {
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

    async fn ping(&self, cfg: &ModelProviderConfig) -> Result<(), PluginError> {
        let api_key = cfg.api_key.clone().unwrap_or_default();
        let request = json!({
            "model": cfg.model,
            "messages": [{"role": "user", "content": "ping"}],
            // Anthropic 协议要求 max_tokens >= 1，部分兼容网关要求更大，统一用安全值
            "max_tokens": 16,
        });

        let response = get_http_client()
            .post(self.get_api_url(cfg))
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

        Ok(())
    }
}

// === 行解析（core 契约） ===

impl SseLineParser for AnthropicProtocol {
    fn parse_line(&self, line: &str) -> Vec<ProtocolEvent> {
        let mut evs = Vec::new();
        if let Some(stripped) = line.strip_prefix("event:") {
            let mut etype = self.current_event_type.lock().unwrap();
            let type_val = stripped.trim().to_string();
            *etype = Some(type_val);
            return evs;
        }

        let Some(data) = sse_data(line) else {
            return evs;
        };
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
                "message_start" => {
                    // 携带 input_tokens（与 message_delta 的 output_tokens 合并为 Usage）
                    if let Some(in_tok) = json
                        .get("message")
                        .and_then(|m| m.get("usage"))
                        .and_then(|u| u.get("input_tokens"))
                        .and_then(|v| v.as_u64())
                    {
                        evs.push(ProtocolEvent::Usage(Usage {
                            input: Some(in_tok as u32),
                            output: None,
                        }));
                    }
                }
                "message_delta" => {
                    // 流结束原因（Anthropic 叫 stop_reason）
                    if let Some(stop) = json
                        .get("delta")
                        .and_then(|d| d.get("stop_reason"))
                        .and_then(|v| v.as_str())
                    {
                        evs.push(ProtocolEvent::Finish(FinishReason::from_provider(Some(
                            stop,
                        ))));
                    }
                    // 携带 output_tokens
                    if let Some(out_tok) = json
                        .get("usage")
                        .and_then(|u| u.get("output_tokens"))
                        .and_then(|v| v.as_u64())
                    {
                        evs.push(ProtocolEvent::Usage(Usage {
                            input: None,
                            output: Some(out_tok as u32),
                        }));
                    }
                }
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

    fn open_partial_line(&self, _head: &str) -> Option<Box<dyn PartialLineExtractor>> {
        Some(Box::new(
            JsonLineExtractor::new(AnthropicPartial::default()),
        ))
    }
}

/// 未结束行的增量提取：只认 `content_block_delta` 事件里 `delta.*` 那几个字段。
///
/// 判别方式与 `parse_line` 不同：那里靠 SSE 的 `event:` 行（存在 `self` 上），这里
/// 只能看 JSON 自带的顶层 `type`——`event:` 是**上一行**，本行的提取器看不到它。
/// Anthropic 的 `content_block_delta` 数据里必然带 `"type":"content_block_delta"`，
/// 且 `delta.type` 的取值与字段名一一对应（`text_delta`→`text`、
/// `thinking_delta`→`thinking`…），所以**按字段名归类**即可，不必读 `delta.type`。
///
/// **路径必须与 `parse_line` 读的字段一一对应**，否则前缀截断会重复或吃字。
#[derive(Default)]
struct AnthropicPartial {
    /// 顶层 `type` 的取值（`Observe` 攒出来的）
    etype: String,
    observing: bool,
    kind: Option<AnthropicField>,
    /// `content_block_delta` 的块下标（工具参数归属）
    index: Option<usize>,
}

enum AnthropicField {
    Content,
    Reasoning,
    ToolArgs(usize),
}

impl PartialJsonSink for AnthropicPartial {
    fn begin_string(&mut self, path: &FieldPath<'_>) -> StrAction {
        self.kind = None;
        if path.is(&["type"]) {
            self.etype.clear();
            self.observing = true;
            return StrAction::Observe;
        }
        self.observing = false;
        // 只有 content_block_delta 会产出增量；别的事件的 `delta` 是别的东西
        // （例如 `message_delta.delta.stop_reason`），按字段名匹配会误伤。
        if self.etype != "content_block_delta" {
            return StrAction::Skip;
        }
        self.kind = if path.is(&["delta", "text"]) {
            Some(AnthropicField::Content)
        } else if path.is(&["delta", "reasoning_content"])
            || path.is(&["delta", "thinking"])
            || path.is(&["delta", "thought"])
            || path.is(&["delta", "reasoning"])
        {
            Some(AnthropicField::Reasoning)
        } else if path.is(&["delta", "partial_json"]) {
            // 下标未知就放弃本次增量（退回等换行），绝不用错下标产出事件
            self.index.map(AnthropicField::ToolArgs)
        } else {
            None
        };
        if self.kind.is_some() {
            StrAction::Emit
        } else {
            StrAction::Skip
        }
    }

    fn text(&mut self, t: &str, out: &mut Vec<ProtocolEvent>) {
        if self.observing {
            self.etype.push_str(t);
            return;
        }
        match self.kind {
            Some(AnthropicField::Content) => out.push(ProtocolEvent::ContentDelta(t.to_string())),
            Some(AnthropicField::Reasoning) => {
                out.push(ProtocolEvent::ReasoningDelta(t.to_string()))
            }
            Some(AnthropicField::ToolArgs(i)) => out.push(ProtocolEvent::ToolCallDelta(
                i,
                None,
                None,
                Some(t.to_string()),
            )),
            None => {}
        }
    }

    fn scalar(&mut self, path: &FieldPath<'_>, raw: &str) {
        if path.is(&["index"]) {
            self.index = raw.parse().ok();
        }
    }
}

// === 注册到通用对象创建机制 ===

fn build(_ctx: Arc<dyn InvokeRequest>) -> Arc<dyn ModelProtocol> {
    Arc::new(AnthropicProtocol::new())
}

crate::submit_object_creator!(MODEL_PROTOCOL_ANTHROPIC_MESSAGES, build, dyn ModelProtocol);

#[cfg(test)]
#[path = "anthropic_messages.test.rs"]
mod tests;
