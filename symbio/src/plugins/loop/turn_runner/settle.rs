//! 阶段⑤ 收束落格（T-179 · B3，PL-015 §5 动作 B3）。
//!
//! 失败（兜底格）与中止（收束格）的两条**唯一构造点**。两者都是发言收束，
//! 因此同过写侧闸（[`closure_granted`]）：拒绝 ⇒ 不落格，该轮留在未收束态。
//!
//! 拆分契约：函数体逐字来自 `TurnRunner::append_fallback` / `::append_aborted`，
//! 仅把 `Self::` 改为自由函数调用。

use super::*;

/// 兜底落格（I3）的唯一构造点：失败也是一句话。
///
/// 载荷 = **产出部分**（`produced`，失败前已产出的半截正文 / 思考 / 工具事实——
/// 它们是事实，不因失败就不记）+ `reason: fallback`（闭集，可判）+
/// `detail`（失败原因的人话，只给人读）+ `status: "failed"`。
///
/// `artifacts_landed` = 本轮已新落的产物格数（ADR-067 的产物收束判据）：兜底格
/// 本身也是发言收束，发言闸拒它时，本轮**落过产物**就不算未收束。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn append_fallback<S>(
    store: &S,
    actor: &ActorSpec,
    turn: u64,
    user_seq: u64,
    detail: &str,
    cost_ms: u64,
    produced: serde_json::Value,
    artifacts_landed: u64,
) -> Result<TurnOutcome, crate::symbio_core::AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    land_failure(
        store,
        actor,
        turn,
        user_seq,
        detail,
        cost_ms,
        produced,
        artifacts_landed,
        crate::symbio_core::ClosureReason::Fallback,
    )
    .await
}

/// 上界耗尽（T-227 步 1）：步数巡回超过上限时落**同一格**，`reason` 不同。
///
/// 为什么不另开一格：上界耗尽与失败都是「本轮没说完就停了」，载荷形状一模一样，
/// 差别只在 `reason` 这个闭集值（`exhausted` = 步数 / 预算 / max_tokens 掐断）。
/// 另开一格会让读侧为了同一个问题（「这一轮为什么停」）问两个名字。
///
/// 与 `append_fallback` 共用 [`land_failure`]：闸门、幂等键、`produced` 形状全部一致
/// ——两条故障路径各写一遍落格，必然有一处先漂。
///
/// 入参与 `append_fallback` **逐字一致**（多一个参数都不能接受：两个构造点要能
/// 互换而不改调用点），因此同样带 lint 豁免。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn append_exhausted<S>(
    store: &S,
    actor: &ActorSpec,
    turn: u64,
    user_seq: u64,
    detail: &str,
    cost_ms: u64,
    produced: serde_json::Value,
    artifacts_landed: u64,
) -> Result<TurnOutcome, crate::symbio_core::AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    land_failure(
        store,
        actor,
        turn,
        user_seq,
        detail,
        cost_ms,
        produced,
        artifacts_landed,
        crate::symbio_core::ClosureReason::Exhausted,
    )
    .await
}

