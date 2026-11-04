//! 环上三条 `policy.*` 行动作：判决 / 召回 / 压缩。
//!
//! 三者的共同纪律：**只产出事实，不往 `TurnState` 寄存读侧**（DE-048 D-306 /
//! D-313 P2-2）。动作做完就落格；下一步要读的是日志，不是本轮的字段。

use std::sync::Arc;

use crate::session_port;

use symbio_kernel::dialog::{
    DecideOutcome, DecideRequest, DecisionPlan, ParseVerdictRequest, Verdict,
};
use symbio_kernel::exec::ExecAbortSignal;
use symbio_kernel::plugin::{Plugin, PluginError, PluginInvokeRequest, PluginInvokeRequestExt};
use symbio_plugin_api::routes::{ROUTE_CLASSIFY_DECIDE, ROUTE_CLASSIFY_PARSE};
use symbio_sdk::adapters::{SilentDeltas, TokenIssuer};
use symbio_sdk::chat::chat_node::{ChatNode, NodeRole};
use symbio_sdk::keys::{PATH, SESSION_ID};
use symbio_sdk::turn::node_build::dialog_node;

use super::super::chat_loop::compose::{compose_text, wording_round_trip};
use super::super::chat_loop::inputs::last_user_text;
use super::super::chat_loop::state::{ChatOrchestrator, SessionContext, TurnState};
use super::super::context::{conversation_view, CONVERSATION_VIEW_LIMIT};
use super::super::port::classify::{classify_round_trip, escalate_unclassified};
use super::super::port::{abort_round_trip, model_port, Compress, Round};
use super::super::stage_events::{AnswerFact, OpeningFact, StageFact};
use super::ActorError;

/// 一次轮首判决的**结果**：判决（可缺省）+ 它打了几次模型。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Decision {
    verdict: Option<Verdict>,
    model_calls: u64,
}

/// `policy.classify` 行：判决 → 执行 → 出口格。
pub(crate) async fn classify(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn PluginInvokeRequest>,
    store: &Arc<dyn symbio_kernel::store::Store<Event = symbio_kernel::event::Event>>,
    context: &mut SessionContext,
    state: &mut TurnState,
) -> Result<(), ActorError> {
    let Some(utterance) = state.user_text() else {
        return Ok(());
    };
    let round = state.round();

    // 模型服务缺席不是「判不出来」：落失败格并中止，不许静默滑进工具循环
    // （D-298；此前那条路径在这里被折叠成了「无判决」）。
    let decision = match decide(orchestrator, ctx, context, state, &utterance).await {
        Ok(decision) => decision,
        Err(e) => {
            let fact = StageFact::classify(crate::loop_rows::STEP_DECIDE, round, None, 0)
                .with_result("failed");
            super::super::stage_events::land(
                &**store,
                state.turn_no,
                state.user_seq,
                &symbio_sdk::authz::request_principal(ctx.as_ref()),
                &[fact],
            )
            .map_err(ActorError::Append)?;
            return Err(ActorError::Read(e));
        }
    };

    let verdict = decision.verdict.clone();
    let snapshot = symbio_kernel::dialog::RunSnapshot::default();
    let (result, answer, opening) = match verdict.clone() {
        Some(verdict) => {
            execute_verdict(orchestrator, ctx, context, state, verdict, &snapshot).await
        }
        None => ("proceed".to_string(), None, None),
    };

    let mut fact = StageFact::classify(
        crate::loop_rows::STEP_DECIDE,
        round,
        verdict.as_ref(),
        decision.model_calls,
    )
    .with_result(&result);
    if let Some(answer) = answer {
        fact = fact.with_answer(answer);
    }
    if let Some(opening) = opening {
        fact = fact.with_opening(opening);
    }
    super::super::stage_events::land(
        &**store,
        state.turn_no,
        state.user_seq,
        &symbio_sdk::authz::request_principal(ctx.as_ref()),
        &[fact],
    )
    .map_err(ActorError::Append)?;
    Ok(())
}

