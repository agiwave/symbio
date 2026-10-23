//! 轮首判决的调用点 —— `session` ⇄ `classify` 的那条边。
//!
//! ## 它做三件事
//!
//! 1. 把**对话线**投影出来（[`conversation_view`]）——插件不读存储，读的是投影；
//! 2. 经容器 `route` 调 `classify/decide`（`ctx.fork()` + `PATH` 常量 + 契约载荷）；
//! 3. 判完了就收判决；没判完就照它给的**计划**打一次**入账的**模型往返，
//!    应答再经 `classify/parse` 收回成判决。
//!
//! ## 为什么模型往返搬到了这一侧（ADR-122 决策 2）
//!
//! 判决的模型往返必须是**义务环里的一笔账**（请求 / 已发 / 应答三格）：插件自己
//! 打模型的话，那次往返在账上是**看不见**的——崩溃后不晓得上次发到哪，租约也无从
//! 看护它。于是 `classify` 只出**计划**（提示词 + 消息 + 模型），派发归这一侧。
//!
//! ## 「没有模型服务」是一次**零格**的判决
//!
//! 找服务在派发**之前**：找不到 ⇒ 这一轮压根不欠模型往返 ⇒ 行不成立 ⇒ 零入账格
//! （ADR-122 决策 4）。反过来（先派发再发现没有服务）会落一条「派发了却没打模型」
//! 的账——三格从此不代表一次模型往返，日志不能再用来数调用。
//!
//! ## 为什么经容器 `route` 而不是直连插件实例
//!
//! 直连要求 session 按值持有 `classify`，那会绕过地址分发（`plugin-entry-audit` 的
//! E-007 正是拦这个），并在插件重建后钉住旧实例。`parent` 已经是容器句柄
//! （[`ChatOrchestrator::parent`]），本方案不需要新增任何持有关系。
//!
//! ## 「没有这个插件」是一个**正常状态**，不是错误
//!
//! `classify` 的卸载语义就是「全部输入直接进工具循环」（= 引入它之前的行为）。
//! 因此这里对路由失败的处理是**返回「无判决」并放行**，而不是报错：
//!
//! - `NotFound`（未挂载 / 已停用）：**静默**——这是预期的装配形态，
//!   每次判决都告警会变成噪声；
//! - 其它错误（契约不符 / 内部失败）：告警，但**仍然放行**——判决是增强，
//!   不是正确性的前提。把它升级成会话失败，等于让一个可选插件能拖垮主路径。
//!
//! 这两条合起来就是「可卸载」在调用侧的落地形态：**调用方按缺插件处理**。

use std::sync::Arc;

use crate::symbio_core::chat_node::ChatNode;
use crate::symbio_core::{
    capability_model_resolve, DecideOutcome, DecisionPlan, ExecAbortSignal, LlmAdapter,
    ParseVerdictRequest, Plugin, PluginError, PluginInvokeRequest, PluginInvokeRequestExt,
    ProviderLlmAdapter, Registry, SilentDeltas, TokenIssuer, Verdict, CAPABILITY_VISITOR,
};
use crate::symbio_core::{
    DecideRequest, PATH, ROUTE_CLASSIFY_DECIDE, ROUTE_CLASSIFY_PARSE, SESSION_ID,
};

use super::super::context::{conversation_view, CONVERSATION_VIEW_LIMIT};
use super::super::port::classify::{classify_round_trip, escalate_unclassified};
use super::super::port::Round;
use super::state::{ChatOrchestrator, SessionContext, TurnState};

/// 一次轮首判决的**结果**：判决（可缺省）+ 它打了几次模型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Decision {
    /// 判决。`None` = **没有判决**（没挂载 `classify` / 路由失败 / 契约不符）——
    /// 调用方按「引入本插件之前的行为」继续，即全部输入进工具循环。
    pub(crate) verdict: Option<Verdict>,
    /// 这一轮判决发起的**模型往返次数**（规则短路 / 没有模型服务 = 0）。
    ///
    /// 它是那次往返在事实层唯一的**数**（`turn.stage{decide}` 的 `model_calls`），
    /// 判据侧拿它对账「观测 ⇔ 三格」。
    pub(crate) model_calls: u64,
}

