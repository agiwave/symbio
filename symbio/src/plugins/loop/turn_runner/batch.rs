//! 阶段③ 工具批派发与产物落格（T-179 · B3，PL-015 §5 动作 B3）。
//!
//! 派发本身在 [`crate::symbio_core::DispatchPort`]（插件侧实现）；本段只承接
//! 回来的 [`DispatchOutcome`]：逐条过产物闸、落 `artifact.added`、把本批调用
//! 结构累加进本轮事实。等待用户 / 继续下一轮的判定归 [`super::stream`]。
//!
//! **拆分契约**：循环体逐字来自 `run_with_tools` 的 2c 段，`if` 条件零变化。

use super::*;

/// 落本批产物格，并把调用结构累加进本轮事实。
pub(crate) async fn land_artifacts<S>(
    run: &mut Prepared,
    store: &S,
    rt: &LlmTurn,
    outcomes: &[crate::symbio_core::DispatchOutcome],
    round_facts: &StreamNodeFacts,
) -> Result<(), crate::symbio_core::AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    let turn = run.turn;
    let actor = run.actor.clone();
    let user_seq = run.user_seq;
    let turn_root_id = run.turn_root_id.clone();
    // 本轮的中途产出（思考 / 正文）**随这一轮第一格产物入格**。顺序是判据：
    // 实时面是「思考→卡片→思考→卡片」，若只记在收束格，重开会话会整体挪到
    // 全部工具卡片之后（见拆分前 2c 的长注释）。
    let round_nodes = rounds_payload(std::slice::from_ref(round_facts))
        .pop()
        .unwrap_or_else(|| serde_json::json!({}));
    for (oi, outcome) in outcomes.iter().enumerate() {
        // 产物闸（`ClosureKind::Artifact`，ADR-067）：不持 `produce.artifact` ⇒
        // 这条结果不入格（warn 一次，不刷屏）。结果照进 prompt / 照记
        // `all_tool_calls`——闸管的是「入格」，不是「发生过」。
        if !closure_granted(&actor, ClosureKind::Artifact) {
            if oi == 0 {
                crate::plugin_warn!(
                    "actors",
                    "[v2] 产物被授权拒绝（{} 缺 produce.artifact），本轮工具结果不入格（turn={turn}）",
                    actor.principal
                );
            }
            continue;
        }
        store.append(
            Event::pending(
                format!("a-{turn}-{}", run.artifact_no),
                crate::symbio_core::EVENT_ARTIFACT_ADDED,
                Entity::Artifact,
                Verb::Asserted,
                turn,
                &actor.principal,
            )
            .with_produced_by(user_seq)
            .with_payload(serde_json::json!({
                "tool": outcome.name,
                "text": outcome.text,
                "tool_call_id": outcome.call_id,
                "wire_id": outcome.wire_id,
                "args": outcome.args,
                "result_id": outcome.result_id,
                "success": outcome.ok,
                // 等待用户：结果节点是一张**待用户动作的卡**（`user_prompt` +
                // `waiting_user_action`），不是普通结果文本。
                "needs_user_action": outcome.needs_user_action,
                "prompt": outcome.prompt,
                "failure_kind": outcome.failure_kind,
                "turn_id": turn_root_id,
                // 本轮中途产出：只随**第一格**带一次（同一轮多张卡片时
                // 思考只有一段，挂两处即两个 owner）。
                "round": (oi == 0).then(|| round_nodes.clone()),
            })),
        )?;
        run.artifact_no += 1;
        run.artifacts_landed += 1;
    }
    // 本轮工具调用的结构进累积（配得上结果的记结果节点 id）。
    run.all_tool_calls.extend(
        rt.tool_calls
            .iter()
            .map(|tc| tool_fact_of(tc, outcome_of(outcomes, tc))),
    );
    Ok(())
}