/// 执行一条判决：说该说的话、把要落的内容交回给调用方。
///
/// 返回值 = `(result, answer, opening)`；`result == "answered"` 时本轮由
/// `land_answered_turn` 收束（那体答话就是这里的 `answer`）。
#[allow(clippy::too_many_lines)]
async fn execute_verdict(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn PluginInvokeRequest>,
    context: &mut SessionContext,
    state: &TurnState,
    verdict: Verdict,
    snapshot: &symbio_kernel::dialog::RunSnapshot,
) -> (String, Option<AnswerFact>, Option<OpeningFact>) {
    if !orchestrator.compose_enabled {
        symbio_plugin_api::plugin_debug!("session", "[Compose] 措辞未启用，本轮不产出对话面文本");
        return ("proceed".to_string(), None, None);
    }

    match &verdict {
        Verdict::Answered { reason } => {
            let reply = if verdict.needs_wording_generation() {
                wording_round_trip(orchestrator, ctx, context, state, &verdict, snapshot).await
            } else {
                compose_text(orchestrator, ctx, context, &verdict, snapshot).await
            };
            let Some(reply) = reply else {
                symbio_plugin_api::plugin_warn!(
                    "session",
                    "[Compose] 判决为 Answered（reason={reason}）但取不到措辞，本轮降级进工具循环"
                );
                return ("proceed".to_string(), None, None);
            };
            context.messages.push(dialog_node(
                &reply.text,
                reason,
                false,
                &orchestrator.view.principal,
            ));
            let (dialog_id, dialog_reason, exclude_from_context) = last_assistant_node(context);
            let reasoning = reply.reasoning.map(|text| {
                let id = reply
                    .reasoning_id
                    .unwrap_or_else(symbio_sdk::llm::turn::llm_short_id);
                (id, text)
            });
            (
                "answered".to_string(),
                Some(AnswerFact {
                    utterance: state.user_text().unwrap_or_default(),
                    reply: reply.text,
                    cost_ms: reply.cost_ms,
                    dialog_id,
                    dialog_reason,
                    exclude_from_context,
                    reasoning,
                }),
                None,
            )
        }
        Verdict::Escalate { reason } => {
            if !orchestrator.opening_enabled {
                symbio_plugin_api::plugin_debug!(
                    "session",
                    "[Compose] 开场白未启用，Escalate 本轮不写首响"
                );
                return ("proceed".to_string(), None, None);
            }
            let Some(reply) = compose_text(orchestrator, ctx, context, &verdict, snapshot).await
            else {
                return ("proceed".to_string(), None, None);
            };
            context.messages.push(dialog_node(
                &reply.text,
                reason,
                true,
                &orchestrator.view.principal,
            ));
            let (dialog_id, dialog_reason, _) = last_assistant_node(context);
            (
                "proceed".to_string(),
                None,
                Some(OpeningFact {
                    text: reply.text,
                    dialog_id,
                    dialog_reason,
                }),
            )
        }
        // 轮首不产生 `Report`；留一条保守分支，行为与引入判决之前一致。
        Verdict::Report => ("proceed".to_string(), None, None),
    }
}

/// 刚 push 进去的那条 assistant 节点的 `(id, reason, exclude_from_context)`。
fn last_assistant_node(context: &SessionContext) -> (String, String, bool) {
    context
        .messages
        .iter()
        .rev()
        .find(|m| m.node_role() == Some(NodeRole::Assistant))
        .map(|m| {
            let meta = m.meta.as_ref();
            (
                m.id.clone(),
                meta.and_then(|v| v.get("reason"))
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string(),
                meta.and_then(|v| v.get("exclude_from_context"))
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false),
            )
        })
        .unwrap_or_default()
}

