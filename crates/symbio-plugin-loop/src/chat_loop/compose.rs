//! 对话面措辞的调用点 + **落点** —— `session` ⇄ `compose` 的那条边。
//!
//! ## 它做三件事
//!
//! 1. 把**对话线**投影出来（与 `decide.rs` 同一个函数、同一个窗口）；
//! 2. 经容器 `route` 调 `compose/wording`（`ctx.fork()` + `PATH` 常量 + 契约载荷）；
//! 3. 把返回的文本**写进转写**——唯一写入者仍是 `session`（D-020）。
//!    `compose` 只返回一个 `String`，一个节点都不写。
//!
//! ## 落点：**根级**，不是某个 Turn 之下
//!
//! 首响 / 答话属于**对话线**（用户真的看到过的话），落在会话根上——与用户消息
//! 互为兄弟，是一个独立的节点，不是某一轮工作的一部分。三条理由，第一条是结构性的：
//!
//! 1. **说这话的时候还没有 Turn**：判决与首响发生在轮首（`run_chat_loop` 步骤 2），
//!    而 `Turn` 容器由 `prepare_request_sections` 创建——`Answered` 那条路径
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
//! ## 返回值：只交措辞
//!
//! 判决执行（哪种判决要落什么事实 / 是否收尾）已上移到行动作
//! （`turn_exec::policy`）；本模块只负责**问 compose 要一段文本**。

use std::sync::Arc;

use symbio_kernel::dialog::{ComposeRequest, RunSnapshot, Verdict, WordingReply};
use symbio_sdk::chat::chat_node::ChatNode;

use symbio_kernel::plugin::{PluginError, PluginInvokeRequest, PluginInvokeRequestExt};
use symbio_plugin_api::routes::ROUTE_COMPOSE_WORDING;
use symbio_sdk::keys::{PATH, SESSION_ID};

use super::super::context::{conversation_view, CONVERSATION_VIEW_LIMIT};
use super::super::port::{round_trip, Round};
use super::state::{ChatOrchestrator, SessionContext, TurnState};

/// 生成产线那一次措辞调用，**按端口 `model` 记账**。
///
/// ## 为什么账记在这一侧而不是 `compose` 里
///
/// DE-015 §2.4：`request` 由**决策者**写。决定「这一轮欠一次模型往返」的是拿着判决
/// 的编排层；`compose` 只是被叫去把那句话组织出来，它连事实源都没有（转写的唯一
/// 写入者仍是 `session`，D-020）。而 DE-015 判据 2 要的是**每一次**模型调用都有
/// `sent` + 应答——「内部静默请求」从来不是豁免理由：它同样是一次对外调用，
/// 崩溃后同样得让租约看护收走它。
///
/// ## 为什么只包这一条产线
///
/// `Escalate` 首响与 `Report` 汇报都是模板，零模型往返。给它们也落一条
/// `request(port=model)`，三格就不再等价于「一次模型调用」，日志从此数不准——
/// 那与「调了模型没有账」是同一种病的两个方向。
///
/// ## 记账失败换成告警，不换成沉默，也不换成第二次调用
///
/// 派发被拒时执行体没跑、应答落格失败时它已经跑完，两种 `Err` 都不能靠重跑来救。
/// 交回 `None` 走的是既有那条「无措辞 ⇒ `Answered` 降级进工具循环」的路（本轮仍然
/// 会答，只是不再短路）。
pub(crate) async fn wording_round_trip(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn PluginInvokeRequest>,
    context: &SessionContext,
    turn: &TurnState,
    verdict: &Verdict,
    snapshot: &RunSnapshot,
) -> Option<WordingReply> {
    let Some(store) = context.session.facts() else {
        // 临时会话没有事实源 ⇒ 没有账可记，照旧说话（持久轮已由 `open-turn` 拦过）。
        return compose_text(orchestrator, ctx, context, verdict, snapshot).await;
    };
    // 步名带**请求轮次**：一幕可以有多次往返（auto 续写），每次是一笔独立的账
    // （与端口 `turn` 的 id 同一口径，见主循环的 `turn-{turn}-d{drive}-r{round}`）。
    let step = format!("w{}", turn.round());
    let round = Round {
        turn: turn.turn_no,
        produced_by: turn.user_seq,
        drive_seq: turn.drive_seq,
        now: turn.now_ms,
        step: &step,
        via: "compose/wording",
    };
    match round_trip(store.as_ref(), round, || async {
        // 措辞拿不到（未挂载 / 生成失败）是**这一趟的平凡结果**，不是这次往返的
        // 失败：往返本身成了（请求 / 已发 / 应答三格齐），只是它没带来文本。
        // 于是错误类型是 `String`——这一趟没有失败，只有「没有文本」。
        Ok::<_, String>(compose_text(orchestrator, ctx, context, verdict, snapshot).await)
    })
    .await
    {
        Ok(reply) => reply,
        Err(e) => {
            symbio_plugin_api::plugin_warn!(
                "session",
                "[Compose] 答话往返的端口记账失败（本轮不短路）: {e}"
            );
            None
        }
    }
}

/// 调 `compose/wording` 拿一段文本。
///
/// 返回 `None` = 没有措辞可用（未挂载 `compose` / 路由失败 / 出参为空）。
/// 与 `decide.rs` 同一条处置原则：**「没有这个插件」是一个正常状态**，
/// `NotFound` 静默放行，其它错误告警但**仍然放行**——措辞是增强，不是正确性的前提。
pub(crate) async fn compose_text(
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
                symbio_plugin_api::plugin_warn!(
                    "session",
                    "[Compose] 措辞载荷不是合法契约，按「无措辞」处理：{e}"
                );
                None
            }
        },
        Err(PluginError::NotFound(_)) => {
            // 未挂载 / 已停用：这正是「卸载平凡值」的形态，不是故障。
            symbio_plugin_api::plugin_debug!(
                "session",
                "[Compose] 未挂载措辞插件，本轮不产出对话面文本"
            );
            None
        }
        Err(e) => {
            symbio_plugin_api::plugin_warn!(
                "session",
                "[Compose] 措辞调用失败，本轮不产出对话面文本：{e}"
            );
            None
        }
    }
}

/// 只取措辞、不落点（轮边界汇报路径）。
///
/// 与判决执行体（`policy::execute_verdict`）的差别有两条，都是接线逼出来的：
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
        symbio_plugin_api::plugin_debug!("session", "[Compose] 措辞未启用，本轮不汇报");
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
