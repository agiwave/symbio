//! 核心 ModelProvider —— 唯一模型契约：模型参数自含 + 协议适配钩子。
//!
//! 本模块是「ModelProvider 类型体系合并」改造的产物：
//! - 原 `ModelProvider` trait（协议钩子契约）与 `ModelProviderEntry`
//!   （注册条目）合并为唯一的 `ModelProvider` **具体结构体**；
//! - 模型参数（原 `schemas/model/model_config.rs` 的 `ModelConfig`）直接
//!   内嵌为结构体字段——含 `max_context_tokens` 等计算参数，该配置文件
//!   随之删除；
//! - 原协议 trait 更名 `ModelProtocol`：纯钩子（URL/请求头/请求体/行解析/
//!   ping/上下文上限查询），全部只接收 `&ModelProvider`，实现体保留在
//!   model 插件 `protocols/` 目录下。
//!
//! 依赖关系（用户需求 ①）：
//! - model 插件在 traverse 中按上下文（用户选中的模型）注册唯一生效的
//!   `ModelProvider` 进 `CAPABILITY_VISITOR`（需求 ②）；
//! - session 从 `CapabilityVisitor` 直接取得该实例并驱动对话（需求 ③）；
//! - 上下文窗口等参数由 `ModelProvider` 自含并计算（需求 ④）。

use crate::plugin_warn;
use crate::symbio_core::CapabilityMeta;
use async_trait::async_trait;
use reqwest::header::HeaderMap;
use serde::{Deserialize, Serialize};
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

/// 推理配置（自 `schemas/model/model_config.rs` 迁入——该文件已随本次合并删除）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReasoningConfig {
    pub effort: String,
}

/// MODEL 协议特质 —— 抽象不同模型提供商的通信细节（纯钩子）
///
/// 命名说明：原核心 trait 名为 `ModelProvider`，与合并后的唯一契约结构体
/// 重名，故更名为 `ModelProtocol`。实现体保留在 model 插件的 `protocols/`
/// 目录下，各自提供 `build` 函数并经 `submit_object_creator!` 自动注册；
/// 调用方使用 `create_object::<dyn ModelProtocol>(protocol_id, ctx)` 取得实例。
///
/// 所有钩子只接收 `&ModelProvider`——模型参数由参数对象自含，
/// 不再存在独立的 `ModelConfig`。
///
/// 已注册 id（参见各协议文件）：
/// - `openai_responses`（别名 `responses`）— openai_responses.rs
/// - `openai_chat`（别名 `chat`）— openai_chat.rs
/// - `anthropic_messages`（别名 `anthropic`）— anthropic_messages.rs
/// - `gemini_api`（别名 `gemini`）— gemini_api.rs
#[async_trait]
pub trait ModelProtocol: Send + Sync {
    /// 获取 API 请求端点 URL
    fn get_api_url(&self, provider: &ModelProvider) -> String;

    /// 获取 HTTP 请求头
    fn get_headers(&self, provider: &ModelProvider) -> HeaderMap;

    /// 构造请求体 JSON
    fn prepare_request(
        &self,
        provider: &ModelProvider,
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
    async fn ping(&self, provider: &ModelProvider) -> Result<(), PluginError> {
        let _ = provider;
        // fail-closed：协议未显式实现 ping 时，验证必须显式失败而非静默放行
        Err(PluginError::InternalError(
            "protocol does not implement ping".to_string(),
        ))
    }

    /// 查询模型服务上报的最大上下文 token 数（尽力而为）。
    ///
    /// 仅当服务端能主动提供该信息时返回 `Some(limit)`（如 Ollama `/api/show`、
    /// LM Studio `/api/v0/models`、Gemini ListModels 的 `inputTokenLimit`）；
    /// 云端 API 普遍不暴露此信息，默认返回 `None`（调用方仅使用用户设置）。
    ///
    /// 调用方经 [`ModelProvider::effective_context_tokens`] 用它对用户设置的
    /// `max_context_tokens` 做 `min(用户设置, 服务上报)` 收敛，避免本地模型
    /// （num_ctx 通常远小于模型训练窗口）因请求超限而报错。
    async fn query_context_limit(&self, provider: &ModelProvider) -> Option<u32> {
        let _ = provider;
        None
    }

    /// 解析流的一行内容（处理 SSE 协议）
    fn parse_response_line(&self, line: &str) -> Vec<ProtocolEvent>;
}

/// MODEL Provider —— 唯一模型契约（合并原 `ModelProvider` trait 与 `ModelProviderEntry`）
///
/// 结构：
/// - **身份**：`provider_id` / `protocol_id` / `system_prompt` / `rate_limit_ms`；
/// - **模型参数**：自 `ModelConfig` 迁入（JSON 字段名冻结于 model 插件的
///   `ModelProviderConfig`，用户配置文件兼容）；
/// - **协议适配器**：`protocol: Arc<dyn ModelProtocol>`——纯钩子实现，
///   `execute_turn` 等完整行为由本结构体的固有方法提供。
///
/// 会话引擎（session）只依赖本结构体；model 插件负责按上下文构造并注册
/// 唯一生效实例。
#[derive(Clone)]
pub struct ModelProvider {
    // —— 身份 ——
    /// Provider 唯一标识（注册表键；限流器与日志按此归组）
    pub provider_id: String,
    /// 已解析的协议 id（`resolve_protocol_id` 产物，如 `openai_responses`）
    pub protocol_id: String,
    /// 该 Provider 的系统提示词（可选；同时注册于其 id 键与 "default" 键）
    pub system_prompt: Option<String>,
    /// 请求间隔限流（毫秒；0 表示不限）
    pub rate_limit_ms: u64,

