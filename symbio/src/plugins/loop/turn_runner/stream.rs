//! 阶段② 请求 / 流式（T-179 · B3，PL-015 §5 动作 B3）。
//!
//! 一次工具循环迭代：拼请求消息 → 调模型 → 按 [`classify_round`] 分流。
//! 收束（`Halt`）与配置缺口（`ConfigGap`）在这里就地落格；派发（`Act`）把
//! 工具批交给 [`super::batch`]，等待用户的落进展格，其余继续下一轮。
//!
//! **拆分契约**：函数体逐字来自 `run_with_tools` 的 loop 体，`if` / `match`
//! 条件零变化；状态只经 [`Prepared`] 在阶段间传递。

use super::*;

/// 跑**一次**请求。`Ok(None)` = 继续下一轮；`Ok(Some(outcome))` = 本轮已收口。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn stream_round<S>(
    run: &mut Prepared,
    store: &S,
    llm: &dyn LlmAdapter,
    tok: &FullModel,
    sink: std::sync::Arc<dyn crate::symbio_core::DeltaSink>,
    tools: &[crate::symbio_core::CapabilityMeta],
    dispatch: Option<&dyn crate::symbio_core::DispatchPort>,
) -> Result<Option<TurnOutcome>, crate::symbio_core::AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    // 基线 + 本轮交换。**拼接消息数组**：拼字符串会把角色拍平、丢掉 `tool_call_id`。
    let mut messages = run.base_messages.clone();
    messages.extend(run.exchange.iter().cloned());

    // 2. 生成（实测耗时在 adapter 边界取得；失败路径的耗时从调用起点算）。
    match llm.generate_turn(tok, &messages, tools, sink.clone()).await {
        Ok(rt) => {
            run.cost_ms += rt.cost_ms;
            // 每轮的流式节点事实在**分发之前**取定：工具分发的定稿会重置桥，
            // 之后再取只拿得到下一轮的节点——本轮的中途正文 / 思考就静默丢了。
            let round_facts = node_facts(&run.nodes);

            // 2a. 本轮不再请求工具 ⇒ 收束（落 final）。
            if classify_round(&rt, dispatch.is_some()) == RoundDecision::Halt {
                return halt(run, store, llm, rt, &round_facts).await.map(Some);
            }

            // 2b. 模型请求了工具 ⇒ 分发。没有分发方却收到工具调用是**配置
            //     缺口**（无工具轮不该出现工具调用），按失败诚实回报。
            let Some(dispatch) = dispatch else {
                let cost = run.started.elapsed().as_millis() as u64;
                // 调用本身是事实：没有分发通道也要记「请求过什么」。
                run.all_tool_calls
                    .extend(rt.tool_calls.iter().map(|tc| tool_fact_of(tc, None)));
                let produced = produced_payload(
                    &rt.text,
                    llm.model_id(),
                    &run.turn_root_id,
                    &round_facts,
                    &run.all_tool_calls,
                    &run.rounds,
                );
                return settle::append_fallback(
                    store,
                    &run.actor,
                    run.turn,
                    run.user_seq,
                    "模型请求了工具，但本轮没有工具分发通道",
                    cost,
                    produced,
                    run.artifacts_landed,
                )
                .await
                .map(Some);
            };
            let outcomes = dispatch.dispatch(&rt).await;

            // 2c. 产物落格：每条工具结果一格，溯源指向本轮用户格。
            batch::land_artifacts(run, store, &rt, &outcomes, &round_facts).await?;

            // 2d. 收束于等待用户 ⇒ 落**进展格**（`status: "pending"`：模型已
            //     说过的话照样是事实，等待是状态），不收束轮次。
            if outcomes.iter().any(|o| o.needs_user_action) {
                let produced = produced_payload(
                    &rt.text,
                    llm.model_id(),
                    &run.turn_root_id,
                    &round_facts,
                    &run.all_tool_calls,
                    &run.rounds,
                );
                // 进展格的事件 id 按**本轮已有进展格数**编号（同产物格的幂等
                // 模式）：等待轮可能已落过进展格，从 0 起会撞幂等键。
                let progress_no = run
                    .full_snapshot
                    .iter()
                    .filter(|e| e.turn == run.turn && e.kind == EVENT_ASSISTANT_PROGRESS)
                    .count() as u64;
                store.append(
                    Event::pending(
                        format!("p-{}-{progress_no}", run.turn),
                        EVENT_ASSISTANT_PROGRESS,
                        Entity::Turn,
                        Verb::Progressed,
                        run.turn,
                        &run.actor.principal,
                    )
                    .with_produced_by(run.user_seq)
                    .with_cost_ms(run.cost_ms)
                    .with_payload(with_status(produced, "pending")),
                )?;
                return Ok(Some(TurnOutcome {
                    turn: run.turn,
                    text: rt.text,
                    cost_ms: run.cost_ms,
                    usage: rt.usage,
                    fell_back: false,
                    aborted: false,
                    awaits_user: true,
                    closure_denied: false,
                    // 等待轮的收束留给恢复幕，但**本轮已产出的正文**照样带真原因。
                    finish: rt.finish.clone(),
                }));
            }

            // 2e. 把这一轮交换**累加**进 prompt，继续下一轮。
            run.rounds.push(round_facts);
            run.exchange
                .extend(tool_exchange_messages(&rt, &outcomes, run.tool_round));
            run.tool_round += 1;

            // 2f. 轮内注入（生产里是「用户中途补充」）。挂在 2e 之后：工具交换
            // 先接上，注入的内容排在它之后——时间序。
            if let Some(inject) = &run.inject {
                let injected = inject(Some(run.user_seq)).await;
                run.exchange.extend(injected);
            }
            Ok(None)
        }
        Err(AdapterError::Aborted) => aborted(run, store, llm).await.map(Some),
        Err(e) => failed(run, store, llm, &e.to_string()).await.map(Some),
    }
}

