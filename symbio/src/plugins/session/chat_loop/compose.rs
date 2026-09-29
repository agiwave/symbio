//! 对话面措辞的调用点 + **落点** —— `session` ⇄ `reply` 的那条边。
//!
//! ## 它做三件事
//!
//! 1. 把**对话线**投影出来（与 `decide.rs` 同一个函数、同一个窗口）；
//! 2. 经容器 `route` 调 `reply/compose`（`ctx.fork()` + `PATH` 常量 + 契约载荷）；
//! 3. 把返回的文本**写进转写**——唯一写入者仍是 `session`（ADR-020）。
//!    `reply` 只返回一个 `String`，一个节点都不写。
//!
//! ## 落点：**根级**，不是某个 Turn 之下
//!
//! 首响 / 答话属于**对话线**（用户真的看到过的话），因此 `parent_id = None`：
//! 它在会话根上与用户消息互为兄弟，正是 [`conversation_view`] 认的形状。
//! 挂在 Turn 之下会让它变成工作线的一轮——前端「对话」面板看不到它，
//! 而下一轮的 `reply` 也就不知道上一轮说过什么（措辞会重复或断裂）。
//!
//! ## 两个标记回答两个问题，**不合并**
//!
//! | 产物 | `meta.surface` | `meta.exclude_from_context` |
//! |---|---|---|
//! | `Answered` 的答话 | `"reply"` | **不设** —— 它就是这一轮的答复，是对话内容 |
//! | `Escalate` 的首响 | `"reply"` | `true` —— 界面开场白，不是模型的对话内容 |
//! | `Report` 的汇报 | `"reply"` | `true` —— 进度是给用户看的，不是模型的对话内容 |
//!
//! 首响与汇报必须剔除，硬理由有两条：① 它们是**面向用户**的界面文本，不是"模型的
//! 对话历史"；② 首响紧跟用户消息，若进请求包，线上会出现连续两条 `assistant`
//! （Anthropic 一类协议要求交替，会直接 400）。剔除点在 model 插件
//! （`message_builder` 的 `flatten_chat_messages`），与 `compression` 节点同层处置。
//!
//! 而 [`conversation_view`] **不看** `exclude_from_context`——两个过滤器各管一条线，
//! 详见该函数的模块文档。
//!
//! ## 措辞拿不到时**降级进工具循环**，不沉默
//!
//! `Answered` 而措辞为空（未挂载 `reply` / `reply_enabled = false` / 连兜底模板都取不到）
//! ⇒ 本轮**照旧进工具循环**。理由与 `triage` 分类失败的兜底方向是同一条：
//! **沉默是这里最坏的失败形态**（用户什么都收不到，且没有任何错误信号），
//! 而进工具循环退化成"引入判决之前的行为"——慢一点，但有答案。
//!
//! `Escalate` 拿不到首响则只是"少一句开场白"，本轮照旧干活——**不降级**（本来就要干活）。
//! `Report` 拿不到措辞则只是"这次没说"，且**不消耗汇报配额**（见 `progress.rs`）——
//! 下一次轮边界会再试一次。
//!
//! ## 返回值：**说没说**与**收不收尾**是两个问题
//!
//! [`VerdictEffect`] 把它们分开编码，而不是一个 `bool`。两个 `bool` 会多出一个
//! 不可达组合（"收尾了但没说话"不存在），而一个 `bool` 根本答不了"说没说"——
//! 那正是中途汇报要读的量（说了一句才消耗配额）。

use std::sync::Arc;

use crate::symbio_core::schemas::dialog::{ComposeRequest, RunSnapshot, Verdict};
use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
};
use crate::symbio_core::{
    clock_now_ms, llm_short_id, PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PATH,
    ROUTE_REPLY_COMPOSE, SESSION_ID,
};

use super::super::context::{conversation_view, CONVERSATION_VIEW_LIMIT};
use super::state::{ChatOrchestrator, SessionContext};

/// `meta.surface` 的取值：本节点由**措辞**能力产出。
///
/// 它答的问题是"谁产出的"（供前端按来源轻量渲染），与"模型该不该看到"
/// （`exclude_from_context`）是两件事。
const SURFACE_REPLY: &str = "reply";

/// 汇报节点的理由码（`meta.reason`）。
///
/// ## 为什么它由 `session` 拥有，而不是 `reply` 的抄本
///
/// `reply/reasons.rs` 与 `triage/reasons.rs` 是同一份词汇表的**两份抄本**，因为
/// 生产方与消费方分处两个插件、不能共享常量。而汇报的理由码是**本侧自己产的**：
/// 判决 `Report` 由 `session` 判出（见 `schemas/dialog.rs` 的变体表），措辞只是执行它
/// ——`reply` 按判决分派，不看这个码。因此它没有"第二份抄本"可漂移，就地定义。
const REASON_PROGRESS: &str = "progress";

