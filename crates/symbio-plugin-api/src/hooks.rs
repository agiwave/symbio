use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::routes::ROUTE_HOOK_FIRE;
use symbio_core::{Plugin, PluginInvokeRequest, PluginInvokeRequestExt, PATH, SESSION_ID};

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
    /// 可用于追踪子任务状态、实体监控等
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

/// 钩子**没跑起来**时注入 `context` 的固定标记。
///
/// 钩子是旁路观察者，失败必须留在执行路径上（`should_proceed` 不变、调用点不中断），
/// 但「钩子跑了」与「钩子没跑」不能同形：装配错了挂钩插件的会话，此前看到的是
/// 一切正常——前置拦截静默失效。标记让这件事在返回值、日志与断言里都有名字。
pub const HOOK_UNAVAILABLE_MARKER: &str = "[hook-unavailable]";

/// 触发一次生命周期钩子，返回钩子插件的输出。
///
/// **返回 `HookOutput` 而非 `Result`，是刻意的**：钩子是**旁路观察者**——它的职责是
/// 「被通知到」，不是「否决主流程」。路由失败（没有钩子插件、插件未挂载、插件自己
/// 报错）在函数内部就吞成 `HookOutput::default()`，因此调用点的 `let _ = fire_hook(...)`
/// **丢的不是错误**（没有错误可丢），只是不需要那份输出。
///
/// [D-086](../../../docs/003-design/014-单环一表.md#d-086-loopsession-切片契约turndispatch-进程内交付turnspec-纯值面内部动作走地址面 "DE-014-单环一表") §6 上移宿主：session 在派发前
/// 触发 `UserPromptSubmit`，loop 在轮内触发 `PreCompact` / `PreToolUse` / `PostToolUse`，
/// 两个插件共用这一条路由助手（不是新机制）。
pub async fn fire_hook(
    parent: &Option<Arc<dyn Plugin>>,
    event: HookEvent,
    ctx: Arc<dyn PluginInvokeRequest>,
) -> HookOutput {
    let p = match parent {
        Some(p) => p,
        None => return HookOutput::default(),
    };

    let session_id = ctx.get(SESSION_ID).unwrap_or_default();

    let hook_ctx = ctx.fork();
    hook_ctx.set(PATH, ROUTE_HOOK_FIRE.to_string());
    let _ = hook_ctx.set_payload(serde_json::json!({
        "session_id": session_id,
        "event": serde_json::to_value(&event).unwrap_or(serde_json::json!({})),
    }));

    match p.clone().route(hook_ctx).await {
        Ok(resp) => match resp.get::<HookOutput>() {
            Ok(output) => output,
            // 插件在、但返回的东西不是钩子输出（版本不匹配 / 外部脚本吐了别的帧）：
            // 这与「没有钩子」同形，标记 + 告警把它分开。
            Err(e) => {
                tracing::warn!(
                    "钩子 {} 的输出无法解析：{e}（按未执行处理，不拦截）",
                    event.event_name()
                );
                HookOutput {
                    context: Some(format!(
                        "{HOOK_UNAVAILABLE_MARKER} {} 的输出无法解析：{e}",
                        event.event_name()
                    )),
                    ..Default::default()
                }
            }
        },
        // 插件缺失 / 路由失败：此前静默按「放行」处理，于是「钩子没跑」看起来像
        // 「钩子跑了且放行」。标记 + 告警让降级可观测，判定链保持不变。
        Err(e) => {
            tracing::warn!(
                "钩子 {} 执行失败：{e}（按未执行处理，不拦截）",
                event.event_name()
            );
            HookOutput {
                context: Some(format!(
                    "{HOOK_UNAVAILABLE_MARKER} {} 执行失败：{e}",
                    event.event_name()
                )),
                ..Default::default()
            }
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

#[cfg(test)]
#[path = "hooks.test.rs"]
mod tests;
