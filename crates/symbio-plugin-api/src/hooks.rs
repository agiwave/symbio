use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::routes::ROUTE_HOOK_FIRE;
use symbio_kernel::plugin::{Plugin, PluginInvokeRequest, PluginInvokeRequestExt};
use symbio_sdk::keys::{PATH, SESSION_ID};

/// Hook 事件枚举，用于在关键业务节点触发外部通知或拦截
///
/// ## 哪些变体真的会响（`D-478`）
///
/// 14 个变体里**只有三个有产者**：`PreToolUse` / `PostToolUse`（工具端口执行体，调用前后）、
/// `PreCompact`（压缩端口执行体，压缩前）。三者都是**动作的前后沿**——它们发生之前没有任何
/// 格可以订阅。
///
/// 其余十一个**没有产者，且不补**：要通知的事情都已经是一条**事实**（逐个注明是哪条），
/// 订阅方读事实（名字经 `shared_events` 投影成生成常量）。同一件事两条通道正是漂移的来源。
/// 设计参考 LangChain / LlamaIndex 的生命周期模式（Pre/Post 成对、可拦截），但那里的事件
/// 没有事实面。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", content = "data")]
pub enum HookEvent {
    /// 工具调用前 - 可用于权限检查、审计日志、调用拦截
    PreToolUse {
        tool_name: String,
        tool_input: serde_json::Value,
    },
    /// 工具调用成功后 - 可用于结果记录、数据处理、后续工作流
    PostToolUse {
        tool_name: String,
        tool_input: serde_json::Value,
        tool_output: serde_json::Value,
    },
    /// 工具调用失败 - **无产者（`D-478`）**：失败是产物格的事实（`artifact.added` 带
    /// `success = false` 与 `failure_kind`）。
    PostToolUseFailure {
        tool_name: String,
        tool_input: serde_json::Value,
        error: String,
    },
    /// 通用通知 - **无产者（`D-478`）**：无业务语义；真通知该是自己的事实（带 owner 与格位），
    /// 不是一条通用字符串通道。
    Notification {
        message: String,
        notification_type: String,
    },
    /// 用户提示提交 - **无产者（`D-478`）**：用户提交就是 `user.message`（开轮的溯源也指向它）。
    UserPromptSubmit { prompt: String },
    /// 会话开始 - **无产者（`D-478`）**：由 `user.message` + `session.config`（本轮生效的
    /// 提示词 / 工具表 / 设置）表达。
    SessionStart { source: String, model: String },
    /// 会话结束 - **无产者（`D-478`）**：结束是 `session.terminated`（`(Scope, Closed)`），
    /// 收束另有 `session.settled`。
    SessionEnd { reason: String },
    /// 对话停止 - **无产者（`D-478`）**：一轮的停止由 `chat.assistant.final`（终答）与
    /// `session.settled` 表达；「为什么停」在驱动的返回里，不是一条通用字符串。
    Stop { last_message: String },
    /// 子代理启动 - **无产者（`D-478`）**：委托是一次工具调用（`agent_run`），开合落在
    /// `artifact.added`（带工具名、参数与结果）。
    SubagentStart { subagent_name: String, task: String },
    /// 子代理停止 - **无产者（`D-478`）**：同 `SubagentStart`（结果在 `tool = agent_run` 的
    /// 产物格，含 `success` / `text`）。
    SubagentStop {
        subagent_name: String,
        task: String,
        result: String,
    },
    /// 消息压缩前 - 可用于备份原始消息、记录压缩前状态
    PreCompact,
    /// 消息压缩后 - **无产者（`D-478`）**：压缩完成落 `session.consolidated`（`generation` /
    /// `fidelity` / `sources`）。「压缩前后消息数」是**读数**不是事件载荷：它属上下文管线的
    /// 统计，塞进钩子只会让订阅方按一个没有来源的数字评估压缩效果。
    PostCompact {
        original_message_count: usize,
        compacted_message_count: usize,
    },
    /// 权限请求 - **无产者（`D-478`）**：授权结论是事实——`authz.granted` / `authz.denied`
    /// 带工具名与拒因；「请求」那一步由会话的答复路径承载。
    PermissionRequest { tool_name: String, reason: String },
    /// 停止失败 - **无产者（`D-478`）**：驱动卡住落 `driver.stalled`（`D-393` 的安全阀出口，
    /// `Exhausted` / `Stalled` 共用一种形状）。
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
/// 触发点只有三处（`D-478`）：工具端口执行体在调用前后触发 `PreToolUse` / `PostToolUse`
///（DE-048 D-312），压缩端口执行体在压缩前触发 `PreCompact`。其余变体没有产者，
/// 它们要通知的事情已经是事实——订阅方读事实，不走第二条通道。各插件共用这一条
/// 路由助手（不是新机制）。
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