/// 判决的**执行点**：措辞 + 落点。
///
/// `snapshot` 是**运行现状**（只有 `Report` 读它，见 [`RunSnapshot`]）。轮首的两个变体
/// 不看运行现状——它们说的是"这一轮怎么办"，而现状说的是"干到哪一步了"。
///
/// 收尾动作由调用方做（`finish_turn` 是主循环的唯一收尾点）——本函数只负责
/// "说什么"与"写下来"，不负责"怎么结束"。
pub(crate) async fn apply_verdict(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn PluginInvokeRequest>,
    context: &mut SessionContext,
    verdict: Verdict,
    snapshot: &RunSnapshot,
) -> VerdictEffect {
    // 调用方关掉了措辞（或压根没挂 `reply`）⇒ 与"未挂载 reply"逐字一致：
    // `Answered` 降级进工具循环，`Escalate` 没有首响，`Report` 什么都没说。
    // 三条都在下面自然成立。
    if !orchestrator.reply_enabled {
        crate::plugin_debug!("session", "[Reply] 措辞未启用，本轮不产出对话面文本");
        return VerdictEffect::Silent;
    }

    match &verdict {
        Verdict::Answered { reason } => {
            let Some(text) = compose_text(orchestrator, ctx, context, &verdict, snapshot).await
            else {
                // 判决说能直接答，但没人能说话 ⇒ **降级进工具循环**（不沉默）。
                crate::plugin_warn!(
                    "session",
                    "[Reply] 判决为 Answered（reason={reason}）但取不到措辞，本轮降级进工具循环"
                );
                return VerdictEffect::Silent;
            };
            context.messages.push(dialog_node(&text, reason, false));
            VerdictEffect::Finish
        }
        Verdict::Escalate { reason } => {
            // 首响：拿不到就只是少一句开场白，本轮照旧干活。
            match compose_text(orchestrator, ctx, context, &verdict, snapshot).await {
                Some(text) => {
                    context.messages.push(dialog_node(&text, reason, true));
                    VerdictEffect::Spoke
                }
                None => VerdictEffect::Silent,
            }
        }
        // 汇报：措辞从**运行现状**组织（`reply` 的模板产线，零 LLM 往返）。
        // 拿不到就不说——本轮照旧干活，且不消耗配额（调用方按 [`VerdictEffect`] 判）。
        Verdict::Report => {
            match compose_text(orchestrator, ctx, context, &verdict, snapshot).await {
                Some(text) => {
                    context
                        .messages
                        .push(dialog_node(&text, REASON_PROGRESS, true));
                    VerdictEffect::Spoke
                }
                None => VerdictEffect::Silent,
            }
        }
    }
}

/// [`apply_verdict`] 的产物：**说没说**一句话，以及**要不要收尾**。
///
/// 合成一个枚举而不是两个 `bool`：`Finish` 必然包含"说了话"（收尾的前提就是答话
/// 已写下），两个 `bool` 会多出一个不可达组合。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VerdictEffect {
    /// `Answered` 且答话已写下：本轮**到此收尾**（调用方据此 `finish_turn`）。
    Finish,
    /// 说了一句话，本轮继续（`Escalate` 首响 / `Report` 汇报）。
    Spoke,
    /// 一句话都没说，本轮继续（措辞未启用 / 取不到措辞 / 措辞为空）。
    Silent,
}

/// 调 `reply/compose` 拿一段文本。
///
/// 返回 `None` = 没有措辞可用（未挂载 `reply` / 路由失败 / 出参为空）。
/// 与 `decide.rs` 同一条处置原则：**「没有这个插件」是一个正常状态**，
/// `NotFound` 静默放行，其它错误告警但**仍然放行**——措辞是增强，不是正确性的前提。
async fn compose_text(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn PluginInvokeRequest>,
    context: &SessionContext,
    verdict: &Verdict,
    snapshot: &RunSnapshot,
) -> Option<String> {
    let parent = orchestrator.parent.as_ref()?;
    let session_id = ctx.get(SESSION_ID).unwrap_or_default();

    let conversation: Vec<ChatMessage> =
        conversation_view(&context.messages, CONVERSATION_VIEW_LIMIT);

    let req = ctx.fork();
    req.set(PATH, ROUTE_REPLY_COMPOSE.to_string());
    req.set(SESSION_ID, session_id.clone());
    req.set_payload(ComposeRequest {
        session_id,
        verdict: verdict.clone(),
        context: conversation,
        snapshot: snapshot.clone(),
    })
    .ok()?;

    match parent.clone().route(req).await {
        Ok(payload) => match payload.get::<String>() {
            Ok(text) if !text.trim().is_empty() => Some(text),
            // 空串是**平凡值**（"没有对话面文本"），不是错误：调用方据此不写节点。
            Ok(_) => None,
            Err(e) => {
                crate::plugin_warn!(
                    "session",
                    "[Reply] 措辞载荷不是合法契约，按「无措辞」处理：{e}"
                );
                None
            }
        },
        Err(PluginError::NotFound(_)) => {
            // 未挂载 / 已停用：这正是「卸载平凡值」的形态，不是故障。
            crate::plugin_debug!("session", "[Reply] 未挂载措辞插件，本轮不产出对话面文本");
            None
        }
        Err(e) => {
            crate::plugin_warn!("session", "[Reply] 措辞调用失败，本轮不产出对话面文本：{e}");
            None
        }
    }
}

/// 构造对话面文本节点（**纯函数**，可单测）。
///
/// `exclude_from_context`：`Answered` 的答话为 `false`（它进请求包——它就是这一轮的
/// 答复），`Escalate` 的首响为 `true`（界面开场白，不进请求包）。判据见模块文档。
fn dialog_node(text: &str, reason: &str, exclude_from_context: bool) -> ChatMessage {
    let mut meta = serde_json::json!({
        "surface": SURFACE_REPLY,
        "reason": reason,
    });
    if exclude_from_context {
        meta["exclude_from_context"] = serde_json::json!(true);
    }
    ChatMessage {
        id: llm_short_id(),
        // 根级：与用户消息互为兄弟（对话线），不是某个 Turn 的子节点
        parent_id: None,
        role: Some(MessageRole::Assistant),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text(text.to_string())),
        status: Some(MessageStatus::Completed),
        timestamp: Some(clock_now_ms()),
        meta: Some(meta),
        ..Default::default()
    }
}

#[cfg(test)]
#[path = "compose.test.rs"]
mod tests;
