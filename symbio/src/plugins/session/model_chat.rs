// Corresponding Frontend: tauri/src/protocols/model_chat.ts
use crate::symbio_core::schemas::session::chat_message::{ChatMessage, ResumeRequest};
use serde::{Deserialize, Serialize};

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
///
/// 关于 `Option` 与序列化（契约，2026-09-14 归一）：
/// - 字段保持 `Option` 是**契约要求**：`None`（调用方未表态）与"零值"语义不同
///   （如 `auto_compress: None` → chat_loop 取默认 `true`；`max_tool_rounds: None` → 无限轮次）。
///   因此**不得**改成带 serde default 的裸类型，否则缺省会翻转为零值语义。
/// - 输出**不使用** `skip_serializing_if`：`None` 序列化为 `null`，使键集合恒定存在，
///   不随字段取值增删（历史上 `stream` 无该属性、其余字段有，导致同一协议在不同取值下
///   键集不同，消费方难以稳定判别"未设置"）。
/// - 输入侧 `Option` 字段 serde 天然接受"缺键"与"`null`"，二者等价，无需逐字段 `default`。
#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Request {
    /// 系统提示词
    pub system_prompt: Option<String>,
    /// 当前需要发送的单条消息
    pub single_message: Option<ChatMessage>,
    /// 是否使用流式输出
    pub stream: Option<bool>,

    /// 最大工具轮数
    pub max_tool_rounds: Option<usize>,
    /// 工具上下文窗口（最近 N 轮工具调用保留明细）
    pub tool_context_window: Option<usize>,
    /// 是否开启自动语义压缩
    pub auto_compress: Option<bool>,
    /// 是否启用工具压缩（context_compact 暴露给模型 + 水位提醒；独立于 auto_compress）
    pub enable_compact_tool: Option<bool>,

    /// 指定本次会话使用的 Model Provider ID（来自 `ModelProvidersConfig.providers`）
    /// 由 session 编排层解析（精确 id → is_default → 首个注册），为空时同样走回退链取默认 Provider
    pub provider_id: Option<String>,
    /// 是否加载历史会话消息。
    /// - `None` / `Some(true)`：从会话存储加载历史（默认行为）
    /// - `Some(false)`：仅使用本次 `single_message`，不携带任何历史会话信息
    ///   （用于心跳任务等"无上下文"场景）
    pub load_history: Option<bool>,
    /// 会话恢复操作（与 `single_message` 互斥）。
    ///
    /// 存在时由 session 插件会话循环（`session/chat_loop.rs:run_chat_loop`）在 turn 循环前处理：
    /// - `RetryTurn`：删除 Failed Turn 及其所有子节点，重新走 LLM 请求
    /// - `Retry`/`Approve`/`Reject`/`Supply`/`Answer`：删除旧工具响应子节点 →
    ///   重新执行工具（approve/retry/supply）或直接生成结果（reject/answer）→
    ///   创建新响应子节点 → 成功则继续 turn 循环，失败则退出等下次 resume。
    ///
    /// CAPABILITY_VISITOR 已由 agent chat handler 设置，`execute_tool_async` 直接复用。
    pub resume: Option<ResumeRequest>,
}

#[cfg(test)]
mod tests;