/// 轮首判决：`classify` 短路或按计划打一次入账的模型往返。
async fn decide(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn PluginInvokeRequest>,
    context: &SessionContext,
    state: &TurnState,
    utterance: &str,
) -> Result<Decision, PluginError> {
    let Some(parent) = orchestrator.parent.as_ref() else {
        return Ok(Decision {
            verdict: None,
            model_calls: 0,
        });
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
        symbio_plugin_api::plugin_warn!(
            "session",
            "[Classify] 判决载荷装不进请求，本轮直接进工具循环"
        );
        return Ok(Decision {
            verdict: None,
            model_calls: 0,
        });
    }

    let outcome = match parent.clone().route(req).await {
        Ok(payload) => match payload.get::<DecideOutcome>() {
            Ok(outcome) => outcome,
            Err(e) => {
                symbio_plugin_api::plugin_warn!(
                    "session",
                    "[Classify] 判决载荷不是合法契约，按「无判决」处理：{e}"
                );
                return Ok(Decision {
                    verdict: None,
                    model_calls: 0,
                });
            }
        },
        Err(PluginError::NotFound(_)) => {
            symbio_plugin_api::plugin_debug!(
                "session",
                "[Classify] 未挂载判决插件，本轮直接进工具循环"
            );
            return Ok(Decision {
                verdict: None,
                model_calls: 0,
            });
        }
        Err(e) => {
            symbio_plugin_api::plugin_warn!(
                "session",
                "[Classify] 判决调用失败，本轮直接进工具循环：{e}"
            );
            return Ok(Decision {
                verdict: None,
                model_calls: 0,
            });
        }
    };

    let plan = match outcome {
        DecideOutcome::Decided { verdict } => {
            return Ok(Decision {
                verdict: Some(verdict),
                model_calls: 0,
            });
        }
        DecideOutcome::NeedsModel { plan } => plan,
    };

    decide_by_model(ctx, context, state, parent, session_id, plan).await
}

async fn decide_by_model(
    ctx: &Arc<dyn PluginInvokeRequest>,
    context: &SessionContext,
    state: &TurnState,
    parent: &Arc<dyn Plugin>,
    session_id: String,
    plan: DecisionPlan,
) -> Result<Decision, PluginError> {
    let Some(store) = context.facts() else {
        return Err(PluginError::InternalError(
            "本会话没有事实源，判决往返不入账".into(),
        ));
    };

    let step = format!("c{}", state.round());
    let round = Round {
        turn: state.turn_no,
        produced_by: state.user_seq,
        drive_seq: state.drive_seq,
        now: state.now_ms,
        step: &step,
        via: "classify/decide",
    };
    let Some(llm) = model_port(ctx).await.and_then(|provider| {
        provider.llm_round_trip(
            plan.model_id.as_deref(),
            &plan.system_prompt,
            ExecAbortSignal::of(&**ctx),
        )
    }) else {
        if let Err(e) = abort_round_trip(
            store.as_ref(),
            round,
            "`port.model` 缺席：没有可用的模型提供者",
        ) {
            symbio_plugin_api::plugin_warn!(
                symbio_deploy::PLUGIN_ID_SESSION,
                "[Classify] 缺席失败格落格失败：{e:?}"
            );
        }
        return Err(PluginError::InternalError(
            "`port.model` 缺席：没有可用的模型提供者".into(),
        ));
    };
    let tok = TokenIssuer::issue_deep();
    let parse_parent = parent.clone();
    let parse_ctx = ctx.clone();
    let parse_session = session_id.clone();

    let decision = classify_round_trip(
        store.as_ref(),
        round,
        DecideOutcome::NeedsModel { plan },
        move |plan| async move {
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
    .await
    .map_err(PluginError::InternalError)?;

    Ok(Decision {
        verdict: Some(decision.verdict),
        model_calls: decision.model_calls,
    })
}

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
        symbio_plugin_api::plugin_warn!(
            "session",
            "[Classify] 判决解析载荷装不进请求，按「判不出来」处理"
        );
        return escalate_unclassified();
    }
    match parent.clone().route(req).await {
        Ok(payload) => match payload.get::<Verdict>() {
            Ok(verdict) => verdict,
            Err(e) => {
                symbio_plugin_api::plugin_warn!(
                    "session",
                    "[Classify] 判决解析载荷不是合法契约，按「判不出来」处理：{e}"
                );
                escalate_unclassified()
            }
        },
        Err(e) => {
            symbio_plugin_api::plugin_warn!(
                "session",
                "[Classify] 判决解析失败，按「判不出来」处理：{e}"
            );
            escalate_unclassified()
        }
    }
}

