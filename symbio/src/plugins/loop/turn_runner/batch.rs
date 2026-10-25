//! 阶段③ 工具批派发与产物落格（· B3，DE-021 §5 动作 B3）。
//!
//! 派发本身在 [`super::super::tool_dispatch::ToolDispatch`]（三动作契约）；本段只承接
//! 回来的 [`DispatchOutcome`]：逐条过产物闸、落 `artifact.added`。等待用户 /
//! 继续下一轮的判定归 [`super::stream`]。
//!
//! **拆分契约**：循环体逐字来自 `run_with_tools` 的 2c 段，`if` 条件零变化。

use super::*;

/// 落本批产物格。
pub(crate) async fn land_artifacts<S>(
    run: &mut Prepared,
    store: &S,
    outcomes: &[super::super::tool_dispatch::DispatchOutcome],
) -> Result<(), crate::symbio_core::AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    let turn = run.turn;
    let actor = run.actor.clone();
    let user_seq = run.user_seq;
    let turn_root_id = run.turn_root_id.clone();
    // 本轮的中途产出（思考 / 正文）**不在产物格里**：它们属于这一轮的
    // `response(port=model)` 事实（`output_fields`），重建在**应答落格的位置**
    // 折出节点——顺序因此是实时的「思考→卡片→思考→卡片」，不会整体挪到全部
    // 工具卡片之后（见拆分前 2c 的长注释）。
    for (oi, outcome) in outcomes.iter().enumerate() {
        // 产物闸（`ClosureKind::Artifact`，D-067）：不持 `produce.artifact` ⇒
        // 这条结果不入格（warn 一次，不刷屏）。结果照进 prompt / 照记
        // `all_tool_calls`——闸管的是「入格」，不是「发生过」。
        if let Err(why) = closure_gate(&actor, ClosureKind::Artifact) {
            if oi == 0 {
                // 拒绝落否定事实，**只落一次**：`deny` 的幂等键是
                // `deny-{turn}-artifact`，同一轮多条结果说的是同一件事。
                deny(
                    store,
                    &actor,
                    turn,
                    user_seq,
                    ClosureKind::Artifact,
                    crate::symbio_core::EVENT_ARTIFACT_ADDED,
                    &why,
                );
                crate::plugin_warn!(
                    "actors",
                    "[v2] 产物被授权拒绝（{why}），本轮工具结果不入格（turn={turn}）"
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
            })),
        )?;
        run.artifact_no += 1;
        run.artifacts_landed += 1;
    }
    Ok(())
}
