//! 阶段② 请求 / 流式（T-179 · B3，PL-015 §5 动作 B3）。
//!
//! 一次工具循环迭代：拼请求消息 → 调模型 → 按 [`classify_round`] 分流。
//! 收束（`Halt`）与配置缺口（`ConfigGap`）在这里就地落格；派发（`Act`）把
//! 工具批交给 [`super::batch`]，等待用户的落进展格，其余继续下一轮。
//!
//! **拆分契约**：函数体逐字来自 `run_with_tools` 的 loop 体，`if` / `match`
//! 条件零变化；状态只经 [`Prepared`] 在阶段间传递。

use super::*;

/// 一次端口调用的失败：三类，各有各的收口。
///
/// 为什么要分类而不是直接抛 `AdapterError`：中止与失败是**两种停法**
/// （收束格 vs 兜底格），而控制面不一致（行不欠 / 端口没声明）**根本不是适配器的
/// 错**——把它们揉进一个错误类型，就只能给它们同一个形状。
///
/// 两个端口（`model` / `tool`）共用它：**分类的口径与端口无关**——端口只决定
/// 落哪几格协议事实，不决定「失败该怎么收束」。
enum PortCallError {
    /// 适配器中止（`AdapterError::Aborted`）——中止收束格。
    Aborted,
    /// 适配器失败——兜底格（`detail` 是适配器的人话，只给人读）。
    Failed(String),
    /// 协议事实落不进去——往上传：事件都记不下来，后面的一切都不可信。
    Store(crate::symbio_core::AppendError),
    /// 控制面自身不一致（行不欠 / 端口没声明）——执行体没跑成，如实当失败报。
    Control(String),
}

/// 决策者写 `request` 并问表：**端口无关的那一半**（announce + 行还欠不欠）。
///
/// 两个端口（`model` / `tool`）共用它，于是「先落 `request`、再拿刚写完的那份切片
/// 问表」这条顺序只写一次——它是个容易写反的地方（先问表再落 `request`，行恒不欠，
/// 于是每次派发都被拒），而写反了不会有任何东西报错。
fn open_call<S>(
    store: &S,
    call: &super::super::port::Call,
    row: &str,
    turn: u64,
    produced_by: u64,
) -> Result<bool, PortCallError>
where
    S: Store<Event = Event> + ?Sized,
{
    super::super::port::announce(store, call, turn, produced_by).map_err(PortCallError::Store)?;
    // 派发前的事实切片必须是**刚写完 `request` 的那一份**：行欠不欠看它。
    let facts = store.range(Seq::new(0));
    Ok(super::super::loop_rows::owes(
        row,
        &facts,
        turn,
        crate::symbio_core::clock_now_ms(),
    ))
}

