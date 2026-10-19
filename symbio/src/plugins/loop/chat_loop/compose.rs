//! 对话面措辞的调用点 + **落点** —— `session` ⇄ `compose` 的那条边。
//!
//! ## 它做三件事
//!
//! 1. 把**对话线**投影出来（与 `decide.rs` 同一个函数、同一个窗口）；
//! 2. 经容器 `route` 调 `compose/wording`（`ctx.fork()` + `PATH` 常量 + 契约载荷）；
//! 3. 把返回的文本**写进转写**——唯一写入者仍是 `session`（ADR-020）。
//!    `compose` 只返回一个 `String`，一个节点都不写。
//!
//! ## 落点：**根级**，不是某个 Turn 之下
//!
//! 首响 / 答话属于**对话线**（用户真的看到过的话），落在会话根上——与用户消息
//! 互为兄弟，是一个独立的节点，不是某一轮工作的一部分。三条理由，第一条是结构性的：
//!
//! 1. **说这话的时候还没有 Turn**：判决与首响发生在轮首（`run_chat_loop` 步骤 2），
//!    而 `Turn` 容器由 `prepare_turn_inputs` 创建（步骤 3）——`Answered` 那条路径
//!    更是整轮都不产生 Turn。要挂在 Turn 之下就得先造一个容器再挂上去，那是为了
//!    放一句话而改变工作的形状。
//! 2. **它不是工作的一轮**：Turn 组承载「这一轮干了什么」（推理 / 工具 / 正文），
//!    首响是这一轮**之外**的一句旁白。
//! 3. **位置不参与对话线的分界**（判据只有角色与类型，见 [`conversation_view`]
//!    的模块文档）。这里选根级是因为**语义**——它是一句独立的话——不是因为
//!    "分界函数只认这个形状"。
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
//! `Answered` 而措辞为空（未挂载 `compose` / `compose_enabled = false` / 连兜底模板都取不到）
//! ⇒ 本轮**照旧进工具循环**。理由与 `classify` 分类失败的兜底方向是同一条：
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

use crate::symbio_core::chat_node::ChatNode;
use crate::symbio_core::{ComposeRequest, RunSnapshot, Verdict, WordingReply};
use crate::symbio_core::{
    PluginError, PluginInvokeRequest, PluginInvokeRequestExt, PATH, ROUTE_COMPOSE_WORDING,
    SESSION_ID,
};
// 对话面节点的构造器已上移 core `node_build`（[ADR-082](../../../../../docs/010-adr/082.md "ADR-082") 裁决三）
use crate::symbio_core::node_build::dialog_node;

use super::super::context::{conversation_view, CONVERSATION_VIEW_LIMIT};
use super::state::{ChatOrchestrator, SessionContext};

