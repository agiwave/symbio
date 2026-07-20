use serde::{Deserialize, Serialize};

/// Hook 事件枚举，用于在关键业务节点触发外部通知或拦截
///
/// 设计遵循行业最佳实践，参考了 LangChain、LlamaIndex 等主流框架的 hook 设计模式：
/// - 采用生命周期事件模式（Pre/Post）
/// - 支持同步拦截和异步通知
/// - 提供详细的上下文信息供外部处理
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", content = "data")]
pub enum HookEvent {
    /// 工具调用前事件 - 在执行工具调用前触发
    /// 可用于权限检查、审计日志、调用拦截等场景
    PreToolUse {
        tool_name: String,
        tool_input: serde_json::Value,
    },
    /// 工具调用成功后事件 - 在工具执行成功后触发
    /// 可用于结果记录、数据处理、后续工作流触发等
    PostToolUse {
        tool_name: String,
        tool_input: serde_json::Value,
        tool_output: serde_json::Value,
    },
    /// 工具调用失败事件 - 在工具执行失败时触发
    /// 可用于错误处理、告警通知、降级策略等
    PostToolUseFailure {
        tool_name: String,
        tool_input: serde_json::Value,
        error: String,
    },
    /// 通用通知事件 - 用于发送各类业务通知
    /// notification_type 可区分通知类型（如 info/warning/error 等）
    Notification {
        message: String,
        notification_type: String,
    },
    /// 用户提示提交事件 - 用户提交新消息时触发
    /// 可用于输入预处理、敏感词过滤、记录用户行为等
    UserPromptSubmit { prompt: String },
    /// 会话开始事件 - 新会话创建时触发
    /// source 表示会话来源，model 表示使用的模型
    SessionStart { source: String, model: String },
    /// 会话结束事件 - 会话正常结束时触发
    /// reason 说明结束原因（如用户主动结束、达到会话上限等）
    SessionEnd { reason: String },
    /// 对话停止事件 - 在 chat_loop 终止时触发（包括正常结束和异常中止）
    ///
    /// **触发场景**（参考 chat_loop.rs 中的调用点）：
    /// 1. 初始检查失败（如缺少必要上下文）
    /// 2. 检测到中止标志（用户主动取消）
    /// 3. 自动压缩过程中发生错误
    /// 4. 协议处理失败
    /// 5. 没有工具调用的单轮对话正常结束
    /// 6. 多轮工具调用完成后的最终结束
    ///
    /// last_message 表示最后一条有效消息内容，用于外部系统了解对话状态
    Stop { last_message: String },
    /// 子代理启动事件 - 启动子代理任务时触发
    /// 可用于追踪子任务状态、资源监控等
    SubagentStart { subagent_name: String, task: String },
    /// 子代理停止事件 - 子代理任务完成时触发
    /// result 包含子代理执行结果摘要
    SubagentStop {
        subagent_name: String,
        task: String,
        result: String,
    },
    /// 消息压缩前事件 - 在执行会话消息压缩前触发
    /// 可用于备份原始消息、记录压缩前状态等
    PreCompact,
    /// 消息压缩后事件 - 在会话消息压缩完成后触发
    /// 包含压缩前后的消息数量，便于评估压缩效果
    PostCompact {
        original_message_count: usize,
        compacted_message_count: usize,
    },
    /// 权限请求事件 - 需要用户授权时触发
    /// 用于实现工具调用的人工确认机制
    PermissionRequest { tool_name: String, reason: String },
    /// 停止失败事件 - 尝试停止对话但失败时触发
    /// 可用于错误告警、重试机制等
    StopFailure { error: String, error_type: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookOutput {
    pub should_proceed: bool,
    pub context: Option<String>,
    pub block_reason: Option<String>,
}

impl Default for HookOutput {
    fn default() -> Self {
        Self {
            should_proceed: true,
            context: None,
            block_reason: None,
        }
    }
}

impl HookEvent {
    pub fn event_name(&self) -> &'static str {
        match self {
            HookEvent::PreToolUse { .. } => "PreToolUse",
            HookEvent::PostToolUse { .. } => "PostToolUse",
            HookEvent::PostToolUseFailure { .. } => "PostToolUseFailure",
            HookEvent::Notification { .. } => "Notification",
            HookEvent::UserPromptSubmit { .. } => "UserPromptSubmit",
            HookEvent::SessionStart { .. } => "SessionStart",
            HookEvent::SessionEnd { .. } => "SessionEnd",
            HookEvent::Stop { .. } => "Stop",
            HookEvent::SubagentStart { .. } => "SubagentStart",
            HookEvent::SubagentStop { .. } => "SubagentStop",
            HookEvent::PreCompact => "PreCompact",
            HookEvent::PostCompact { .. } => "PostCompact",
            HookEvent::PermissionRequest { .. } => "PermissionRequest",
            HookEvent::StopFailure { .. } => "StopFailure",
        }
    }
}

impl HookOutput {
    pub fn allow() -> Self {
        Self {
            should_proceed: true,
            context: None,
            block_reason: None,
        }
    }

    pub fn deny(reason: impl Into<String>) -> Self {
        Self {
            should_proceed: false,
            context: None,
            block_reason: Some(reason.into()),
        }
    }

    pub fn with_context(mut self, ctx: impl Into<String>) -> Self {
        self.context = Some(ctx.into());
        self
    }

    pub fn is_blocking(&self) -> bool {
        !self.should_proceed || self.block_reason.is_some()
    }
}
