//! 核心 ModelProvider —— 唯一模型契约（纯 object-safe trait）。
//!
//! 本模块只暴露**纯 trait** `ModelProvider`，作为 session 会话引擎与 model
//! 插件之间唯一的模型契约：
//! - session 只依赖本 trait（经 `Arc<dyn ModelProvider>` 持有），对协议适配
//!   细节零感知；
//! - 协议适配（`ModelProtocol` 钩子 trait、`resolve_protocol_id`、
//!   `MODEL_PROTOCOL_*` 注册常量、`ReasoningConfig`）定义在 model 插件内部
//!   （`plugins/model/protocols/`），core 不暴露这些类型；
//! - model 插件的 `BoundProvider`（持久化配置 + 协议钩子实现的绑定）是本
//!   trait 的生产实现：`execute_turn` 五态机、`effective_context_tokens`
//!   收敛、ping 等完整行为均由其提供。
//!
//! 依赖关系：
//! - model 插件在 traverse 中按上下文（用户选中的模型）注册唯一生效的
//!   `Arc<dyn ModelProvider>` 进 `CAPABILITY_VISITOR`；
//! - session 从 `CapabilityVisitor` 直接取得该实例并驱动对话；
//! - 上下文窗口等参数经 trait 方法自含地暴露（`max_context_tokens` /
//!   `effective_context_tokens`）。

use crate::symbio_core::CapabilityMeta;
use async_trait::async_trait;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use super::turn::TurnOutput;
use crate::symbio_core::schemas::session::chat_message::ChatMessage;
use crate::symbio_core::{PluginChannel, PluginError};

/// 流结束原因（由各协议的 `finish_reason` / `stop_reason` / `finishReason` 归一化）
///
/// ## 为什么必须有它
///
/// 若不区分结束原因，流结束一律被当成"正常完成"。当模型因 `max_tokens` 用尽而停在
/// `Length` 时：
/// 1. 断在半句的文本会被当完整回复呈现；
/// 2. 若截断发生在 `tool_calls` 的参数 JSON 中间，工具调用永远收集不完 →
///    `tools_done` 为空 → 循环按"无工具调用"正常退出——**用户看到的就是"对话突然结束"**。
///
/// 只有拿到 `FinishReason`，`chat_loop` 才能区分"自然结束"与"被长度截断"，并触发自动续写或明确报错。
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

/// MODEL Provider —— 唯一模型契约（纯 object-safe trait）
///
/// 会话引擎（session）只依赖本 trait；实现体（model 插件的 `BoundProvider`：
/// 持久化配置 `ModelProviderConfig` + 协议钩子实现 `Arc<dyn ModelProtocol>`
/// 的绑定）经 `CAPABILITY_VISITOR` 注册唯一生效实例。
///
/// trait 只暴露 session 需要的最小面：
/// - **身份/参数**：`provider_id`（限流器与日志归组键）、`api_protocol`
///   （原始协议配置串）、`rate_limit_ms`、`max_context_tokens`；
/// - **上下文收敛**：`effective_context_tokens`（服务端上报值与用户设置取 min）；
/// - **单轮执行**：`execute_turn`（构造请求 → 带中止的 POST → SSE 流解析 →
///   `TurnOutput`），协议差异由实现体内部的协议钩子吸收。
#[async_trait]
pub trait ModelProvider: Send + Sync {
    /// Provider 唯一标识（注册表键；限流器与日志按此归组）
    fn provider_id(&self) -> &str;

    /// 原始协议配置串（用户可写别名，如 `openai_responses`）
    fn api_protocol(&self) -> &str;

    /// 请求间隔限流（毫秒；0 表示不限）
    fn rate_limit_ms(&self) -> u64;

    /// 用户设置的上下文窗口上限（生效值见 [`Self::effective_context_tokens`]）
    fn max_context_tokens(&self) -> u32;

    /// 计算生效上下文 token 上限：`min(用户设置, 服务上报)`。
    ///
    /// 服务端能主动上报上限时（本地模型常见）取两者较小值，避免请求超限；
    /// 否则（云端 API 普遍不暴露）直接使用用户设置。
    async fn effective_context_tokens(&self) -> u32;

    /// 执行完整单轮 LLM 请求：构造请求体 → 带中止的 POST → SSE 流解析。
    ///
    /// Phase E 的统一入口。五态匹配（`Aborted` / `RetryWithoutContextId` /
    /// `Err` / `RateLimited` / `Ok`）与 SSE 解析由实现体提供，使任何
    /// `ModelProvider` 实例都自带完整的「请求-解析」能力，会话引擎无需再
    /// 依赖 model 插件内部实现。
    #[allow(clippy::too_many_arguments)]
    async fn execute_turn(
        &self,
        system_prompt: &str,
        messages: &[ChatMessage],
        tools: &[CapabilityMeta],
        root_id: &str,
        channel: &mut PluginChannel,
        abort_flag: &Arc<AtomicBool>,
    ) -> Result<TurnOutput, PluginError>;
}