/// 汇报节点的理由码（`meta.reason`）。
///
/// ## 为什么它由 `session` 拥有，而不是共享词表里的一行
///
/// 对话面的理由码词表在 `schemas::dialog`，因为它的生产方（`classify`）与消费方
/// （`compose`）分处两个插件。而汇报的理由码是**本侧自己产的**：判决 `Report` 由
/// `session` 判出（见 `dialog/mod.rs` 的变体表），措辞只是执行它——`compose`
/// 按判决分派，不看这个码。它没有第二个生产方，因此不进那份共享词表。
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
) -> (VerdictEffect, Option<WordingReply>) {
    // 调用方关掉了措辞（或压根没挂 `compose`）⇒ 与"未挂载 compose"逐字一致：
    // `Answered` 降级进工具循环，`Escalate` 没有首响，`Report` 什么都没说。
    // 三条都在下面自然成立。
    if !orchestrator.compose_enabled {
        crate::plugin_debug!("session", "[Compose] 措辞未启用，本轮不产出对话面文本");
        return (VerdictEffect::Silent, None);
    }

    match &verdict {
        Verdict::Answered { reason } => {
            let Some(reply) = compose_text(orchestrator, ctx, context, &verdict, snapshot).await
            else {
                // 判决说能直接答，但没人能说话 ⇒ **降级进工具循环**（不沉默）。
                crate::plugin_warn!(
                    "session",
                    "[Compose] 判决为 Answered（reason={reason}）但取不到措辞，本轮降级进工具循环"
                );
                return (VerdictEffect::Silent, None);
            };
            context
                .messages
                .push(dialog_node(&reply.text, reason, false));
            // 思考事实随答话交给调用方：`Answered` 收束格是**事实层**里这条
            // 模型往回的唯一去处（见 `execute_final_reply_turn` 的入参）。
            (VerdictEffect::Finish, Some(reply))
        }
        Verdict::Escalate { reason } => {
            // 首响开关关掉 ⇒ 不写那句开场白，本轮照旧进工具循环干活。
            // 与 `compose_enabled` 关掉的区别：那个连真实答复（`Answered`）一起没，
            // 本开关只削派活前的这一句旁白（机制化前端的 Typing 占位已接管"有动静"
            // 的职责，这句模板话与执行过程信息重复）。
            if !orchestrator.opening_enabled {
                crate::plugin_debug!("session", "[Compose] 开场白未启用，Escalate 本轮不写首响");
                return (VerdictEffect::Silent, None);
            }
            // 首响：拿不到就只是少一句开场白，本轮照旧干活。
            match compose_text(orchestrator, ctx, context, &verdict, snapshot).await {
                Some(reply) => {
                    context
                        .messages
                        .push(dialog_node(&reply.text, reason, true));
                    (VerdictEffect::Spoke, None)
                }
                None => (VerdictEffect::Silent, None),
            }
        }
        // 汇报：措辞从**运行现状**组织（`compose` 的模板产线，零 LLM 往返）。
        // 拿不到就不说——本轮照旧干活，且不消耗配额（调用方按 [`VerdictEffect`] 判）。
        Verdict::Report => {
            match compose_text(orchestrator, ctx, context, &verdict, snapshot).await {
                Some(reply) => {
                    context
                        .messages
                        .push(dialog_node(&reply.text, REASON_PROGRESS, true));
                    (VerdictEffect::Spoke, None)
                }
                None => (VerdictEffect::Silent, None),
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

/// 调 `compose/wording` 拿一段文本。
///
/// 返回 `None` = 没有措辞可用（未挂载 `compose` / 路由失败 / 出参为空）。
/// 与 `decide.rs` 同一条处置原则：**「没有这个插件」是一个正常状态**，
/// `NotFound` 静默放行，其它错误告警但**仍然放行**——措辞是增强，不是正确性的前提。
async fn compose_text(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn PluginInvokeRequest>,
    context: &SessionContext,
    verdict: &Verdict,
    snapshot: &RunSnapshot,
) -> Option<WordingReply> {
    compose_text_from(orchestrator, ctx, &context.messages, verdict, snapshot).await
}

/// [`compose_text`] 的真身：会话视图由调用方给。
///
/// 拆这一步不是为了「少一层」，而是轮边界那条路拿不到 `&SessionContext`
/// （见 [`report_text`]）——两条路共用同一个路由调用，否则改措辞要改两处。
async fn compose_text_from(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn PluginInvokeRequest>,
    messages: &[ChatNode],
    verdict: &Verdict,
    snapshot: &RunSnapshot,
) -> Option<WordingReply> {
    let parent = orchestrator.parent.as_ref()?;
    let session_id = ctx.get(SESSION_ID).unwrap_or_default();

    let conversation: Vec<ChatNode> = conversation_view(messages, CONVERSATION_VIEW_LIMIT);

    let req = ctx.fork();
    req.set(PATH, ROUTE_COMPOSE_WORDING.to_string());
    req.set(SESSION_ID, session_id.clone());
    req.set_payload(ComposeRequest {
        session_id,
        verdict: verdict.clone(),
        context: conversation,
        snapshot: snapshot.clone(),
    })
    .ok()?;

    match parent.clone().route(req).await {
        Ok(payload) => match payload.get::<WordingReply>() {
            Ok(reply) if reply.text.trim().is_empty() => {
                // 空串是**平凡值**（"没有对话面文本"），不是错误：调用方据此不写节点。
                None
            }
            Ok(reply) => Some(reply),
            Err(e) => {
                crate::plugin_warn!(
                    "session",
                    "[Compose] 措辞载荷不是合法契约，按「无措辞」处理：{e}"
                );
                None
            }
        },
        Err(PluginError::NotFound(_)) => {
            // 未挂载 / 已停用：这正是「卸载平凡值」的形态，不是故障。
            crate::plugin_debug!("session", "[Compose] 未挂载措辞插件，本轮不产出对话面文本");
            None
        }
        Err(e) => {
            crate::plugin_warn!(
                "session",
                "[Compose] 措辞调用失败，本轮不产出对话面文本：{e}"
            );
            None
        }
    }
}

/// 只取措辞、不落点（缺口 4 的轮边界路径）。
///
/// 与 [`apply_verdict`] 的差别有两条，都是接线逼出来的：
/// 1. **不拿 `&mut SessionContext`**：轮边界回调是 `Fn + Send + Sync + 'static`，
///    捕获不了 `run_chat_loop` 的栈局部 `context`。
/// 2. **拿会话视图而不是整个 context**：`ComposeRequest` 里那一段就是
///    `context.messages` 的一个切片，拆开传同一个信息、不丢东西。
///
/// 调用方（`chat_loop::round_report_hook`）拿到文本后交回 `turn_exec`——
/// 因为落格要 `EventWalStore`，而开着它的是那边。
pub(crate) async fn report_text(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn PluginInvokeRequest>,
    conversation: &[ChatNode],
    snapshot: &RunSnapshot,
) -> Option<String> {
    if !orchestrator.compose_enabled {
        crate::plugin_debug!("session", "[Compose] 措辞未启用，本轮不汇报");
        return None;
    }
    // 汇报是**模板**产线（零模型往返），思考事实恒 `None`，这里只取文本。
    compose_text_from(orchestrator, ctx, conversation, &Verdict::Report, snapshot)
        .await
        .map(|r| r.text)
}

#[cfg(test)]
#[path = "compose.test.rs"]
mod tests;
