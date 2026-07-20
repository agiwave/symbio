use super::chat_message::ChatMessage;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub message: ChatMessage,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum StreamEvent {
    Update {
        message: ChatMessage,
    },
    /// 删除指定 id 的消息节点。
    ///
    /// 用于工具调用恢复时删除旧的 pending/failed 子节点
    /// （恢复后由新 id 的 Text 结果子节点替代）。
    Delete {
        message_id: String,
    },
    /// 业务级错误（可恢复，UI 应显示给用户）
    ///
    /// `error` 字段是面向用户的本地化短消息。
    Error {
        error: String,
    },
    Connected {
        session_id: String,
        is_working: bool,
        messages: Vec<ChatMessage>,
    },
    Disconnected,
    Status {
        status: String,
    },
    // V2.6: 新增业务级控制信令（替代原有的 PluginSignal）
    Abort,
    /// 子会话结束（正常完成或崩溃），用于激活等待该子会话的父会话。
    /// 父会话据此把 `result` 作为 agent_run 工具结果写回并继续下一轮。
    SessionResumed {
        /// 已结束的子会话 id
        session_id: String,
        /// 需要被激活的父会话 id
        parent_session_id: String,
        /// 子会话是否以失败/崩溃告终
        failed: bool,
        /// 子会话最终文本（failed 时为 None），供父会话作 tool result
        result: Option<String>,
    },
}