/// 一次模型往返 = 端口 `model` 的一次调用（T-227 步 2）。
///
/// 协议事实由 [`port`](crate::plugins::loop::port) 模块写（**唯一写方**）：
/// `request`（决策者：这一步欠一次模型往返）→ `sent`（引擎，跑**之前**）→
/// `response` / `failed`。
///
/// ## 为什么模型调用要落事实
///
/// 粗端口 `turn` 的粒度是**一轮**：崩溃后只能重放整轮。模型调用是重放的**最小
/// 单位**——落成事实之后，“上次发到第几次模型请求”有答案，而那一轮里的工具批次
/// 也能按它对齐。
///
/// ## 为什么逻辑 id 带**幕锚**与**步号**
///
/// 续写幕复用轮号且步号从 0 重来，只带轮号会与首幕撞成同一笔——首幕已经有过
/// `sent`，幂等键命中之后派发会被拒（与 T-223 的 `turn` 端口同一条理由）。
///
/// ## 为什么 id 以**端口名**开头
///
/// 视图 `in_flight` / `stale_sent` 的关账口径按 **id** 匹配，不带端口
/// （`open_calls`：`response` / `retry` 关同 id + attempt，`failed` 关同逻辑 id）
/// ——也就是说，**id 必须跨端口唯一**，否则一个端口的失败会关掉另一个端口的账。
/// 以端口名开头是最省心的做法：`turn-…` / `model-…` / `tool-…` 天然不撞。
async fn call_model<S>(
    run: &Prepared,
    store: &S,
    llm: &dyn LlmAdapter,
    tok: &FullModel,
    messages: &[crate::symbio_core::PromptMessage],
    tools: &[crate::symbio_core::CapabilityMeta],
    sink: std::sync::Arc<dyn crate::symbio_core::DeltaSink>,
) -> Result<crate::symbio_core::LlmTurn, PortCallError>
where
    S: Store<Event = Event> + ?Sized,
{
    let call = super::super::port::Call::new(
        "model",
        format!("model-{}-d{}-s{}", run.turn, run.drive_seq, run.step),
        1,
    );
    let owed = open_call(store, &call, "dispatch-model", run.turn, run.user_seq)?;
    let out = super::super::port::dispatch(
        store,
        super::super::loop_rows::PORTS,
        super::super::port::Dispatch {
            call: &call,
            turn: run.turn,
            produced_by: run.user_seq,
            owed,
        },
        || llm.generate_turn(tok, messages, tools, sink.clone()),
        |e: &AdapterError| e.to_string(),
    );
    match out.await {
        Ok(rt) => Ok(rt),
        Err(super::super::port::DispatchError::Actor(AdapterError::Aborted)) => {
            Err(PortCallError::Aborted)
        }
        Err(super::super::port::DispatchError::Actor(e)) => {
            Err(PortCallError::Failed(e.to_string()))
        }
        Err(super::super::port::DispatchError::Store(e)) => Err(PortCallError::Store(e)),
        Err(other) => Err(PortCallError::Control(format!("{other:?}"))),
    }
}

