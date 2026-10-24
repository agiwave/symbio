//! 阶段② 请求 / 流式（· B3，DE-021 §5 动作 B3）。
//!
//! 一次工具循环迭代：拼请求消息 → 调模型 → 按 [`classify_round`] 分流。
//! 收束（`Halt`）与配置缺口（`ConfigGap`）在这里就地落格；派发（`Act`）把
//! 工具批交给 [`super::batch`]，等待用户的落进展格，其余继续下一轮。
//!
//! **拆分契约**：函数体逐字来自 `run_with_tools` 的 loop 体，`if` / `match`
//! 条件零变化；状态只经 [`Prepared`] 在阶段间传递。

use super::*;

/// 一次模型往返的失败：三类，各有各的收口。
///
/// 为什么要分类而不是直接抛 `AdapterError`：中止与失败是**两种停法**
/// （收束格 vs 兜底格），而控制面不一致（行不欠 / 端口没声明）**根本不是适配器的
/// 错**——把它们揉进一个错误类型，就只能给它们同一个形状。
enum ModelCallError {
    /// 适配器中止（`AdapterError::Aborted`）——中止收束格。
    Aborted,
    /// 适配器失败——兜底格（`detail` 是适配器的人话，只给人读）。
    Failed(String),
    /// 协议事实落不进去——往上传：事件都记不下来，后面的一切都不可信。
    Store(crate::symbio_core::AppendError),
    /// 控制面自身不一致（行不欠 / 端口没声明）——模型没跑成，如实当失败报。
    Control(String),
}

/// 一次模型往返 = 端口 `model` 的一次调用。
///
/// 协议事实由 `port` 模块写（**唯一写方**）：
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
/// `sent`，幂等键命中之后派发会被拒（与 的 `turn` 端口同一条理由）。
///
/// ## 为什么 id 以**端口名**开头
///
/// 视图 `in_flight` / `stale_sent` 的关账口径按 **id** 匹配，不带端口
/// （`open_calls`：`response` / `retry` 关同 id + attempt，`failed` 关同逻辑 id）
/// ——也就是说，**id 必须跨端口唯一**，否则一个端口的失败会关掉另一个端口的账。
/// 以端口名开头是最省心的做法：`turn-…` 与 `model-…` 天然不撞。
async fn call_model<S>(
    run: &mut Prepared,
    store: &S,
    llm: &dyn LlmAdapter,
    tok: &FullModel,
    messages: &[crate::symbio_core::PromptMessage],
    tools: &[crate::symbio_core::CapabilityMeta],
    sink: std::sync::Arc<dyn crate::symbio_core::DeltaSink>,
) -> Result<crate::symbio_core::LlmTurn, ModelCallError>
where
    S: Store<Event = Event> + ?Sized,
{
    let call = super::super::port::Call::new(
        "model",
        format!("model-{}-d{}-s{}", run.turn, run.drive_seq, run.step),
        1,
    );
    let turn = run.turn;
    let drive = run.drive_seq;
    let round = call.id.clone();
    super::super::port::announce(store, &call, run.turn, run.user_seq)
        .map_err(ModelCallError::Store)?;
    // 应答载荷带上本次交回的工具调用（D-117）：创建行 `open-tools` 的集合
    // 展开读 `calls`；元素 id 与随后的 `request(port=tool)` 用同一公式。
    // 行欠不欠由引擎现读（D-136）：这里只交出「授权这次派发的行」与读账的 now。
    let out = super::super::port::dispatch_with(
        store,
        super::super::loop_rows::PORTS,
        super::super::port::Dispatch {
            call: &call,
            turn: run.turn,
            produced_by: run.user_seq,
            row: "dispatch-model",
            now: run.now_ms(),
            // 这次往返等在当前 drive 的 future 里：进程一死等待方就没了。
            awaited: false,
        },
        || llm.generate_turn(tok, messages, tools, sink.clone()),
        |e: &AdapterError| e.to_string(),
        |rt| {
            let calls: Vec<serde_json::Value> = rt
                .tool_calls
                .iter()
                .enumerate()
                .map(|(idx, tc)| {
                    serde_json::json!({
                        "port": "tool",
                        "id": super::super::tool_dispatch::batch_call_id(
                            turn,
                            drive,
                            idx,
                            tc.id.as_deref(),
                            &round,
                        ),
                        "name": tc.name.clone().unwrap_or_default(),
                        "arguments": tc.arguments.clone(),
                    })
                })
                .collect();
            serde_json::json!({ "calls": calls })
        },
    );
    match out.await {
        Ok(rt) => Ok(rt),
        Err(super::super::port::DispatchError::Actor(AdapterError::Aborted)) => {
            Err(ModelCallError::Aborted)
        }
        Err(super::super::port::DispatchError::Actor(e)) => {
            Err(ModelCallError::Failed(e.to_string()))
        }
        Err(super::super::port::DispatchError::Store(e)) => Err(ModelCallError::Store(e)),
        Err(other) => Err(ModelCallError::Control(format!("{other:?}"))),
    }
}