/// 轮首判决。
///
/// 判完（规则短路）⇒ 零入账格；没判完 ⇒ 一次**入账的**模型往返 + 一次解析路由。
pub(crate) async fn decide_turn(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn PluginInvokeRequest>,
    context: &SessionContext,
    turn: &TurnState,
    utterance: &str,
) -> Decision {
    let Some(parent) = orchestrator.parent.as_ref() else {
        return Decision {
            verdict: None,
            model_calls: 0,
        };
    };
    let session_id = ctx.get(SESSION_ID).unwrap_or_default();

    let conversation: Vec<ChatNode> = conversation_view(&context.messages, CONVERSATION_VIEW_LIMIT);

    let req = ctx.fork();
    req.set(PATH, ROUTE_CLASSIFY_DECIDE.to_string());
    req.set(SESSION_ID, session_id.clone());
    if req
        .set_payload(DecideRequest {
            session_id: session_id.clone(),
            utterance: Some(utterance.to_string()),
            context: conversation,
        })
        .is_err()
    {
        crate::plugin_warn!(
            "session",
            "[Classify] 判决载荷装不进请求，本轮直接进工具循环"
        );
        return Decision {
            verdict: None,
            model_calls: 0,
        };
    }

    let outcome = match parent.clone().route(req).await {
        Ok(payload) => match payload.get::<DecideOutcome>() {
            Ok(outcome) => outcome,
            Err(e) => {
                crate::plugin_warn!(
                    "session",
                    "[Classify] 判决载荷不是合法契约，按「无判决」处理：{e}"
                );
                return Decision {
                    verdict: None,
                    model_calls: 0,
                };
            }
        },
        Err(PluginError::NotFound(_)) => {
            // 未挂载 / 已停用：这正是「卸载平凡值」的形态，不是故障。
            crate::plugin_debug!("session", "[Classify] 未挂载判决插件，本轮直接进工具循环");
            return Decision {
                verdict: None,
                model_calls: 0,
            };
        }
        Err(e) => {
            crate::plugin_warn!(
                "session",
                "[Classify] 判决调用失败，本轮直接进工具循环：{e}"
            );
            return Decision {
                verdict: None,
                model_calls: 0,
            };
        }
    };

    let plan = match outcome {
        // 判完了（规则短路）：**零模型往返** ⇒ 零入账格（ADR-122 决策 4）。
        DecideOutcome::Decided { verdict } => {
            return Decision {
                verdict: Some(verdict),
                model_calls: 0,
            };
        }
        DecideOutcome::NeedsModel { plan } => plan,
    };

    decide_by_model(ctx, context, turn, parent, session_id, plan).await
}

