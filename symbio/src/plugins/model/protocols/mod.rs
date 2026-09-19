//! MODEL 协议模块 —— 协议契约（model 插件私有）。
//!
//! `trait ModelProtocol`（协议适配钩子）是 model 插件的**私有抽象**：
//! 钩子签名全部收 `&ModelProviderConfig`（持久化配置 schema），由
//! `bound_provider::BoundProvider` 绑定配置与协议后实现 core 的纯
//! `ModelProvider` trait。core 不承载任何协议抽象——插件外部只见
//! `dyn ModelProvider`。
//!
//! 本模块同时承载：
//! - `MODEL_PROTOCOL_*` 注册常量（插件内部实现细节，不外泄）
//! - `resolve_protocol_id`：`api_protocol` 别名 → 注册 id 的解析（含兜底）
//! - 各协议实现（openai_chat / openai_responses / anthropic_messages / gemini_api）
//!   与 OpenAI 兼容网关的上下文探测（context_probe）
//!
//! 协议的连通性验证统一为 `ModelProtocol::ping` 直调。

mod anthropic_messages;
mod context_probe;
mod gemini_api;
mod openai_chat;
mod openai_responses;

use crate::plugin_warn;
use crate::symbio_core::schemas::session::chat_message::ChatMessage;
use crate::symbio_core::{CapabilityMeta, PluginError, ProtocolEvent};
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

/// 协议适配钩子 —— model 插件私有契约。
///
/// 钩子签名一律收 `&ModelProviderConfig`（持久化配置 schema），不收
/// `&ModelProvider`（配置 + 协议实例的运行期聚合体）：协议实现只读配置字段，
/// 与绑定方式解耦。
#[async_trait]
pub trait ModelProtocol: Send + Sync {
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

    /// 解析一行 SSE 数据为协议事件（空行/注释行返回空 Vec）
    fn parse_response_line(&self, line: &str) -> Vec<ProtocolEvent>;

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
mod tests {
    use super::*;

    /// SSE 载荷解析必须容忍「那个空格在不在」与行尾 `\r`——
    /// 严格匹配 `"data: "` 的后果不是报错而是**静默空流**（连 `[DONE]` 都匹配不上）。
    #[test]
    fn sse_data_tolerates_missing_space_and_trailing_cr() {
        assert_eq!(sse_data("data: {\"a\":1}"), Some("{\"a\":1}"));
        assert_eq!(sse_data("data:{\"a\":1}"), Some("{\"a\":1}"));
        assert_eq!(sse_data("data:\t{\"a\":1}"), Some("{\"a\":1}"));
        assert_eq!(sse_data("data: {\"a\":1}\r"), Some("{\"a\":1}"));
        assert_eq!(sse_data("data: [DONE]"), Some("[DONE]"));
        assert_eq!(sse_data("data:[DONE]"), Some("[DONE]"));
    }

    /// 非数据行必须仍然返回 None（否则会被当成 JSON 去解析）
    #[test]
    fn sse_data_rejects_non_data_lines() {
        assert_eq!(sse_data("event: message_start"), None);
        assert_eq!(sse_data(""), None);
        assert_eq!(sse_data(": keep-alive"), None);
        assert_eq!(sse_data("{\"error\":1}"), None);
        assert_eq!(sse_data("data"), None);
    }

    #[test]
    fn protocol_aliases_resolve_to_registered_ids() {
        assert_eq!(resolve_protocol_id("chat"), MODEL_PROTOCOL_OPENAI_CHAT);
        assert_eq!(
            resolve_protocol_id("responses"),
            MODEL_PROTOCOL_OPENAI_RESPONSES
        );
        assert_eq!(
            resolve_protocol_id("anthropic"),
            MODEL_PROTOCOL_ANTHROPIC_MESSAGES
        );
        assert_eq!(resolve_protocol_id("gemini"), MODEL_PROTOCOL_GEMINI_API);
        // 未识别值兜底 openai_chat
        assert_eq!(resolve_protocol_id("bogus"), MODEL_PROTOCOL_OPENAI_CHAT);
    }
}
