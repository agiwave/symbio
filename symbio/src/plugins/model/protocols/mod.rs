//! MODEL 协议模块 —— 协议契约（model 插件私有）。
//!
//! `trait ModelProtocol`（协议适配钩子）是 model 插件的**私有抽象**：
//! 钩子签名全部收 `&ModelProviderConfig`（持久化配置 schema），由
//! `bound_provider::BoundProvider` 绑定配置与协议后实现 core 的纯
//! `ModelProvider` trait。core 不承载任何协议抽象——插件外部只见
//! `dyn ModelProvider`。
//!
//! 本模块同时承载：
//! - `ModelProtocolEvent`：协议适配器产出的**统一事件方言**——它是
//!   [`sse::SseLineParser::parse_line`] 的返回类型，由 `super::stream` 的流循环消费
//! - `MODEL_PROTOCOL_*` 注册常量（插件内部实现细节，不外泄）
//! - `resolve_protocol_id`：`api_protocol` 别名 → 注册 id 的解析（含兜底）
//! - 各协议实现（openai_chat / openai_responses / anthropic_messages / gemini_api）
//!   与 OpenAI 兼容网关的上下文探测（context_probe）
//! - [`sse`]：`SseLineParser` / `SsePartialLineExtractor` 行解析契约
//!   （2026-09-26 由 `symbio_core::llm::sse` 迁入，理由见该文件文档头）
//!
//! 协议的连通性验证统一为 `ModelProtocol::ping` 直调。

mod anthropic_messages;
mod context_probe;
mod gemini_api;
mod openai_chat;
mod openai_responses;
mod partial_json;
mod sse;

pub(crate) use sse::utf8_chunk;
pub use sse::{SseLineParser, SsePartialLineExtractor};

use crate::plugin_warn;
use crate::symbio_core::schemas::session::chat_message::ChatMessage;
use crate::symbio_core::{CapabilityMeta, ModelFinishReason, ModelUsage, PluginError};
use async_trait::async_trait;
use reqwest::header::HeaderMap;
use serde_json::Value;

use super::model_providers::ModelProviderConfig;

// ============ Model 协议 id ============

/// Anthropic Messages 协议
pub const MODEL_PROTOCOL_ANTHROPIC_MESSAGES: &str = "anthropic_messages";
/// OpenAI Chat Completions 协议
pub const MODEL_PROTOCOL_OPENAI_CHAT: &str = "openai_chat";
/// OpenAI Responses 协议
pub const MODEL_PROTOCOL_OPENAI_RESPONSES: &str = "openai_responses";
/// Gemini API 协议
pub const MODEL_PROTOCOL_GEMINI_API: &str = "gemini_api";

/// 解析用户配置的 `api_protocol` 别名为注册 id。
///
/// 兼容历史别名：`responses`→`openai_responses`、`chat`→`openai_chat`、
/// `anthropic`→`anthropic_messages`、`gemini`→`gemini_api`；
/// 未识别值兜底 `openai_chat`（OpenAI 兼容生态最广）。
pub fn resolve_protocol_id(api_protocol: &str) -> &'static str {
    match api_protocol {
        "openai_responses" | "responses" => MODEL_PROTOCOL_OPENAI_RESPONSES,
        "openai_chat" | "chat" => MODEL_PROTOCOL_OPENAI_CHAT,
        "anthropic_messages" | "anthropic" => MODEL_PROTOCOL_ANTHROPIC_MESSAGES,
        "gemini_api" | "gemini" => MODEL_PROTOCOL_GEMINI_API,
        other => {
            plugin_warn!(
                "model",
                "未识别的 api_protocol \"{other}\"，回退 openai_chat"
            );
            MODEL_PROTOCOL_OPENAI_CHAT
        }
    }
}

/// 从一行 SSE 文本取出 `data:` 之后的载荷。
///
/// 容忍「那个空格在不在」：规范写的是 `data: `，但网关 / 反代在转发时常把空格
/// 去掉或换成 tab。严格匹配的后果不是报错而是**静默空流**——请求成功、一个字都
/// 没有，连 `[DONE]` 都匹配不上，排查成本极高。
///
/// 行尾的 `\r` 一并去掉：分块边界可能把它留在行里，`serde_json` 会因尾随字符
/// 解析失败。返回 `None` = 这一行不是数据行（空行 / 注释 / `event:` 等）。
pub fn sse_data(line: &str) -> Option<&str> {
    let rest = line.strip_prefix("data:")?;
    let rest = rest.strip_prefix([' ', '\t']).unwrap_or(rest);
    Some(rest.trim_end())
}

