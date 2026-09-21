//! 会话编排器：请求装配 + RAII 守卫 + 消费循环的调度入口。
//!
//! 主链路：校验请求 → 预算限流（`rate_limit`）→ 构造 `ChatOrchestrator`（模型配置 /
//! 父插件钩子 / 协议适配器的 session 侧确定性持有）→ 交付 chat_ctx 并把后续轮次
//! 委托给 `chat_loop`。
//!
//! ## 模块分工
//!
//! | 文件 | 职责 |
//! |---|---|
//! | 本文件 | 装配 + RAII 守卫（`AiControlGuard` / `WorkingGuard`）+ `resolve_required_session_id` |
//! | [`broadcast`] | 三个广播出口：错误 + 收敛 / 帧投递 / 忙闲状态 |
//! | [`consume`] | 消费循环：`fail_before_loop` / `run_chat_loop_task` / `handle_abort` |
//! | [`entry`] | 两个 one-off 入口：`chat/send` / `chat/abort` + `ensure_auto_title` |
//! | [`failure`] | 失败降级持久化：`persist_failure` + `subtree_of` |
//!
//! `impl SessionPlugin` 按职责分块分散在子模块（Rust 允许多个 inherent impl，
//! 方法声明顺序无语义）。

use super::active::{ActiveSessionState, REQUEST_ID_COUNTER};
use super::chat_loop::StopSignal;
use super::chat_pipeline::{attach_capabilities, collect_capabilities};
use super::model_chat;
// 运行态的结局常量与变更类型：子模块经 `use super::*;` 取用（`consume` / `entry`
// / 本文件都要用），因此在这里导入一次，而不是各子模块各导一遍。
use super::plugin::{SessionPlugin, OUTCOME_ABORTED, OUTCOME_COMPLETED, OUTCOME_FAILED};
use crate::plugin_debug;
use crate::symbio_core::schemas::{
    session::chat_message as cm,
    session::{session_chat, session_chat_response},
};
use crate::symbio_core::{
    take_errors, InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginChannel,
    PluginError, PluginFrame, PluginPayload, MODE, PROVIDER_ID, RISK_LEVEL, SESSION_ID, WORKDIR,
};
use broadcast::SessionStateChange;
use serde_json::json;
use std::sync::atomic::Ordering;

use std::sync::Arc;
use std::time::Duration;

/// 解析**必填**会话 id：头 → 请求体 → 报错。
///
/// 统一此前散落在 send/resume/abort/heartbeat_trigger 各入口的重复特判。三档取值：
/// - `ctx` 头里的 `SESSION_ID`（前端 WS 帧注入，优先级最高）；
/// - `fallback`（请求体 `req.session_id`，RPC 直连调用方）；
/// - 空串或历史哨兵 `"default"` 一律视为缺失 → `ValidationError`。
///
/// **修复的行为**：abort / heartbeat_trigger 两个 one-off 入口此前不看请求体，
/// 只靠 `unwrap_or("default")` 再自我判定为非法，使 `req.session_id` 永远不可达。
/// `"default"` 作为非法值的理由并非"它是保留 id"，而是它曾是缺省占位符——
/// 拿它当真实会话去 abort/触发会静默作用于不存在的会话，宁可显式报错。
///
/// 注：`heartbeat_trigger` 入口已于 2026-09-18 整体取消（见
/// `docs/legacy-route-migration.md` §5.1），此处保留它是因为这条注释记录的是
/// **当时的修复范围**——把已删入口从历史里抹掉会让「为什么这个函数长这样」失去依据。
pub(crate) fn resolve_required_session_id(
    ctx: &Arc<dyn InvokeRequest>,
    fallback: Option<&str>,
) -> Result<String, PluginError> {
    let raw = ctx
        .get(SESSION_ID)
        .or_else(|| fallback.map(|s| s.to_string()));
    match raw {
        Some(id) if !id.is_empty() && id != "default" => Ok(id),
        _ => Err(PluginError::ValidationError("session_id 不能为空".into())),
    }
}

/// `ai_control_tx` 登记守卫：保证消费循环的**任何**出口都会清走登记的控制通道
/// sender——正常路径显式 `disarm`，panic unwind 由 `Drop` 兜底。
///
/// ## 为什么需要它
///
/// `handle_abort` 以「`ai_control_tx` 是否为 `None`」作为 chat_loop 子任务是否
/// 仍在运行的**唯一**判据。历史上消费循环有两处提前出口（业务 Error 帧、
/// 1800s 消费超时）写作 `return`，直接跳过了循环之后的清理块，留下指向已关闭
/// 通道的陈旧 sender——判据从此永久为假：abort 必然空等 3s 才走兜底复位，
/// 且 Abort 帧投进死通道被静默丢弃（前端收不到中止确认）。
///
/// 登记与注销收拢进同一对象后，"新增出口忘记清理"不再能静默通过：`Drop` 保证
/// 至少有一次清理必然发生，也不依赖后续维护者记住"新增出口必须穿过清理块"这条
/// 隐性契约。与 [`WorkingGuard`] 同型（后者兜 `is_working`）。
struct AiControlGuard {
    state: Arc<ActiveSessionState>,
    /// `false` 表示已清理，Drop 成为 no-op（正常路径走 `disarm` 同步清理，
    /// 避免多一次 spawn 调度延迟）。
    armed: bool,
}

