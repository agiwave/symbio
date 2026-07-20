// Corresponding Frontend: tauri/src/protocols/model_chat.ts
use crate::symbio_core::schemas::session::chat_message::{ChatMessage, ResumeRequest};
use serde::{Deserialize, Serialize};

/// 思考能力配置
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ThinkingConfig {
    pub enabled: bool,
    pub level: String,
    pub budget_tokens: u32,
}

/// Model 推理请求 (由 Session 插件或 Agent 发起)
///
/// 注意：工作区路径 (workdir) 由 PluginMessage.workdir 路由层统一传递，
/// 不在此业务结构体中重复定义。
///
/// 核心协议设计说明：
/// - 本协议采用"单消息输入"模式，不再携带完整会话历史
/// - 具体协议实现层（如 OpenAI Chat/Responses、Anthropic、Gemini）决定是否需要获取历史
/// - 有状态协议（如 OpenAI Responses）通过 previous_response_id 关联上下文
/// - 无状态协议（如标准 OpenAI Chat）需主动从会话服务获取历史消息
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Request {
    /// 系统提示词
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// 当前需要发送的单条消息
    #[serde(skip_serializing_if = "Option::is_none")]
    pub single_message: Option<ChatMessage>,
    /// 思考配置
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingConfig>,
    /// 是否使用流式输出
    pub stream: Option<bool>,

    /// 最大工具轮数
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_tool_rounds: Option<usize>,
    /// 工具上下文窗口（最近 N 轮工具调用保留明细）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_context_window: Option<usize>,
    /// 是否开启自动语义压缩
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_compress: Option<bool>,

    /// 指定本次会话使用的 Model Provider ID（来自 `ModelProvidersConfig.providers`）
    /// 为空时使用默认 Provider
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    /// 是否加载历史会话消息。
    /// - `None` / `Some(true)`：从会话存储加载历史（默认行为）
    /// - `Some(false)`：仅使用本次 `single_message`，不携带任何历史会话信息
    ///   （用于心跳任务等"无上下文"场景）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load_history: Option<bool>,
    /// 会话恢复操作（与 `single_message` 互斥）。
    ///
    /// 存在时由 `model/chat_loop.rs:run_chat_loop` 在 turn 循环前处理：
    /// - `RetryTurn`：删除 Failed Turn 及其所有子节点，重新走 LLM 请求
    /// - `Retry`/`Approve`/`Reject`/`Supply`/`Answer`：删除旧工具响应子节点 →
    ///   重新执行工具（approve/retry/supply）或直接生成结果（reject/answer）→
    ///   创建新响应子节点 → 成功则继续 turn 循环，失败则退出等下次 resume。
    ///
    /// CAPABILITY_MANAGER 已由 agent chat handler 设置，`execute_tool_async` 直接复用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<ResumeRequest>,
}
