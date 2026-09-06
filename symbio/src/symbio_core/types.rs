//! 核心类型定义 (V3.0)

use futures::Stream;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::pin::Pin;

/// 工具调用定义
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ToolCall {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    pub name: String,
    pub arguments: Value,
}

/// Boxed Stream 类型别名
pub type BoxStream<T> = Pin<Box<dyn Stream<Item = T> + Send>>;

// 系统事件类型

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SystemEvent {
    PreToolUse,
    PostToolUse,
    PreFileWrite,
    PostFileWrite,
    PreShellExecute,
    PostShellExecute,
    UserPromptSubmit,
    ToolApprovalRequired,
    SessionStart,
    SessionEnd,
    RequestStart,
    RequestComplete,
    Error,
    Notification,
}

impl SystemEvent {
    pub fn as_str(&self) -> &'static str {
        match self {
            SystemEvent::PreToolUse => "pre_tool_use",
            SystemEvent::PostToolUse => "post_tool_use",
            SystemEvent::PreFileWrite => "pre_file_write",
            SystemEvent::PostFileWrite => "post_file_write",
            SystemEvent::PreShellExecute => "pre_shell_execute",
            SystemEvent::PostShellExecute => "post_shell_execute",
            SystemEvent::UserPromptSubmit => "user_prompt_submit",
            SystemEvent::ToolApprovalRequired => "tool_approval_required",
            SystemEvent::SessionStart => "session_start",
            SystemEvent::SessionEnd => "session_end",
            SystemEvent::RequestStart => "request_start",
            SystemEvent::RequestComplete => "request_complete",
            SystemEvent::Error => "error",
            SystemEvent::Notification => "notification",
        }
    }
}

/// 事件触发结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventResult {
    pub success: bool,
    pub blocked: bool,
    pub block_reason: Option<String>,
    pub output: Option<String>,
    pub error: Option<String>,
}

impl EventResult {
    pub fn success() -> Self {
        Self {
            success: true,
            blocked: false,
            block_reason: None,
            output: None,
            error: None,
        }
    }
}