/// 模型路径：照**计划**打一次入账的模型往返，把应答收回成判决。
///
/// 三个环节的顺序是判据的一部分：**先找服务**（找不到 ⇒ 零格），再**派发**
/// （落请求 / 已发 / 应答三格），最后**解析**（应答 → 判决）。
async fn decide_by_model(
    ctx: &Arc<dyn PluginInvokeRequest>,
    context: &SessionContext,
    turn: &TurnState,
    parent: &Arc<dyn Plugin>,
    session_id: String,
    plan: DecisionPlan,
) -> Decision {
    // ① 模型服务：找服务在派发**之前**——找不到就没有这次往返，也就没有账。
    let Some(provider) = resolve_provider(ctx, plan.model_id.as_deref()).await else {
        crate::plugin_warn!(
            "session",
            "[Classify] 没有可用的模型服务，本轮按「判不出来」处理"
        );
        return Decision {
            verdict: Some(escalate_unclassified()),
            model_calls: 0,
        };
    };
    // ② 事实源：没有它就没有账可记。持久轮的事实源由 `begin_drive` 保证（临时会话
    //    在那里就被拦下），这一段只是把「拦下之前」的形态说清楚。
    let Some(store) = context.session.facts() else {
        crate::plugin_warn!("session", "[Classify] 本会话没有事实源，判决往返不入账");
        return Decision {
            verdict: Some(escalate_unclassified()),
            model_calls: 0,
        };
    };

    // ③ 派发：逻辑 id 与端口 `model` 的 id 同一口径（`model-{turn}-d{drive}-{step}`）。
    let step = format!("c{}", turn.request_round);
    let round = Round {
        turn: turn.turn_no,
        produced_by: turn.user_seq,
        drive_seq: turn.drive_seq,
        now: turn.now_ms,
        step: &step,
        via: "classify/decide",
    };
    let llm =
        ProviderLlmAdapter::for_turn(provider, &plan.system_prompt, ExecAbortSignal::of(&**ctx));
    let tok = TokenIssuer::issue_deep();
    let parse_parent = parent.clone();
    let parse_ctx = ctx.clone();
    let parse_session = session_id.clone();

    let decision = classify_round_trip(
        store.as_ref(),
        round,
        DecideOutcome::NeedsModel { plan },
        move |plan| async move {
            // 内部请求的出口是静默的：分类请求的流式帧绝不能以自有身份进入对话流。
            let out = llm
                .generate_turn(&tok, &plan.messages, &[], Arc::new(SilentDeltas))
                .await
                .map_err(|e| format!("{e:?}"))?;
            Ok(out.text)
        },
        move |text: &str| {
            let text = text.to_string();
            let parent = parse_parent.clone();
            let ctx = parse_ctx.clone();
            let session_id = parse_session.clone();
            async move { parse_reply(&parent, &ctx, &session_id, &text).await }
        },
    )
    .await;

    Decision {
        verdict: Some(decision.verdict),
        model_calls: decision.model_calls,
    }
}

/// 取这次判决要用的模型服务。
///
/// 优先**计划里配的那个** provider id（严格查找），取不到落回当次生效者——
/// 降级方向只有一处定义（`capability_model_resolve`）。找不到 ⇒ `None` ⇒ 零格。
async fn resolve_provider(
    ctx: &Arc<dyn PluginInvokeRequest>,
    model_id: Option<&str>,
) -> Option<Arc<dyn crate::symbio_core::ModelProvider>> {
    let visitor: Arc<dyn Registry> = ctx.get(CAPABILITY_VISITOR)?;
    capability_model_resolve(&*visitor, model_id).await
}

/// 把一次判决往返的**应答**解析成判决（`classify/parse`）。
///
/// 解析留在 `classify` 侧：词表是它的内部知识，编排层不认那四个词（见
/// `dialog/mod.rs` 的 `ParseVerdictRequest` 文档）。解析不成 ⇒ 「判不出来」。
async fn parse_reply(
    parent: &Arc<dyn Plugin>,
    ctx: &Arc<dyn PluginInvokeRequest>,
    session_id: &str,
    text: &str,
) -> Verdict {
    let req = ctx.fork();
    req.set(PATH, ROUTE_CLASSIFY_PARSE.to_string());
    req.set(SESSION_ID, session_id.to_string());
    if req
        .set_payload(ParseVerdictRequest {
            session_id: session_id.to_string(),
            text: text.to_string(),
        })
        .is_err()
    {
        crate::plugin_warn!(
            "session",
            "[Classify] 判决解析载荷装不进请求，按「判不出来」处理"
        );
        return escalate_unclassified();
    }
    match parent.clone().route(req).await {
        Ok(payload) => match payload.get::<Verdict>() {
            Ok(verdict) => verdict,
            Err(e) => {
                crate::plugin_warn!(
                    "session",
                    "[Classify] 判决解析载荷不是合法契约，按「判不出来」处理：{e}"
                );
                escalate_unclassified()
            }
        },
        Err(e) => {
            crate::plugin_warn!(
                "session",
                "[Classify] 判决解析失败，按「判不出来」处理：{e}"
            );
            escalate_unclassified()
        }
    }
}