/// 一次工具批 = 端口 `tool` 的一次调用（T-227 步 3）。
///
/// 协议事实与前两个端口**同一套**（`request` / `sent` / `response` / `failed`），
/// 写方仍是 [`port`](crate::plugins::loop::port) 模块。
///
/// ## 为什么调用单元是「批」而不是「单次调用」
///
/// 执行体是**批原子**的：批首闸门读数一次、交互模式下一条待用户动作就收掉批尾、
/// 批尾未执行的调用统一收口。切到单次调用会得到 N 笔无法独立派发的债——记账比
/// 执行更细，而「更细」在这里不是精度，是**假象**（PL-017 §3.1 的 `expand` 是
/// 为「执行体按调用跑」准备的形状，那一半还不存在）。单次调用的结果本来就逐条
/// 是事实（`artifact.added` 带 `tool_call_id`），恢复面因此没有信息损失。
///
/// ## 逻辑 id 带**批号**
///
/// `b{tool_round}`：一次响应一个批，批号在本幕内单调。幕锚与轮号仍然要带——
/// 续写幕复用轮号且批号从 0 重来。
async fn call_tools<S>(
    run: &Prepared,
    store: &S,
    dispatch: &dyn crate::symbio_core::DispatchPort,
    rt: &LlmTurn,
) -> Result<Vec<crate::symbio_core::DispatchOutcome>, PortCallError>
where
    S: Store<Event = Event> + ?Sized,
{
    let call = super::super::port::Call::new(
        "tool",
        format!("tool-{}-d{}-b{}", run.turn, run.drive_seq, run.tool_round),
        1,
    );
    let owed = open_call(store, &call, "dispatch-tool", run.turn, run.user_seq)?;
    let out = super::super::port::dispatch(
        store,
        super::super::loop_rows::PORTS,
        super::super::port::Dispatch {
            call: &call,
            turn: run.turn,
            produced_by: run.user_seq,
            owed,
        },
        // 分发契约本身不可失败（失败是**信息性结果**：工具报错要喂回模型，不是
        // 派发失败）。`E = String` 只是给引擎一个可写的 `why` 位，不会有人返回它。
        || async { Ok::<_, String>(dispatch.dispatch(rt).await) },
        |e: &String| e.clone(),
    );
    match out.await {
        Ok(outcomes) => Ok(outcomes),
        Err(super::super::port::DispatchError::Store(e)) => Err(PortCallError::Store(e)),
        // 行不欠 / 端口没声明：批**没有跑**。如实当失败报——静默跳过工具会让模型
        // 在下一轮看到「请求过却什么也没发生」。
        Err(other) => Err(PortCallError::Failed(format!("{other:?}"))),
    }
}

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
    //
    // 这一次模型往返是**端口 `model` 的一次调用**（T-227 步 2）：协议事实
    // （`request` / `sent` / `response` / `failed`）由 `port` 模块写（唯一写方）。
    let rt = match call_model(run, store, llm, tok, &messages, tools, sink.clone()).await {
        Ok(rt) => rt,
        Err(PortCallError::Aborted) => return aborted(run, store, llm).await.map(Some),
        Err(PortCallError::Failed(why)) => {
            return failed(run, store, llm, &why).await.map(Some);
        }
        Err(PortCallError::Store(e)) => return Err(e),
        // 控制面自身不一致（行不欠 / 端口没声明）：模型没跑成，如实当失败报。
        Err(PortCallError::Control(why)) => {
            return failed(run, store, llm, &why).await.map(Some);
        }
    };
    run.cost_ms += rt.cost_ms;
    // 步号 +1：下一次模型往返换一个逻辑 id（否则第二次的 `sent` 撞第一次的幂等键）。
    run.step += 1;
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
    // 2b′. 工具批也是一次端口调用（T-227 步 3）：`request` → `sent` → 跑批 →
    //      `response`。**协议事实先于产物格**：批返回之后，它的结果才成为事实
    //      （`artifact.added` 在 2c 落）。
    let outcomes = match call_tools(run, store, dispatch, &rt).await {
        Ok(outcomes) => outcomes,
        Err(PortCallError::Store(e)) => return Err(e),
        // 批没有跑成（控制面不一致 / 引擎判不欠）：如实落兜底格，不静默跳过工具。
        Err(PortCallError::Failed(why)) | Err(PortCallError::Control(why)) => {
            return failed(run, store, llm, &why).await.map(Some);
        }
        // 分发契约不可中止；万一将来的实现返回中止，走与模型端口同一条收束路。
        Err(PortCallError::Aborted) => return aborted(run, store, llm).await.map(Some),
    };

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
    if let Verdict::Denied(why) = allow(
        store,
        &run.actor,
        run.turn,
        run.user_seq,
        ClosureKind::Speech,
        EVENT_ASSISTANT_FINAL,
    ) {
        // 授权级联拒：**不落收束格**——该轮留在未收束态，`check_all` 会把它报出来
        // （C4）。**例外**：本轮落过产物格 ⇒ 已由产物收束（ADR-067）。
        // 拒绝本身已落一条否定事实（`authz.denied`），于是「被拒」不再与「没跑完」同形。
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
        // `reason` 由 `finish` 折出：被 max_tokens 掐断的一轮**答出来了但没答完**，
        // 它与自然说完在 `status` 上同形（都是 completed）、在 `reason` 上不同形。
        .with_payload(with_reason(
            with_status(produced, "completed"),
            reason_of_finish(&rt.finish),
        )),
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
    if let Verdict::Denied(why) = allow(
        store,
        &run.actor,
        run.turn,
        run.user_seq,
        ClosureKind::Speech,
        EVENT_ASSISTANT_FINAL,
    ) {
        // 中止的收束格同过授权级联（收束格是同一个事实，不论哪条路径写出）。
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
        .with_payload(with_terminated(with_reason(
            with_status(produced, "aborted"),
            crate::symbio_core::ClosureReason::Interrupted,
        ))),
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
