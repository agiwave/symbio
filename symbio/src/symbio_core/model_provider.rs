//! 核心 ModelProvider 接口 —— 不同协议大语言模型的请求构造与响应解析封装。
//!
//! 本模块是 Phase A（Model/Session 分工改造）的产物：原 `plugins/model/protocols`
//! 中的协议契约上移到 core，使"协议适配"成为系统级契约——
//! - 会话引擎（session）与工具编排可依赖本契约，而无需依赖 model 插件内部；
//! - model 插件降级为无状态网关：仅负责 provider 注册表、HTTP/SSE 执行与
//!   标准化事件流产出（详见 docs/model-session-refactor.md）。
//!
//! 协议实现的注册仍走 `submit_object_creator!` 工厂机制
//! （`create_object::<dyn ModelProvider>(protocol_id, ctx)` 取得实例），
//! 实现体保留在 model 插件的 `protocols/` 目录下。

use crate::plugin_info;
use crate::plugin_warn;
use crate::symbio_core::schemas::model::model_config::ModelConfig;
use crate::symbio_core::CapabilityMeta;
use async_trait::async_trait;
use reqwest::header::HeaderMap;
use serde_json::Value;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use super::turn::{emit_abort, execute_post_with_abort, parse_sse_stream, PostResult, TurnOutput};
use crate::symbio_core::schemas::session::chat_message::ChatMessage;
use crate::symbio_core::{PluginChannel, PluginError};

/// 流结束原因（由各协议的 `finish_reason` / `stop_reason` / `finishReason` 归一化）
///
/// ## 为什么必须有它
///
/// 在此之前，流结束一律被当成"正常完成"。当模型因 `max_tokens` 用尽而停在
/// `Length` 时，系统会：
/// 1. 把断在半句的文本当完整回复呈现；
/// 2. 若截断发生在 `tool_calls` 的参数 JSON 中间，工具调用永远收集不完 →
///    `tools_done` 为空 → 循环按"无工具调用"正常退出——**用户看到的就是"对话突然结束"**。
///
/// 有了 Finish，`chat_loop` 才能区分"自然结束"与"被长度截断"，并触发自动续写或明确报错。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum FinishReason {
    /// 自然结束
    #[default]
    Stop,
    /// 输出达到长度上限被截断（`finish_reason=length` / `stop_reason=max_tokens` / `MAX_TOKENS`）
    Length,
    /// 因需要调用工具而结束
    ToolCalls,
    /// 被内容安全策略过滤
    ContentFilter,
    /// 未识别的原因（保留原始字符串，便于排查兼容网关）
    Other(String),
}

impl FinishReason {
    /// 从 provider 的原始字符串归一化。`None` 视为自然结束。
    pub fn from_provider(raw: Option<&str>) -> Self {
        match raw.map(|s| s.trim()).unwrap_or("") {
            "" | "stop" | "end_turn" | "STOP" | "stop_sequence" | "completed" => FinishReason::Stop,
            "length" | "max_tokens" | "MAX_TOKENS" | "max_output_tokens" | "incomplete" => {
                FinishReason::Length
            }
            "tool_calls" | "tool_use" | "function_call" => FinishReason::ToolCalls,
            "content_filter" | "SAFETY" | "RECITATION" | "refusal" => FinishReason::ContentFilter,
            other => FinishReason::Other(other.to_string()),
        }
    }

    pub fn is_length(&self) -> bool {
        matches!(self, FinishReason::Length)
    }
}

/// 单次请求的用量（用于校准 token 估算；provider 不一定给，故全部可选）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Usage {
    pub input: Option<u32>,
    pub output: Option<u32>,
}

/// 标准协议事件 - 用于将不同提供商的流解析为统一格式
#[derive(Debug, Clone)]
pub enum ProtocolEvent {
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
    Finish(FinishReason),
    /// 用量统计
    Usage(Usage),
}

/// MODEL 协议特质 - 抽象不同模型提供商的通信细节
#[async_trait]
pub trait ModelProvider: Send + Sync {
    /// 获取 API 请求端点 URL
    fn get_api_url(&self, config: &ModelConfig) -> String;

    /// 获取 HTTP 请求头
    fn get_headers(&self, config: &ModelConfig) -> HeaderMap;

    /// 构造请求体 JSON
    fn prepare_request(
        &self,
        config: &ModelConfig,
        system_prompt: &str,
        messages: &[ChatMessage],
        tools: &[CapabilityMeta],
    ) -> Value;

    /// 连通性验证：向 provider 发起一次最小代价请求（如 `max_tokens` 极小的
    /// 单轮 ping），仅用于配置校验与存活探测。
    ///
    /// Phase E-②：取代原 `get_validation_input` + `handle_chat_stream` 组合的
    /// 帧循环验证路径——验证不应借助会话通道收帧，直接返回 `Ok(())` /
    /// `Err(e)` 即可。openai_responses 的 ping 语义见 `docs/model-session-refactor.md`
    /// 8.4.1（修复"验证路径不对称"问题）。
    async fn ping(&self, config: &ModelConfig) -> Result<(), PluginError> {
        let _ = config;
        // fail-closed：协议未显式实现 ping 时，验证必须显式失败而非静默放行
        Err(PluginError::InternalError(
            "protocol does not implement ping".to_string(),
        ))
    }

