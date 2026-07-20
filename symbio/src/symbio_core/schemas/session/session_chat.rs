// Corresponding Frontend: tauri/src/protocols/chat_input.ts
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
    /// 随每次发送携带；为空时回退会话 metadata.mode（默认 interactive）。
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
    /// 为 None 且 `message` 存在时走正常 send 分支。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume: Option<ResumeRequest>,
}
