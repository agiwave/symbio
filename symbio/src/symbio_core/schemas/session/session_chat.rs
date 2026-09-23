//
// 注意：工作区路径 (workdir) 由 PluginMessage.workdir 路由层统一传递，
// 不在此业务结构体中重复定义。
use super::chat_message::{ChatMessage, ResumeRequest};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Request {
    pub session_id: Option<String>,
    pub agent_id: Option<String>,
    pub message: Option<ChatMessage>,
    /// 选定的 Model Provider ID；为 None 时使用默认 Provider
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    /// 心跳任务专用：本次发送是否携带历史会话信息。
    /// - `None` / `Some(true)`：携带历史（默认行为）
    /// - `Some(false)`：本次发送不加载历史会话信息
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_history: Option<bool>,
    /// 会话运行模式：auto（无人值守，需交互工具返回友好错误不弹框）/ interactive（默认，会话流中渲染交互卡）。
    /// 随每次发送携带；为空时回退会话 metadata.mode（默认 auto
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// 会话执行风险等级阈值：low / medium / high（与 agent_id/provider_id/mode 同级别）。
    /// 随每次发送携带；为空时回退会话 metadata.risk_level（默认 medium）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk_level: Option<String>,
    /// 会话恢复操作（与 `message` 互斥）。
    /// 存在时走 resume 分支：删除旧消息 → 重新执行 → 续写 chat_loop。
    /// 支持场景：
    /// - `RetryTurn`：LLM 失败重试（删除 Failed Turn 及子节点，重新走 LLM 请求）
    /// - `Retry`/`Approve`/`Reject`/`Supply`/`Answer`：工具调用恢复
    ///   为 None 且 `message` 存在时走正常 send 分支。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<ResumeRequest>,
}

/// `session/chat` 的响应：本轮定稿后的消息副本。
///
/// ## 这里曾经住着 `NodeOp` / `NodeChange`
///
/// 消息变更的线协议一度是独立的一层「显式操作」枚举（`upsert` / `append` /
/// `remove` / `reset` / `warn`），后来收敛成只剩一个变元的 `Change`。两者都已
/// 删除：**帧就是一条 [`ChatMessage`]**（见
/// [`crate::symbio_core::transcript_stream::NodeEvent`]）。
///
/// 原来那层枚举只是在重复 `ChatMessage` 已经有的字段：
///
/// | 原枚举里的字段 | 现在的落点 |
/// |---|---|
/// | `Change.message_id` | `ChatMessage.id` |
/// | `Change.delta` | `ChatMessage.delta`（增量；与 `content` 互斥） |
/// | `Change.status` / `error` / `meta` / 身份字段 | `ChatMessage` 的同名字段 |
/// | `Remove { message_id }` | `status = removed`（一次状态迁移） |
/// | `Reset` | 不存在——协议里没有「清空重读」的消息形态，删除逐条下发 |
/// | `Warn { warning }` | 会话节点状态（`TranscriptWriter::warn`），不是消息帧 |
///
/// 于是「消息变更」只剩一种形状：消费端不必再维护第二套解析与第二套合并规则，
/// 「这条消息现在是什么样」与「这帧说了什么」也不再是两份需要对齐的数据。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub message: ChatMessage,
}
