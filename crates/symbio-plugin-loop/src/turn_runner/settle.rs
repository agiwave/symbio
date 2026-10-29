//! 收束落格。
//!
//! 失败（兜底格）与中止（收束格）的两条**唯一构造点**。两者都是发言收束，
//! 因此同过写侧闸（[`closure_gate`]）：拒绝 ⇒ 不落格，该轮留在未收束态。
//!
//! 这两个自由函数就是收束格的**唯一构造点**：调用方不各自拼载荷，
//! 闸门、幂等键与 `produced` 形状因此只有一处。

use super::*;

/// 兜底落格（I3）的唯一构造点：失败也是一句话。
///
/// 载荷 = **产出部分**（`produced`，失败前已产出的半截正文 / 思考 / 工具事实——
/// 它们是事实，不因失败就不记）+ `reason: fallback`（闭集，可判）+
/// `detail`（失败原因的人话，只给人读）+ `status: "failed"`。
///
/// `artifacts_landed` = 本轮已新落的产物格数（D-067 的产物收束判据）：兜底格
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
) -> Result<TurnOutcome, symbio_core::AppendError>
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
        symbio_core::ClosureReason::Fallback,
    )
    .await
}

/// 上界耗尽：步数巡回超过上限时落**同一格**，`reason` 不同。
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
) -> Result<TurnOutcome, symbio_core::AppendError>
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
        symbio_core::ClosureReason::Exhausted,
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
    reason: symbio_core::ClosureReason,
) -> Result<TurnOutcome, symbio_core::AppendError>
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
            EVENT_CHAT_ASSISTANT_FALLBACK,
            &why,
        );
        symbio_core::plugin_warn!(
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
            finish: symbio_core::ModelFinishReason::Stop,
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
            EVENT_CHAT_ASSISTANT_FALLBACK,
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
        finish: symbio_core::ModelFinishReason::Stop,
    })
}

/// 定稿收束格的**唯一构造点**（`TC-4`）：`chat.assistant.final` 只在这里落。
///
/// 五条路径写的是同一格——驱动循环里的 `close-turn`、零往返轮（轮首就地收束）、
/// 快路、流式完成、中止。它们**只在何时写、载荷写什么**上不同，事实本身的形状
/// （id `f-{turn}`、`turn × closed`、溯源指向用户格）必须由一处决定：让每处各写
/// 一遍，同一格的字段迟早分叉，而「收束格有几份写方」在事实面上看不出来。
///
/// 写方身份不在这里决定：`actor.principal` 由调用方给，事实照记。声明面照旧只有
/// [`super::super::loop_rows::ACTOR_CLOSE`] 的 `writes` 覆盖这一格——它回答「谁有权
/// 写」，本函数回答「从哪里写出去」。
pub(crate) fn land_final<S>(
    store: &S,
    actor: &ActorSpec,
    turn: u64,
    user_seq: u64,
    cost_ms: u64,
    payload: serde_json::Value,
) -> Result<(), symbio_core::AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    store.append(
        Event::pending(
            format!("f-{turn}"),
            EVENT_CHAT_ASSISTANT_FINAL,
            Entity::Turn,
            Verb::Closed,
            turn,
            &actor.principal,
        )
        .with_produced_by(user_seq)
        .with_cost_ms(cost_ms)
        .with_payload(payload),
    )?;
    Ok(())
}

/// 中止收束格的构造点（D-052 决策 1）：**闸门与首请求之间**到达的中止。
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
) -> Result<TurnOutcome, symbio_core::AppendError>
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
            EVENT_CHAT_ASSISTANT_FINAL,
            &why,
        );
        symbio_core::plugin_warn!(
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
            finish: symbio_core::ModelFinishReason::Stop,
        });
    }
    // 半截产出为空：请求还没发出，模型没产出任何正文 / 思考 / 工具事实。
    let produced = produced_payload(
        "",
        llm.model_id(),
        turn_root_id,
        None,
        &answer_id_of("", "", &StreamNodeFacts::default(), turn_root_id),
        &[],
    );
    land_final(
        store,
        actor,
        turn,
        user_seq,
        0,
        with_terminated(with_reason(
            with_status(produced, "aborted"),
            symbio_core::ClosureReason::Interrupted,
        )),
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
        finish: symbio_core::ModelFinishReason::Stop,
    })
}
