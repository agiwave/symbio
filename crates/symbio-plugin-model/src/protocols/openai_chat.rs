//! OpenAI Chat (V1) 协议实现

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use serde_json::{json, Value};
use std::sync::Arc;

use super::super::model_providers::ModelProviderConfig;
use super::super::types::CapabilityMeta;
use super::partial_json::{FieldPath, JsonLineExtractor, PartialJsonSink, StrAction};
use super::profile::{self, first_str, OPENAI_CHAT_REASONING_KEYS};
use super::sse::{SseLineParser, SsePartialLineExtractor};
use super::ModelProtocolEvent;
use super::{description_for_llm, error_text, sse_data, ModelProtocol, MODEL_PROTOCOL_OPENAI_CHAT};
use crate::http::get_http_client;
use symbio_core::capability_to_wire;
use symbio_core::{ModelFinishReason, ModelUsage, PluginError, PluginInvokeRequest};

pub struct OpenaiChatProtocol;

/// 思考分片的字段兼容提取。
///
/// ## 字段名与顺序不在本文件
///
/// 它们是**数据**，在 [`profile::OPENAI_CHAT_REASONING_KEYS`]：OpenAI 兼容生态里思考分片
/// 没有统一约定（DeepSeek 风格 `delta.reasoning_content`、AMD Radeon AI 开发者平台的
/// MiMo-V2.6-Flash 用 `delta.reasoning` 且首帧带 `reasoning_content: null`）。
/// 这里只负责把命中的那一段变成 `String`——链式 `or_else` 若在此处再写一遍，
/// 迟早与 Anthropic 那侧漂移成「一处认 `reasoning`、另一处不认」。
fn reasoning_text_of(delta: &Value) -> Option<String> {
    first_str(delta, OPENAI_CHAT_REASONING_KEYS).map(str::to_string)
}

/// 把消息数组**最前**的连续 `system` 段并进基础 `system`，返回（合并文本，吃掉几条）。
///
/// ## 为什么要并
///
/// 出厂形状原本是**两条** system：这里 prepend 一条，而消息数组的第一元素按
/// [D-048a](../../../symbio-plugin-session/README.md#d-048a-v2-的-prompt-形状必须回到结构化消息不把工具结果拍平成纯文本 "crates/symbio-plugin-session/src/README") 后果节本身就是一条 system（记忆段 /
/// 就绪段 / 委派者段）。两条都排在最前、位置合法，但**部分 OpenAI 兼容网关只认一条**
/// ——它们直接回 `HTTP 400 {"message":"System message must be at the beginning."}`
/// （门外对照：同一份请求体，MiMo 收、qwen3-8-flash-next 拒）。
/// 后果不是「格式不好看」而是硬失败：那类 provider 上 `full` 档第一轮就走不出去。
///
/// ## 为什么是「并入基础 system」而不是让各协议自己并
///
/// 后者要把「合并」写进每一家协议，而**形状基线（D-057）钉的是出厂送出了什么形状**——
/// 形状只该有一个产地。故在此处一次并掉：前缀的文本拼到基础 system 之后（原顺序，
/// 先基础后前缀），空行分段。
///
/// ## 边界（不猜）
///
/// 只合 `content` 是**纯文本**的那些段；遇到 Parts（多模态）就停下不再吃——
/// 拼错比多一条 system 更糟。吃掉 0 条时行为与改前逐字相同。
fn merge_leading_system(system: &str, msgs: &[crate::types::NativeMessage]) -> (String, usize) {
    let mut merged = system.to_string();
    let mut consumed = 0usize;
    for m in msgs {
        if m.role != crate::types::NodeRole::System {
            break;
        }
        // 取 `to_api_value` 算出的最终 content（含 `prompt` 前缀的拼接结果），
        // 而不是自己再拼一遍——拼接口径单点，不在这里开第二份。
        let Some(text) = m
            .to_api_value()
            .get("content")
            .and_then(|c| c.as_str())
            .map(str::to_string)
        else {
            break; // Parts / 空：不猜它的文本形态，停下
        };
        if text.trim().is_empty() {
            consumed += 1 // 空段不吃进文本，但仍算已并（它不该再单独发出去）
        } else {
            if !merged.is_empty() {
                merged.push_str("\n\n")
            }
            merged.push_str(&text);
            consumed += 1
        }
    }
    (merged, consumed)
}

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
                symbio_core::plugin_warn!("model", "Invalid characters in OpenAI API key");
            }
        }
        h
    }

    fn prepare_request(
        &self,
        cfg: &ModelProviderConfig,
        system: &str,
        messages: &[symbio_core::chat_node::ChatNode],
        tools: &[CapabilityMeta],
    ) -> Value {
        let flattened_messages = crate::message_builder::flatten_chat_messages(messages);
        // **出厂只发一条 system**：基础 `system` 与数组最前的 system 前缀段并成一条，
        // 中间空行分段（空行分段是本仓既有的拼接惯例，`NativeMessage::to_api_value`
        // 拼 `prompt` 用的就是它）。
        let (system_text, consumed) = merge_leading_system(system, &flattened_messages);
        let mut openai_msgs = vec![json!({"role": "system", "content": system_text})];
        for m in &flattened_messages[consumed..] {
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

        req["max_tokens"] = json!(cfg.max_tokens.unwrap_or(profile::OPENAI_CHAT.default));
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
            // 探测值见档案表：GLM 那条 `max_tokens > 2` 的下限把 256 钉在这里
            "max_tokens": profile::OPENAI_CHAT.probe,
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
                        evs.push(ModelProtocolEvent::Error(error_text(err)));
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
                    if let Some(r) = reasoning_text_of(delta) {
                        evs.push(ModelProtocolEvent::ReasoningDelta(r));
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
                evs.push(ModelProtocolEvent::Error(error_text(err)));
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

/// `choices[0].delta` 下承载思考正文的字段名——由档案表拼出。
///
/// 与 Anthropic 那侧同一个理由：整行解析与前缀提取必须认同一份名单，否则 core
/// 按前缀截断时会重复或吃字。名单在 [`profile::OPENAI_CHAT_REASONING_KEYS`]，
/// 这里只负责把它套到本协议的路径形状上。
fn is_reasoning_field(path: &FieldPath<'_>) -> bool {
    OPENAI_CHAT_REASONING_KEYS
        .iter()
        .any(|k| path.is(&["choices", "delta", *k]))
}

impl PartialJsonSink for ChatPartial {
    fn begin_string(&mut self, path: &FieldPath<'_>) -> StrAction {
        self.kind = if path.is(&["choices", "delta", "content"]) {
            Some(ChatField::Content)
        } else if is_reasoning_field(path) {
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

symbio_core::submit_object_creator!(MODEL_PROTOCOL_OPENAI_CHAT, build, dyn ModelProtocol);

#[cfg(test)]
#[path = "openai_chat.test.rs"]
mod tests;
