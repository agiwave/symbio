//! 核心 ModelProvider —— 唯一模型契约（纯 object-safe trait）。
//!
//! 本模块只暴露**纯 trait** `ModelProvider`，作为 session 会话引擎与 model
//! 插件之间唯一的模型契约：
//! - session 只依赖本 trait（经 `Arc<dyn ModelProvider>` 持有），对协议适配
//!   细节零感知；
//! - 协议适配（`ModelProtocol` 钩子 trait、`resolve_protocol_id`、
//!   `MODEL_PROTOCOL_*` 注册常量、`ReasoningConfig`、协议事件方言
//!   `ModelProtocolEvent`、行解析契约 `SseLineParser`）定义在 model 插件内部
//!   （`plugins/model/protocols/`），core 不暴露这些类型；
//! - model 插件的 `BoundProvider`（持久化配置 + 协议钩子实现的绑定）是本
//!   trait 的生产实现：`execute_turn` 五态机、`effective_context_tokens`
//!   收敛、ping 等完整行为均由其提供。
//!
//! ## 依赖方对照表（ADR-023 决策 2）
//!
//! | 符号 | 谁依赖 | 依赖什么 |
//! |---|---|---|
//! | [`ModelProvider`] | session（`chat_loop` 经 `Arc<dyn ModelProvider>` 驱动）· model（`BoundProvider` 实现） | `execute_turn` / `effective_context_tokens` / 身份与限流方法 |
//! | [`ModelFinishReason`] | session（区分「自然结束」与 `max_tokens` 截断）· model（`TurnOutput::finish` 的赋值方） | `from_provider` / `is_length` |
//! | [`ModelUsage`] | model（上报）· session（校准 token 估算） | 字段读取 |
//!
//! 三个符号都是**会话引擎 ↔ 模型**两侧共用的，故留在本层；同一文件里
//! 只有一侧认的协议词汇（事件方言、行解析契约）已随之迁出，见上一条。
//!
//! 依赖关系：
//! - model 插件在 traverse 中按上下文（用户选中的模型）注册唯一生效的
//!   `Arc<dyn ModelProvider>` 进 `CAPABILITY_VISITOR`；
//! - session 从 `CapabilityVisitor` 直接取得该实例并驱动对话；
//! - 上下文窗口等参数经 trait 方法自含地暴露（`max_context_tokens` /
//!   `effective_context_tokens`）。

use crate::symbio_core::{CapabilityMeta, ExecEnv, PluginError};
use async_trait::async_trait;

use super::turn::TurnOutput;
use crate::symbio_core::schemas::session::chat_message::ChatMessage;

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
/// 只有拿到 `ModelFinishReason`，`chat_loop` 才能区分"自然结束"与"被长度截断"，并触发自动续写或明确报错。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ModelFinishReason {
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

impl ModelFinishReason {
    /// 从 provider 的原始字符串归一化。`None` 视为自然结束。
    pub fn from_provider(raw: Option<&str>) -> Self {
        match raw.map(|s| s.trim()).unwrap_or("") {
            "" | "stop" | "end_turn" | "STOP" | "stop_sequence" | "completed" => {
                ModelFinishReason::Stop
            }
            "length" | "max_tokens" | "MAX_TOKENS" | "max_output_tokens" | "incomplete" => {
                ModelFinishReason::Length
            }
            "tool_calls" | "tool_use" | "function_call" => ModelFinishReason::ToolCalls,
            "content_filter" | "SAFETY" | "RECITATION" | "refusal" => {
                ModelFinishReason::ContentFilter
            }
            other => ModelFinishReason::Other(other.to_string()),
        }
    }

    pub fn is_length(&self) -> bool {
        matches!(self, ModelFinishReason::Length)
    }
}

/// 单次请求的用量（用于校准 token 估算；provider 不一定给，故全部可选）
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ModelUsage {
    pub input: Option<u32>,
    pub output: Option<u32>,
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
    ///
    /// `env` 与 [`crate::symbio_core::Capability::execute`] 共用同一类型
    /// （[`ExecEnv`]）：两者都是「一次带中止的流式执行」——出方向写事件、
    /// 入方向读中止。模型执行不被路由，因此没有 `ctx` 入参。
    async fn execute_turn(
        &self,
        system_prompt: &str,
        messages: &[ChatMessage],
        tools: &[CapabilityMeta],
        root_id: &str,
        env: &ExecEnv,
    ) -> Result<TurnOutput, PluginError>;
}