    // —— 模型参数（自 ModelConfig 迁入）——
    /// 供应商标识 (openai, anthropic, lmstudio, ollama 等)
    pub provider: String,
    pub api_base: String,
    pub api_key: Option<String>,
    pub model: String,
    pub temperature: f64,
    pub max_tokens: Option<u32>,
    /// 用户设置的上下文窗口上限（默认 256k；生效值见 [`Self::effective_context_tokens`]）
    pub max_context_tokens: u32,
    /// 上下文中为系统保留的 token 数（压缩触发阈值 = 生效上限 − 本值）
    pub reserved_tokens: u32,
    pub timeout_secs: u64,
    /// 原始协议配置串（用户可写别名；已解析形式见 `protocol_id`）
    pub api_protocol: String,
    /// OpenAI Responses API 的 store 透传
    pub store: bool,
    pub reasoning: Option<ReasoningConfig>,

    // —— 协议适配器 ——
    pub protocol: Arc<dyn ModelProtocol>,
}

impl ModelProvider {
    /// 计算生效上下文 token 上限：`min(用户设置, 服务上报)`。
    ///
    /// 服务端能主动上报上限时（本地模型常见）取两者较小值，避免请求超限；
    /// 否则（云端 API 普遍不暴露）直接使用用户设置。
    pub async fn effective_context_tokens(&self) -> u32 {
        match self.protocol.query_context_limit(self).await {
            Some(limit) => self.max_context_tokens.min(limit),
            None => self.max_context_tokens,
        }
    }

    // —— 协议钩子委托（便捷封装：参数对象自含，调用方无需再传参数）——

    /// 获取 API 请求端点 URL（委托 `protocol.get_api_url`）
    pub fn get_api_url(&self) -> String {
        self.protocol.get_api_url(self)
    }

    /// 获取 HTTP 请求头（委托 `protocol.get_headers`）
    pub fn get_headers(&self) -> HeaderMap {
        self.protocol.get_headers(self)
    }

    /// 构造请求体 JSON（委托 `protocol.prepare_request`）
    pub fn prepare_request(
        &self,
        system_prompt: &str,
        messages: &[ChatMessage],
        tools: &[CapabilityMeta],
    ) -> Value {
        self.protocol
            .prepare_request(self, system_prompt, messages, tools)
    }

    /// 解析流的一行内容（委托 `protocol.parse_response_line`）
    pub fn parse_response_line(&self, line: &str) -> Vec<ProtocolEvent> {
        self.protocol.parse_response_line(line)
    }

    /// 连通性验证（委托 `protocol.ping`；未实现时 fail-closed 返回 Err）
    pub async fn ping(&self) -> Result<(), PluginError> {
        self.protocol.ping(self).await
    }

    /// 查询模型服务上报的最大上下文 token 数（委托 `protocol.query_context_limit`）
    pub async fn query_context_limit(&self) -> Option<u32> {
        self.protocol.query_context_limit(self).await
    }

    /// 执行完整单轮 LLM 请求：构造请求体 → 带中止的 POST → SSE 流解析。
    ///
    /// Phase E 的统一入口。把原先散落在 model 插件 `turn_processor::send_request`
    /// 的五态匹配（`Aborted` / `RetryWithoutContextId` / `Err` / `RateLimited` / `Ok`）
    /// 与 SSE 解析收编到 core，使任何 `ModelProvider` 实例都自带完整的
    /// 「请求-解析」能力，会话引擎无需再依赖 model 插件内部实现。
    ///
    /// 实现对所有协议通用：协议差异已被 `ModelProtocol` 的四个钩子吸收，
    /// 故无需（也不可）按协议覆写。
    #[allow(clippy::too_many_arguments)]
    pub async fn execute_turn(
        &self,
        system_prompt: &str,
        messages: &[ChatMessage],
        tools: &[CapabilityMeta],
        root_id: &str,
        channel: &mut PluginChannel,
        abort_flag: &Arc<AtomicBool>,
    ) -> Result<TurnOutput, PluginError> {
        let body = self.prepare_request(system_prompt, messages, tools);

        let response = match execute_post_with_abort(
            &self.get_api_url(),
            self.get_headers(),
            &body,
            channel,
            abort_flag,
        )
        .await
        {
            PostResult::Aborted => {
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
                return Err(PluginError::InternalError(msg));
            }
            PostResult::RateLimited(msg) => {
                return Err(PluginError::RateLimited(msg));
            }
            PostResult::Ok(resp) => resp,
        };

        match parse_sse_stream(response, root_id, channel, abort_flag, self.protocol.as_ref()).await
        {
            Err(msg) => Err(PluginError::StreamError(msg)),
            Ok(out) => Ok(out),
        }
    }
}

/// 将 `api_protocol` 别名解析为已注册的协议 id
///
/// 未识别的协议回退到 `openai_chat`（与原 `ModelConfig.api_protocol`
/// 默认值 `openai_responses` 的关系：默认值仅作用于未配置场景，
/// 解析失败兜底沿用既有行为）。
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