/// 一次模型往返的推进结果。
pub(crate) enum ModelRound {
    /// 本轮已收口（兜底 / 中止 / 控制面失败——都没有可收束的应答）。
    Done(TurnOutcome),
    /// 模型请求了工具；应答与节点事实已寄存在 `Prepared`，等 [`tool_round`]。
    Tools,
    /// 本轮该收束了；**结局**寄存在 [`Prepared::closing`]，落格归 `close-turn`
    /// 行的执行体（/ 步 5c）。
    ///
    /// 三种收法共用这一个出口：答完了、模型没答成、中止——它们落的格子不同，
    /// 但**都是「这一轮该收了」**，而收束格只由 `close` 执行体写。
    Close,
}

/// 阶段②a：跑**一次**模型往返并分类；请求工具时把应答寄存，不派发。
///
/// 与 [`dispatch_round`] 拆开是 的第一步：引擎要能分别选中「模型往返」与
/// 「工具批次」两笔债，执行体就不能再把它们揉在一次调用里。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn model_round<S>(
    run: &mut Prepared,
    store: &S,
    llm: &dyn LlmAdapter,
    tok: &FullModel,
    sink: std::sync::Arc<dyn crate::symbio_core::DeltaSink>,
    tools: &[crate::symbio_core::CapabilityMeta],
    has_dispatch: bool,
) -> Result<ModelRound, crate::symbio_core::AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    // 基线 + 本轮交换。**拼接消息数组**：拼字符串会把角色拍平、丢掉 `tool_call_id`。
    let mut messages = run.base_messages.clone();
    messages.extend(run.exchange.iter().cloned());

    // 2. 生成（实测耗时在 adapter 边界取得；失败路径的耗时从调用起点算）。
    //
    // 这一次模型往返是**端口 `model` 的一次调用**：协议事实
    // （`request` / `sent` / `response` / `failed`）由 `port` 模块写（唯一写方）。
    let rt = match call_model(run, store, llm, tok, &messages, tools, sink.clone()).await {
        Ok(rt) => rt,
        // 中止：收束格（`status: "aborted"`）由收束动作落，这里只寄存结局。
        Err(ModelCallError::Aborted) => {
            let (cost_ms, produced, text) = aborted_payload(run, llm);
            run.closing = Some(PendingClose::Aborted {
                cost_ms,
                produced,
                text,
            });
            return Ok(ModelRound::Close);
        }
        // 适配器失败：兜底格由收束动作落，这里只寄存结局。
        Err(ModelCallError::Failed(why)) => {
            let (cost_ms, produced) = failed_payload(run, llm);
            run.closing = Some(PendingClose::Failed {
                detail: why,
                cost_ms,
                produced,
            });
            return Ok(ModelRound::Close);
        }
        Err(ModelCallError::Store(e)) => return Err(e),
        // 控制面自身不一致（行不欠 / 端口没声明）：模型**压根没派发**，
        // 端口没有终局事实（`failed(port=model)` 落不下来）⇒ 收束行开不出账
        // ⇒ 兜底格只能就地落（`execute` 执行体的过渡豁免，见 `loop_rows::ACTORS`）。
        Err(ModelCallError::Control(why)) => {
            let (cost_ms, produced) = failed_payload(run, llm);
            return settle::append_fallback(
                store,
                &run.actor,
                run.turn,
                run.user_seq,
                &why,
                cost_ms,
                produced,
                run.artifacts_landed,
            )
            .await
            .map(ModelRound::Done);
        }
    };
    run.cost_ms += rt.cost_ms;
    // 步号 +1：下一次模型往返换一个逻辑 id（否则第二次的 `sent` 撞第一次的幂等键）。
    run.step += 1;
    // 每轮的流式节点事实在**分发之前**取定：工具分发的定稿会重置桥，
    // 之后再取只拿得到下一轮的节点——本轮的中途正文 / 思考就静默丢了。
    let round_facts = node_facts(&run.nodes);

    // 2a. 本轮不再请求工具 ⇒ 该收束了。**不在这里落收束格**：
    //     `close-turn` 行的前件（`turn.closeable`）此刻已经在日志上，派发器下一步
    //     就会选中它，把收束交给 `closer` 执行体。
    if classify_round(&rt, has_dispatch) == RoundDecision::Halt {
        run.closing = Some(PendingClose::Answer {
            rt,
            facts: round_facts,
        });
        return Ok(ModelRound::Close);
    }

    // 2b. 模型请求了工具 ⇒ 寄存。没有分发方却收到工具调用是**配置
    //     缺口**（无工具轮不该出现工具调用），按失败诚实回报。
    //
    //     这一格**就地落**（不走收束动作）：工具批开不出来这件事在日志上没有
    //     任何事实（`failed(port=model)` 落不下来——模型应答是好的，缺的是分发
    //     通道），收束行因此判不出「该收束了」；它由 `execute` 执行体的**过渡
    //     豁免**兜着（`loop_rows::ACTORS`）。生产里这一路不可达（模型调用点恒带
    //     分发通道）。
    if !has_dispatch {
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
        .map(ModelRound::Done);
    }

    run.pending_model = Some(rt);
    run.pending_round = Some(round_facts);
    Ok(ModelRound::Tools)
}

