//! OpenAI Responses (Beta) 协议实现

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use serde_json::{json, Value};
use std::sync::Arc;

use super::super::model_providers::ModelProviderConfig;
use super::super::types::{CapabilityMeta, ContentPart, MessageContent, MessageRole};
use super::partial_json::{FieldPath, JsonLineExtractor, PartialJsonSink, StrAction};
use super::{sse_data, ModelProtocol, MODEL_PROTOCOL_OPENAI_RESPONSES};
use crate::symbio_core::sse::PartialLineExtractor;
use crate::symbio_core::{
    get_http_client, FinishReason, InvokeRequest, PluginError, ProtocolEvent, SseLineParser, Usage,
};
use tracing::debug;

pub struct OpenaiResponsesProtocol;

#[async_trait]
impl ModelProtocol for OpenaiResponsesProtocol {
    fn get_api_url(&self, cfg: &ModelProviderConfig) -> String {
        format!("{}/responses", cfg.api_base)
    }

    fn get_headers(&self, cfg: &ModelProviderConfig) -> HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        if let Ok(v) = "application/json".parse() {
            h.insert("Content-Type", v);
        }
        if let Ok(v) = "realtime=v1".parse() {
            h.insert("OpenAI-Beta", v);
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
                }
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
                            }
                        }
                    }

                    if !full_text.is_empty() {
                        input_items.push(json!({
                            "type": "message",
                            "role": "assistant",
                            "content": [{"type": "output_text", "text": full_text}]
                        }));
                    }

                    // 2. 推送工具调用项（不带 status 字段，兼容性更好）
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
                }
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
                }
                _ => {}
            }
        }

        let mut req = json!({
            "model": cfg.model,
            "input": input_items,
            "instructions": system,
            "temperature": cfg.temperature,
            "stream": true,
            "store": cfg.store,
        });

        if let Some(ref pid) = previous_response_id {
            req["previous_response_id"] = json!(pid);
        }
        req["max_output_tokens"] = json!(cfg.max_tokens.unwrap_or(8192));
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

    /// 连通性验证：Responses API 的最小请求探测。
    ///
    /// ping 走独立的轻量请求，而非 `handle_chat_stream` 全量会话路径——
    /// 与其余三协议保持一致。
    async fn ping(&self, cfg: &ModelProviderConfig) -> Result<(), PluginError> {
        let api_key = cfg.api_key.clone().unwrap_or_default();
        let request = json!({
            "model": cfg.model,
            "input": "ping",
            // OpenAI 限制 max_output_tokens 最小为 16
            "max_output_tokens": 16,
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

impl SseLineParser for OpenaiResponsesProtocol {
    fn parse_line(&self, line: &str) -> Vec<ProtocolEvent> {
        let mut evs = Vec::new();
        let Some(data) = sse_data(line) else {
            return evs;
        };
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
                }
                // 推理增量
                "response.reasoning_text.delta" => {
                    if let Some(d) = json.get("delta").and_then(|v| v.as_str()) {
                        evs.push(ProtocolEvent::ReasoningDelta(d.to_string()));
                    }
                }
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
                }
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
                }
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
                }
                "error" => {
                    if let Some(err) = json
                        .get("error")
                        .and_then(|e| e.get("message"))
                        .and_then(|v| v.as_str())
                    {
                        evs.push(ProtocolEvent::Error(err.to_string()));
                    }
                }
                _ => {}
            }

            // 流结束原因 + 用量（Responses API 在 response.completed / response.incomplete
            // 事件中给出，而非每帧带 finish_reason）。
            if let Some(resp) = json.get("response") {
                if let Some(fr) = resp.get("finish_reason").and_then(|v| v.as_str()) {
                    evs.push(ProtocolEvent::Finish(FinishReason::from_provider(Some(fr))));
                }
                if let Some(u) = resp.get("usage") {
                    let input = u
                        .get("input_tokens")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as u32);
                    let output = u
                        .get("output_tokens")
                        .and_then(|v| v.as_u64())
                        .map(|v| v as u32);
                    if input.is_some() || output.is_some() {
                        evs.push(ProtocolEvent::Usage(Usage { input, output }));
                    }
                }
            }
        }
        evs
    }

    fn open_partial_line(&self, _head: &str) -> Option<Box<dyn PartialLineExtractor>> {
        Some(Box::new(
            JsonLineExtractor::new(ResponsesPartial::default()),
        ))
    }
}

/// 未结束行的增量提取：本协议的字段名**不区分**内容 / 推理 / 参数——它由顶层
/// `type` 决定（`response.output_text.delta` / `response.reasoning_text.delta` /
/// `response.function_call_arguments.delta`），三者都把正文放在同名 `delta` 字段里。
///
/// 因此这里先用 [`StrAction::Observe`] 记下顶层 `type`，再据此决定 `delta` 算什么。
///
/// **`response.completed` / `response.output_item.done` 必须排除**：它们携带的是
/// **全量**文本，增量吐出去会让前端重复一遍，而完整行路径对这些事件根本不产出
/// 文本事件 ⇒ 没有前缀截断来兜底。
///
/// **路径必须与 `parse_line` 读的字段一一对应**，否则前缀截断会重复或吃字。
#[derive(Default)]
struct ResponsesPartial {
    /// 顶层 `type` 的取值（`Observe` 攒出来的）
    etype: String,
    /// 是否正在攒 `type`
    observing: bool,
    kind: Option<ResponsesField>,
    /// `response.function_call_arguments.delta` 的归属下标
    output_index: Option<usize>,
}

enum ResponsesField {
    Content,
    Reasoning,
    ToolArgs(usize),
}

impl PartialJsonSink for ResponsesPartial {
    fn begin_string(&mut self, path: &FieldPath<'_>) -> StrAction {
        self.kind = None;
        if path.is(&["type"]) {
            self.etype.clear();
            self.observing = true;
            return StrAction::Observe;
        }
        self.observing = false;
        if path.is(&["delta"]) {
            self.kind = match self.etype.as_str() {
                "response.text.delta" | "response.output_text.delta" => {
                    Some(ResponsesField::Content)
                }
                "response.reasoning_text.delta" => Some(ResponsesField::Reasoning),
                // 下标未知就放弃本次增量（退回等换行），绝不用错下标产出事件
                "response.function_call_arguments.delta" => {
                    self.output_index.map(ResponsesField::ToolArgs)
                }
                // 全量文本事件：明确不做增量
                "response.completed" | "response.output_item.done" => None,
                _ => None,
            };
        }
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
            Some(ResponsesField::Content) => out.push(ProtocolEvent::ContentDelta(t.to_string())),
            Some(ResponsesField::Reasoning) => {
                out.push(ProtocolEvent::ReasoningDelta(t.to_string()))
            }
            Some(ResponsesField::ToolArgs(i)) => out.push(ProtocolEvent::ToolCallDelta(
                i,
                None,
                None,
                Some(t.to_string()),
            )),
            None => {}
        }
    }

    fn scalar(&mut self, path: &FieldPath<'_>, raw: &str) {
        if path.is(&["output_index"]) {
            self.output_index = raw.parse().ok();
        }
    }
}

// === 注册到通用对象创建机制 ===

fn build(_ctx: Arc<dyn InvokeRequest>) -> Arc<dyn ModelProtocol> {
    Arc::new(OpenaiResponsesProtocol)
}

crate::submit_object_creator!(MODEL_PROTOCOL_OPENAI_RESPONSES, build, dyn ModelProtocol);

#[cfg(test)]
#[path = "openai_responses.test.rs"]
mod tests;