/// 2a 收束：答话 / 空答兜底 / 写侧闸拒。
async fn halt<S>(
    run: &mut Prepared,
    store: &S,
    llm: &dyn LlmAdapter,
    rt: LlmTurn,
    round_facts: &StreamNodeFacts,
) -> Result<TurnOutcome, crate::symbio_core::AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    // 答话取值（正文 / 截断 / 端口思考 / 桥上思考）见 [`final_answer`]。
    // `truncated` 另有用处：兜底话术要说清「是截断」还是「空答」。
    let truncated = rt.finish.is_length();
    let answer = final_answer(&rt, round_facts);
    if answer.trim().is_empty() {
        // 正文与思考**都**空 + 无工具调用 = 模型什么也没答。这是**失败**：
        // 落成 final 会把「没答」记成「答了空话」。
        let cost = run.started.elapsed().as_millis() as u64;
        let produced = produced_payload(
            &answer,
            llm.model_id(),
            &run.turn_root_id,
            round_facts,
            &run.all_tool_calls,
            &run.rounds,
        );
        return settle::append_fallback(
            store,
            &run.actor,
            run.turn,
            run.user_seq,
            if truncated {
                "模型输出被长度上限截断，本轮未产出答话"
            } else {
                "model returned empty text"
            },
            cost,
            produced,
            run.artifacts_landed,
        )
        .await;
    }
    if let Err(why) = closure_gate(&run.actor, ClosureKind::Speech) {
        // 写侧闸拒：**不落收束格**——该轮留在未收束态，`check_all` 会把它报出来
        // （C4）。**例外**：本轮落过产物格 ⇒ 已由产物收束（ADR-067）。
        // 拒绝本身落一条否定事实（`authz.denied`），于是「被拒」不再与「没跑完」同形。
        deny(
            store,
            &run.actor,
            run.turn,
            run.user_seq,
            ClosureKind::Speech,
            EVENT_ASSISTANT_FINAL,
            &why,
        );
        crate::plugin_warn!(
            "actors",
            "[v2] 收束被授权拒绝（{why}），本轮不入格（turn={}）",
            run.turn
        );
        return Ok(TurnOutcome {
            turn: run.turn,
            text: answer,
            cost_ms: run.cost_ms,
            usage: rt.usage,
            fell_back: false,
            aborted: false,
            awaits_user: false,
            closure_denied: run.artifacts_landed == 0,
            // 末轮原因随本轮的收束一起带出去（T-144）。
            finish: rt.finish.clone(),
        });
    }
    let produced = produced_payload(
        &answer,
        llm.model_id(),
        &run.turn_root_id,
        round_facts,
        &run.all_tool_calls,
        &run.rounds,
    );
    store.append(
        Event::pending(
            format!("f-{}", run.turn),
            EVENT_ASSISTANT_FINAL,
            Entity::Turn,
            Verb::Closed,
            run.turn,
            &run.actor.principal,
        )
        .with_produced_by(run.user_seq)
        .with_cost_ms(run.cost_ms)
        .with_payload(with_status(produced, "completed")),
    )?;
    Ok(TurnOutcome {
        turn: run.turn,
        text: answer,
        cost_ms: run.cost_ms,
        usage: rt.usage,
        fell_back: false,
        aborted: false,
        awaits_user: false,
        closure_denied: false,
        finish: rt.finish.clone(),
    })
}