/// 阶段②c：收束寄存的**结局**（/ 步 5c，`close-turn` 行的动作）。
///
/// 前件由日志说（本幕要么「答完了」、要么「模型没答成」），落格在这里做。
/// 没有寄存 ⇒ 引擎收错了账：不落任何格、返回 `None`，让派发器的「无进展」判据把
/// 它显形——不拿一份来历不明的草稿硬造一条收束。
pub(crate) async fn close_round<S>(
    run: &mut Prepared,
    store: &S,
    llm: &dyn LlmAdapter,
) -> Result<Option<TurnOutcome>, crate::symbio_core::AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    let Some(pending) = run.closing.take() else {
        return Ok(None);
    };
    match pending {
        PendingClose::Answer { rt, facts } => halt(run, store, llm, rt, &facts).await.map(Some),
        PendingClose::Failed {
            detail,
            cost_ms,
            produced,
        } => settle::append_fallback(
            store,
            &run.actor,
            run.turn,
            run.user_seq,
            &detail,
            cost_ms,
            produced,
            run.artifacts_landed,
        )
        .await
        .map(Some),
        PendingClose::Aborted {
            cost_ms,
            produced,
            text,
        } => land_aborted(run, store, cost_ms, produced, &text)
            .await
            .map(Some),
    }
}

/// 阶段②b-1：创建（`open-tools` 行的动作， 步 5b）。
///
/// 把模型交回的调用逐条开成 `request(port=tool)`，开出的账寄存在草稿里交给授权步。
/// 没有寄存的模型应答 ⇒ 引擎选错了行：不写任何格，派发器的「无进展」判据会显形。
pub(crate) async fn open_round(
    run: &mut Prepared,
    store: &dyn Store<Event = Event>,
    dispatch: &dyn super::super::tool_dispatch::ToolDispatch,
) {
    let Some(rt) = run.pending_model.as_ref() else {
        return;
    };
    let calls = dispatch.open(store, rt).await;
    run.pending_calls = Some(calls);
}

/// 阶段②b-2：授权（`authorize-tool` 行的动作）：为已开账的调用落放行 / 拒绝。
pub(crate) async fn authorize_round(
    run: &mut Prepared,
    store: &dyn Store<Event = Event>,
    dispatch: &dyn super::super::tool_dispatch::ToolDispatch,
) {
    match run.pending_calls.as_mut() {
        Some(calls) => dispatch.authorize(store, calls).await,
        None => {
            // 恢复幕：草稿里没有模型批次，动作自己去读开幕前列队的唤醒请求。
            let mut none: Vec<super::super::tool_dispatch::ToolPortCall> = Vec::new();
            dispatch.authorize(store, &mut none).await;
        }
    }
}