    /// 解析流的一行内容（处理 SSE 协议）
    fn parse_response_line(&self, line: &str) -> Vec<ProtocolEvent>;

    /// 执行完整单轮 LLM 请求：构造请求体 → 带中止的 POST → SSE 流解析。
    ///
    /// Phase E 的统一入口。把原先散落在 model 插件 `turn_processor::send_request`
    /// 的五态匹配（`Aborted` / `RetryWithoutContextId` / `Err` / `RateLimited` / `Ok`）
    /// 与 SSE 解析收编到 core，使任何 `ModelProvider` 实现都自带完整的
    /// 「请求-解析」能力，会话引擎无需再依赖 model 插件内部实现。
    ///
    /// 默认实现对所有协议通用：协议差异已被 `get_api_url` / `get_headers` /
    /// `prepare_request` / `parse_response_line` 四个钩子吸收，故各协议无需重复实现。
    #[allow(clippy::too_many_arguments)]
    async fn execute_turn(
        &self,
        config: &ModelConfig,
        system_prompt: &str,
        messages: &[ChatMessage],
        tools: &[CapabilityMeta],
        root_id: &str,
        channel: &mut PluginChannel,
        abort_flag: &Arc<AtomicBool>,
    ) -> Result<TurnOutput, PluginError> {
        let body = self.prepare_request(config, system_prompt, messages, tools);

        plugin_info!(
            "model",
            "[DIAG] ModelProvider::execute_turn entered, url={}, msg_count={}, tool_count={}",
            self.get_api_url(config),
            messages.len(),
            tools.len()
        );

        let response = match execute_post_with_abort(
            &self.get_api_url(config),
            self.get_headers(config),
            &body,
            channel,
            abort_flag,
        )
        .await
        {
            PostResult::Aborted => {
                plugin_warn!(
                    "model",
                    "[DIAG] ModelProvider::execute_turn: PostResult::Aborted"
                );
                return Err(PluginError::Aborted);
            }
            PostResult::RetryWithoutContextId => {
                plugin_warn!(
                    "model",
                    "Response context lost (400). Retrying turn without response_ids..."
                );
                emit_abort(channel).await;
                return Err(PluginError::RetryWithoutContextId);
            }
            PostResult::Err(msg) => {
                plugin_warn!(
                    "model",
                    "[DIAG] ModelProvider::execute_turn: PostResult::Err({})",
                    msg
                );
                return Err(PluginError::InternalError(msg));
            }
            PostResult::RateLimited(msg) => {
                plugin_warn!(
                    "model",
                    "[DIAG] ModelProvider::execute_turn: PostResult::RateLimited({})",
                    msg
                );
                return Err(PluginError::RateLimited(msg));
            }
            PostResult::Ok(resp) => {
                plugin_info!(
                    "model",
                    "[DIAG] ModelProvider::execute_turn: PostResult::Ok, status={}",
                    resp.status()
                );
                resp
            }
        };

        match parse_sse_stream(response, root_id, channel, abort_flag, self).await {
            Err(msg) => {
                plugin_warn!(
                    "model",
                    "[DIAG] ModelProvider::execute_turn: parse_sse_stream Err({})",
                    msg
                );
                Err(PluginError::StreamError(msg))
            }
            Ok(mut out) => {
                plugin_info!(
                    "model",
                    "[DIAG] ModelProvider::execute_turn: parse_sse_stream Ok, text_len={}, reasoning_len={}, tool_calls={}",
                    out.text.len(),
                    out.reasoning.len(),
                    out.tool_accumulator.get_completed().len()
                );
                Ok(out)
            }
        }
    }
}

/// 协议工厂 - 通过通用对象创建机制按 id 构造
///
/// 各协议实现各自在自己的文件里提供 `build` 函数并通过
/// `submit_object_creator!` 自动注册；调用方使用
/// `create_object::<dyn ModelProvider>(id, ctx)` 取得实例。
///
/// 已注册 id（参见各协议文件）：
/// - `openai_responses`（别名 `responses`）— openai_responses.rs
/// - `openai_chat`（别名 `chat`）— openai_chat.rs
/// - `anthropic_messages`（别名 `anthropic`）— anthropic_messages.rs
/// - `gemini_api`（别名 `gemini`）— gemini_api.rs
///
/// 将 `ModelConfig.api_protocol` 别名解析为已注册的协议 id
pub fn resolve_protocol_id(name: &str) -> &'static str {
    match name {
        "openai_responses" | "responses" => "openai_responses",
        "openai_chat" | "chat" => "openai_chat",
        "anthropic_messages" | "anthropic" => "anthropic_messages",
        "gemini_api" | "gemini" => "gemini_api",
        // 默认兜底：未识别的协议回退到 openai_chat
        _ => "openai_chat",
    }
}