/// 3a 中止：照落收束格（`status: "aborted"` + `terminated`）——思考 / 正文是事实，
/// 终止是状态；收束 kind 绝不复用兜底格。
async fn aborted<S>(
    run: &mut Prepared,
    store: &S,
    llm: &dyn LlmAdapter,
) -> Result<TurnOutcome, crate::symbio_core::AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    let cost = run.started.elapsed().as_millis() as u64;
    let facts = node_facts(&run.nodes);
    let partial_text = facts.text.clone().unwrap_or_default();
    if let Err(why) = closure_gate(&run.actor, ClosureKind::Speech) {
        // 中止的收束格同过写侧闸（收束格是同一个事实，不论哪条路径写出）。
        deny(
            store,
            &run.actor,
            run.turn,
            run.user_seq,
            ClosureKind::Speech,
            EVENT_ASSISTANT_FINAL,
            &why,
        );
        crate::plugin_warn!(
            "actors",
            "[v2] 中止收束被授权拒绝（{why}），本轮不入格（turn={}）",
            run.turn
        );
        return Ok(TurnOutcome {
            turn: run.turn,
            text: partial_text,
            cost_ms: cost,
            usage: None,
            fell_back: false,
            aborted: true,
            awaits_user: false,
            closure_denied: run.artifacts_landed == 0,
            // 中止是「用户按了停止」，不是「模型被截断」：长度戳不得误触发。
            finish: crate::symbio_core::ModelFinishReason::Stop,
        });
    }
    let produced = produced_payload(
        &partial_text,
        llm.model_id(),
        &run.turn_root_id,
        &facts,
        &run.all_tool_calls,
        &run.rounds,
    );
    store.append(
        Event::pending(
            format!("f-{}", run.turn),
            EVENT_ASSISTANT_FINAL,
            Entity::Turn,
            Verb::Closed,
            run.turn,
            &run.actor.principal,
        )
        .with_produced_by(run.user_seq)
        .with_cost_ms(cost)
        .with_payload(with_terminated(with_status(produced, "aborted"))),
    )?;
    Ok(TurnOutcome {
        turn: run.turn,
        text: partial_text,
        cost_ms: cost,
        usage: None,
        fell_back: false,
        aborted: true,
        awaits_user: false,
        closure_denied: false,
        // 中止轮不入续写判据（用户按的停止不是被 max_tokens 掐断）。
        finish: crate::symbio_core::ModelFinishReason::Stop,
    })
}

/// 3b 兜底落格（I3：到点必答——失败也是一句话，不是静默）。
async fn failed<S>(
    run: &mut Prepared,
    store: &S,
    llm: &dyn LlmAdapter,
    why: &str,
) -> Result<TurnOutcome, crate::symbio_core::AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    // 失败前已产出的半截正文 / 思考照样进载荷（它们是事实）。
    let cost = run.started.elapsed().as_millis() as u64;
    let facts = node_facts(&run.nodes);
    let produced = produced_payload(
        &facts.text.clone().unwrap_or_default(),
        llm.model_id(),
        &run.turn_root_id,
        &facts,
        &run.all_tool_calls,
        &run.rounds,
    );
    settle::append_fallback(
        store,
        &run.actor,
        run.turn,
        run.user_seq,
        why,
        cost,
        produced,
        run.artifacts_landed,
    )
    .await
}