/// 两条故障收束（兜底 / 上界）的**唯一落格点**：只差一个 `reason`。
#[allow(clippy::too_many_arguments)]
async fn land_failure<S>(
    store: &S,
    actor: &ActorSpec,
    turn: u64,
    user_seq: u64,
    detail: &str,
    cost_ms: u64,
    produced: serde_json::Value,
    artifacts_landed: u64,
    reason: crate::symbio_core::ClosureReason,
) -> Result<TurnOutcome, crate::symbio_core::AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    if let Err(why) = closure_gate(actor, ClosureKind::Speech) {
        // 兜底格也是**收束格**，同过写侧闸：拒绝 ⇒ 不落格，但**落一条否定事实**
        // （`authz.denied`）——「被拒」与「没跑完」从此在日志上不同形。
        // `fell_back` 仍为真——模型确实失败了（I3 的「到点必答」由调用方按失败呈现）。
        deny(
            store,
            actor,
            turn,
            user_seq,
            ClosureKind::Speech,
            EVENT_ASSISTANT_FALLBACK,
            &why,
        );
        crate::plugin_warn!(
            "actors",
            "[v2] 兜底收束被授权拒绝（{why}），本轮不入格（turn={turn}）"
        );
        return Ok(TurnOutcome {
            turn,
            text: why.to_string(),
            cost_ms,
            usage: None,
            fell_back: true,
            aborted: false,
            awaits_user: false,
            closure_denied: artifacts_landed == 0,
            finish: crate::symbio_core::ModelFinishReason::Stop,
        });
    }
    // `reason` 是闭集里的那个值（可判，由调用方选：`fallback` / `exhausted`），
    // `detail` 是适配器的原始人话（只给人读）。
    // 两件事必须分开：把原始错误塞进 `reason` 会让它不可判，只留闭集又会丢掉
    // 「具体是 402 还是超时」——而那是排障时唯一有用的东西。
    let mut payload = with_reason(with_status(produced, "failed"), reason);
    payload["detail"] = serde_json::Value::String(detail.to_string());
    store.append(
        Event::pending(
            format!("fb-{turn}"),
            EVENT_ASSISTANT_FALLBACK,
            Entity::Turn,
            Verb::Closed,
            turn,
            &actor.principal,
        )
        .with_produced_by(user_seq)
        .with_cost_ms(cost_ms)
        .with_payload(payload),
    )?;
    Ok(TurnOutcome {
        turn,
        // 兜底的那句话就是失败原因本身（I3：到点必答——失败也是一句话）。
        text: detail.to_string(),
        cost_ms,
        usage: None,
        fell_back: true,
        aborted: false,
        awaits_user: false,
        closure_denied: false,
        finish: crate::symbio_core::ModelFinishReason::Stop,
    })
}

/// 中止收束格的构造点（ADR-052 决策 1）：**闸门与首请求之间**到达的中止。
///
/// 与 adapter 中途返回 `AdapterError::Aborted` 落的是**同一格**
/// （`chat.assistant.final` + `status: "aborted"` + `terminated`）——终止是事实的
/// **状态**，不是新种类。
pub(crate) async fn append_aborted<S>(
    store: &S,
    actor: &ActorSpec,
    turn: u64,
    user_seq: u64,
    llm: &dyn LlmAdapter,
    turn_root_id: &str,
) -> Result<TurnOutcome, crate::symbio_core::AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    if let Err(why) = closure_gate(actor, ClosureKind::Speech) {
        // 中止的收束格同过写侧闸，拒绝同样落否定事实（与兜底臂同一形态）。
        deny(
            store,
            actor,
            turn,
            user_seq,
            ClosureKind::Speech,
            EVENT_ASSISTANT_FINAL,
            &why,
        );
        crate::plugin_warn!(
            "actors",
            "[v2] 中止收束被授权拒绝（{why}），本轮不入格（turn={turn}）"
        );
        return Ok(TurnOutcome {
            turn,
            text: String::new(),
            cost_ms: 0,
            usage: None,
            fell_back: false,
            aborted: true,
            awaits_user: false,
            closure_denied: true,
            finish: crate::symbio_core::ModelFinishReason::Stop,
        });
    }
    // 半截产出为空：请求还没发出，模型没产出任何正文 / 思考 / 工具事实。
    let produced = produced_payload(
        "",
        llm.model_id(),
        turn_root_id,
        &StreamNodeFacts::default(),
        &[],
        &[],
    );
    store.append(
        Event::pending(
            format!("f-{turn}"),
            EVENT_ASSISTANT_FINAL,
            Entity::Turn,
            Verb::Closed,
            turn,
            &actor.principal,
        )
        .with_produced_by(user_seq)
        .with_cost_ms(0)
        .with_payload(with_terminated(with_reason(
            with_status(produced, "aborted"),
            crate::symbio_core::ClosureReason::Interrupted,
        ))),
    )?;
    Ok(TurnOutcome {
        turn,
        text: String::new(),
        cost_ms: 0,
        usage: None,
        fell_back: false,
        aborted: true,
        awaits_user: false,
        closure_denied: false,
        finish: crate::symbio_core::ModelFinishReason::Stop,
    })
}