/// LLM 可见的工具描述：把 `CapabilityMeta.examples` 追加到 description 之后。
///
/// ## 为什么在 model 而不在 core
///
/// 「examples 要送达 LLM」是**协议适配层的组装细节**：core 的 `CapabilityMeta`
/// 只声明 examples 存在，是否拼、以什么措辞拼，由各协议决定——四个协议的请求体
/// 形状本就不同。此前它是 `CapabilityMeta` 上的一个方法，但消费方只有本模块的
/// 四个协议实现，按「依赖方数量」判据（ADR-023）下沉到这里。
///
/// 无 examples 时直接返回原 description，零开销。
pub fn description_for_llm(meta: &CapabilityMeta) -> String {
    match &meta.examples {
        Some(exs) if !exs.is_empty() => {
            format!("{}\n\n示例：\n{}", meta.description, exs.join("\n"))
        }
        _ => meta.description.clone(),
    }
}

/// 标准协议事件 —— 把不同提供商的流解析成**统一方言**。
///
/// ## 依赖方对照表（ADR-023 决策 2）
///
/// | 角色 | 谁 |
/// |---|---|
/// | 生产方 | `anthropic_messages` · `gemini_api` · `openai_chat` · `openai_responses`（完整行）；`partial_json::JsonLineExtractor`（未结束行的增量） |
/// | 消费方 | `super::stream::parse_sse_stream`（唯一消费者，把事件折进 `TurnOutput` 并实时下发子节点） |
///
/// **两个角色同处 model 插件**，所以本枚举不进 `symbio_core`——它是插件内部方言，
/// 不是跨模块契约。对外（session）可见的只有 `ModelProvider::execute_turn` 的返回
/// 类型 `TurnOutput`，协议细节被刻意挡在 trait 之后。
#[derive(Debug, Clone)]
pub enum ModelProtocolEvent {
    /// 文本内容增量
    ContentDelta(String),
    /// 思考/推理过程增量
    ReasoningDelta(String),
    /// 工具调用增量 (index, id, name, arguments_delta)
    ToolCallDelta(usize, Option<String>, Option<String>, Option<String>),
    /// 响应 ID (用于 OpenAI Responses API)
    ResponseId(String),
    /// 错误信息
    Error(String),
    /// 流结束原因（一轮响应最多出现一次）
    Finish(ModelFinishReason),
    /// 用量统计
    Usage(ModelUsage),
}

/// 协议适配钩子 —— model 插件私有契约。
///
/// 钩子签名一律收 `&ModelProviderConfig`（持久化配置 schema），不收
/// `&ModelProvider`（配置 + 协议实例的运行期聚合体）：协议实现只读配置字段，
/// 与绑定方式解耦。
///
/// **行解析在 [`sse`]**：本 trait 以 [`SseLineParser`] 为父 trait，而
/// `super::stream::parse_sse_stream` 只认那个契约（完整行 + 未结束行的增量提取）。
/// 契约与四个协议实现同处本模块，于是「谁实现、谁消费」都在一屏之内；
/// 内核因此不必认识任何协议字段名（ADR-022），也不必承载协议抽象。
#[async_trait]
pub trait ModelProtocol: SseLineParser + Send + Sync {
    /// 请求目标 URL（含路径）
    fn get_api_url(&self, cfg: &ModelProviderConfig) -> String;

    /// 请求头（鉴权 + 协议版本等）
    fn get_headers(&self, cfg: &ModelProviderConfig) -> HeaderMap;

    /// 构造请求体 JSON
    fn prepare_request(
        &self,
        cfg: &ModelProviderConfig,
        system_prompt: &str,
        messages: &[ChatMessage],
        tools: &[CapabilityMeta],
    ) -> Value;

    /// 连通性验证（最小请求探测；默认实现 fail-closed）
    async fn ping(&self, cfg: &ModelProviderConfig) -> Result<(), PluginError> {
        let _ = cfg;
        Err(PluginError::InternalError(
            "protocol does not implement ping".to_string(),
        ))
    }

    /// 探测网关真实可用上下文（尽力而为；None = 不可探测，回退配置值）
    async fn query_context_limit(&self, _cfg: &ModelProviderConfig) -> Option<u32> {
        None
    }
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