/// 阶段②b-3：派发（`dispatch-tools` / `deny-tools` 两行的动作）：落 `sent`、
/// 跑执行层、关账，落产物 / 等待 / 交换。
///
/// `Ok(None)` = 继续下一轮；`Ok(Some)` = 本轮收束（等待用户）。
///
/// 两行共用这一个动作：混合批次里 `dispatch-tools` 先被选中，它顺手把被拒的那几笔
/// 也收尾（`close_batch`）；一批全被拒时 `dispatch-tools` 不欠，`deny-tools` 选中
/// 本动作，仍然要走执行层（审批卡 / 拒绝反馈由它唯一构造）。
pub(crate) async fn dispatch_round(
    run: &mut Prepared,
    store: &dyn Store<Event = Event>,
    llm: &dyn LlmAdapter,
    dispatch: &dyn super::super::tool_dispatch::ToolDispatch,
) -> Result<Option<TurnOutcome>, crate::symbio_core::AppendError> {
    // 三格齐 ⇒ 正常批次；三格皆空 ⇒ 可能是一笔唤醒（恢复幕没有模型应答）。
    // 只缺一两格是草稿错位，不拿唤醒兜底掩盖它。
    let normal = match (
        run.pending_model.take(),
        run.pending_round.take(),
        run.pending_calls.take(),
    ) {
        (Some(rt), Some(round_facts), Some(calls)) => Some((rt, Some(round_facts), calls)),
        _ => None,
    };
    let (rt, round_facts, mut calls) = match normal {
        Some(parts) => parts,
        None => match dispatch.wake(store).await {
            Some(batch) => (batch.turn, None, batch.calls),
            // 没有寄存、也没有唤醒请求 = 引擎没有可派的工具批；不空转。
            None => return Ok(None),
        },
    };
    let outcomes = dispatch.dispatch(store, &rt, &mut calls).await;

    // 2c. 产物落格：每条工具结果一格，溯源指向本轮用户格。
    batch::land_artifacts(run, store, &rt, &outcomes, round_facts.as_ref()).await?;

    // 2d. 收束于等待用户 ⇒ 落**进展格**（`status: "pending"`：模型已
    //     说过的话照样是事实，等待是状态），不收束轮次。
    if outcomes.iter().any(|o| o.needs_user_action) {
        let empty = StreamNodeFacts::default();
        let produced = produced_payload(
            &rt.text,
            llm.model_id(),
            &run.turn_root_id,
            round_facts.as_ref().unwrap_or(&empty),
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
    // 唤醒幕没有模型往返（`round_facts` 为 `None`）：它本来就没有节点事实可累加。
    if let Some(round_facts) = round_facts {
        run.rounds.push(round_facts);
    }
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
    if let Err(why) = closure_gate(&run.actor, ClosureKind::Speech) {
        // 写侧闸拒：**不落收束格**——该轮留在未收束态，`check_all` 会把它报出来
        // （C4）。**例外**：本轮落过产物格 ⇒ 已由产物收束（D-067）。
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
            // 末轮原因随本轮的收束一起带出去。
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

/// 3a 中止的**载荷**（c）：半截产出与实测耗时。
///
/// 载荷与落格分开：收束格只由 `close` 执行体写，中止那一步只负责把结局寄存下来。
fn aborted_payload(run: &Prepared, llm: &dyn LlmAdapter) -> (u64, serde_json::Value, String) {
    let cost_ms = run.started.elapsed().as_millis() as u64;
    let facts = node_facts(&run.nodes);
    let text = facts.text.clone().unwrap_or_default();
    let produced = produced_payload(
        &text,
        llm.model_id(),
        &run.turn_root_id,
        &facts,
        &run.all_tool_calls,
        &run.rounds,
    );
    (cost_ms, produced, text)
}

/// 3a 中止：照落收束格（`status: "aborted"` + `terminated`）——思考 / 正文是事实，
/// 终止是状态；收束 kind 绝不复用兜底格。
async fn land_aborted<S>(
    run: &mut Prepared,
    store: &S,
    cost_ms: u64,
    produced: serde_json::Value,
    partial_text: &str,
) -> Result<TurnOutcome, crate::symbio_core::AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
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
            text: partial_text.to_string(),
            cost_ms,
            usage: None,
            fell_back: false,
            aborted: true,
            awaits_user: false,
            closure_denied: run.artifacts_landed == 0,
            // 中止是「用户按了停止」，不是「模型被截断」：长度戳不得误触发。
            finish: crate::symbio_core::ModelFinishReason::Stop,
        });
    }
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
        .with_cost_ms(cost_ms)
        .with_payload(with_terminated(with_reason(
            with_status(produced, "aborted"),
            crate::symbio_core::ClosureReason::Interrupted,
        ))),
    )?;
    Ok(TurnOutcome {
        turn: run.turn,
        text: partial_text.to_string(),
        cost_ms,
        usage: None,
        fell_back: false,
        aborted: true,
        awaits_user: false,
        closure_denied: false,
        // 中止轮不入续写判据（用户按的停止不是被 max_tokens 掐断）。
        finish: crate::symbio_core::ModelFinishReason::Stop,
    })
}

/// 3b 兜底的**载荷**（c）：失败前已产出的半截产出与实测耗时。
///
/// 失败前已产出的半截正文 / 思考照样进载荷（它们是事实，不因失败就不记）。
fn failed_payload(run: &Prepared, llm: &dyn LlmAdapter) -> (u64, serde_json::Value) {
    let cost_ms = run.started.elapsed().as_millis() as u64;
    let facts = node_facts(&run.nodes);
    let produced = produced_payload(
        &facts.text.clone().unwrap_or_default(),
        llm.model_id(),
        &run.turn_root_id,
        &facts,
        &run.all_tool_calls,
        &run.rounds,
    );
    (cost_ms, produced)
}