/// `policy.recall` 行：召回 → 摘行 → 落召回格。
///
/// 召回读数（逐条正文 + 降级标记）是这一格的载荷：`prepare-turn` 与收束都从事实
/// 重建，不再寄存在 `TurnState`（DE-048 D-306）。
pub(crate) async fn recall(
    ctx: &Arc<dyn PluginInvokeRequest>,
    store: &dyn symbio_kernel::store::Store<Event = symbio_kernel::event::Event>,
    context: &SessionContext,
    state: &mut TurnState,
) -> Result<(), ActorError> {
    let recall_query = last_user_text(context);
    let recall_viewer = symbio_sdk::authz::request_principal(ctx.as_ref());
    let facts = context.facts_snapshot();
    let mut view = super::super::memory_events::recall_view(
        session_port::call::<Vec<symbio_sdk::turn::facts::FleetSnapshot>>(
            context.session.as_ref(),
            symbio_sdk::turn::dispatch::run_ops::RECALL_SNAPSHOTS,
            (),
        )
        .await
        .map_err(ActorError::Read)?,
        recall_query.as_deref(),
        &recall_viewer,
    )
    .unwrap_or_default();

    let retreat: Vec<u64> = symbio_plugin_api::read_shared_view_all("skill.retreat", &facts)
        .ok()
        .and_then(|value| value.as_array().cloned())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.get("recall").and_then(serde_json::Value::as_u64))
                .collect()
        })
        .unwrap_or_default();
    if !retreat.is_empty() {
        view.entries.retain(|entry| !retreat.contains(&entry.seq));
    }

    let resumed = facts
        .iter()
        .find(|e| e.seq.map(|s| s.value()) == Some(state.drive_seq))
        .is_some_and(|e| {
            e.kind == super::super::control_decl::EVENT_LOOP_STEP
                && e.payload.get("id").and_then(serde_json::Value::as_str)
                    == Some(crate::loop_rows::STEP_RESUME)
        });

    let entries: Vec<(u64, String, String)> = view
        .entries
        .iter()
        .map(|e| (e.seq, e.tag.clone(), e.content.clone()))
        .collect();
    super::super::stage_events::land(
        store,
        state.turn_no,
        state.user_seq,
        &recall_viewer,
        &[StageFact::recall(
            crate::loop_rows::STEP_RECALL,
            state.round(),
            state.drive_seq,
            &entries,
            resumed,
            view.degraded,
        )],
    )
    .map_err(ActorError::Append)?;
    Ok(())
}

/// `policy.compress` 行：进一次压缩产线并落「已做」。
pub(crate) async fn compress(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn PluginInvokeRequest>,
    store: &Arc<dyn symbio_kernel::store::Store<Event = symbio_kernel::event::Event>>,
    context: &mut SessionContext,
    state: &mut TurnState,
    system_prompt: &str,
    tools: &[symbio_kernel::capability::CapabilityMeta],
) -> Result<(), ActorError> {
    let overhead_tokens = super::super::context::estimate_overhead_with_tools(system_prompt, tools);
    let compress = Compress::new(
        store.clone(),
        state.turn_no,
        state.user_seq,
        state.drive_seq,
        state.now_ms,
        state.round(),
    );
    match super::super::context::auto_compress_process(
        orchestrator,
        context,
        ctx,
        &state.abort,
        overhead_tokens,
        false,
        Some(&compress),
    )
    .await
    {
        Ok(Some(history_count)) => {
            symbio_plugin_api::plugin_info!(
                "session",
                "[Compress] 上下文已压缩：{} 条消息 → 1 条摘要",
                history_count
            );
            state.last_saved = context.messages.len();
        }
        Ok(None) => {}
        Err(e) => {
            symbio_plugin_api::plugin_warn!("session", "[Compress] 自动压缩失败: {e}");
        }
    }
    super::super::stage_events::land(
        store.as_ref(),
        state.turn_no,
        state.user_seq,
        &symbio_sdk::authz::request_principal(ctx.as_ref()),
        &[StageFact::compress(
            crate::loop_rows::STEP_COMPRESS,
            state.round(),
            compress.calls(),
            state.drive_seq,
        )],
    )
    .map_err(ActorError::Append)?;
    Ok(())
}

#[cfg(test)]
#[path = "policy.test.rs"]
mod tests;