impl AiControlGuard {
    async fn disarm(&mut self) {
        self.state.inner.write().await.ai_control_tx = None;
        self.armed = false;
    }
}

impl Drop for AiControlGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        // Drop 中不能 await：优先 `try_write` 就地同步清理（写锁持有期极短，
        // 几乎总能拿到）；确实取不到才退回 detached spawn（与 WorkingGuard 同型）。
        if let Ok(mut inner) = self.state.inner.try_write() {
            inner.ai_control_tx = None;
            return;
        }
        let state = self.state.clone();
        tokio::spawn(async move {
            state.inner.write().await.ai_control_tx = None;
        });
    }
}

/// 工作态守卫：保障 `is_working` 在任何退出路径（包括 panic 崩溃）下都会收敛。
///
/// ## 背景
///
/// spawn 任务若发生 **panic**，该任务会被 tokio 静默终止，`is_working`
/// 将永远停留在 `true`，导致前端永久显示"AI 处理中"且无法恢复。
///
/// ## 机制
///
/// - 在 spawn 任务开始时构造本守卫，`done` 初始为 `false`。
/// - **正常结束路径**在收尾前把 `done` 置 `true`，使 Drop 成为 no-op
///   （正常路径已自行 `is_working=false` + 广播 idle）。
/// - **panic 路径**：Rust unwind 会执行局部变量析构，`Drop` 被调用，
///   此时 `done==false`，守卫会 spawn 一个 detached 任务：
///   1. 若该 request_id 仍是当前活跃请求，把 `is_working` 复位；
///   2. 把仍处 streaming/pending 的 AI 消息标 `Failed` + 错误并持久化；
///   3. 广播 idle，使前端 UI 状态收敛。
struct WorkingGuard {
    state: Arc<ActiveSessionState>,
    plugin: Arc<SessionPlugin>,
    collected: Arc<tokio::sync::Mutex<crate::plugins::session::transcript::Transcript>>,
    /// 真实会话 id：`persist_failure` 需要它来定位并写入存储。
    /// 注意：绝不能传 `request_id` 的字符串——那是请求序号（如 "42"），
    /// 会令 `open_chat_session` 找不到会话而提前返回，导致崩溃失败永不落库。
    session_id: String,
    /// 本次请求的 Stop 触发器。
    ///
    /// 正常路径下 `run_chat_loop` 的出口已显式 fire 过，此处 Drop 是 no-op；
    /// panic 路径下它是**唯一**能保证 Stop 送达的机制（chat_loop 的出口代码
    /// 根本不会执行）。字段声明顺序决定析构顺序：本字段必须在 `done` 之前
    /// 落地，故放在结构体末尾即可（Drop 中显式调用，不依赖字段析构次序）。
    stop: Arc<StopSignal>,
    done: bool,
}

impl Drop for WorkingGuard {
    fn drop(&mut self) {
        // Stop 钩子兜底（幂等）：无论正常/panic/提前 return，本请求生命周期内
        // 恰好触发一次。放在 `done` 早退之前——正常结束路径同样依赖它兜住
        // "chat_loop 出口漏调"的情形。
        self.stop.fire_fallback();
        if self.done {
            return;
        }
        // panic / 异常退出路径：尽力重置状态并持久化失败，避免前端永久卡死。
        let state = self.state.clone();
        let plugin = self.plugin.clone();
        let collected = self.collected.clone();
        let session_id = self.session_id.clone();
        let crash_msg = "会话处理异常中断（后台任务崩溃），请重试".to_string();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                // 仅当本会话仍在进行中时才复位。
                // 注：旧的「同 request_id 才复位」检查依赖 ActiveSessionStateInner.request_id，
                // 字段已重构到 WorkingGuard.request_id（per-request）；
                // 此处直接靠 `is_working` 兜底：若本会话已被新一轮请求接管，
                // 持久化失败会自然被 persist_failure 内部的版本检查拦截。
                {
                    let mut inner = state.inner.write().await;
                    if inner.is_working {
                        inner.is_working = false;
                    }
                }
                // 把"仍在进行中"的 AI 消息持久化为 Failed + 错误原因
                // （切回会话时能看到上次失败的终态，目标 3）。
                plugin
                    .persist_failure(
                        &session_id,
                        &collected,
                        &crash_msg,
                        cm::MessageStatus::Failed,
                    )
                    .await;
                // 运行态收敛为「以错误结束」：`status = failed` + `attributes.error`
                // 随节点视图一并下发，前端因此不需要"事件 + 启发式"就能显示错误条；
                // 这也就是「崩溃后 UI 立即看到错误」的全部机制（没有第二条 Error 事件）。
                plugin
                    .emit_session_state(
                        &state,
                        SessionStateChange::Finished {
                            outcome: OUTCOME_FAILED,
                            error: Some(crash_msg),
                        },
                    )
                    .await;
            });
        }
    }
}

// 子模块：`impl SessionPlugin` 按职责分块（Rust 允许多个 inherent impl，方法声明顺序无语义）。
mod broadcast;
mod consume;
mod entry;
mod failure;

#[cfg(test)]
#[path = "orchestrator.test.rs"]
mod tests;
