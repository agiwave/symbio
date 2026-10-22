//! 阶段① 轮首准备（T-179 · B3，PL-015 §5 动作 B3）。
//!
//! 阶段事实与派发档入格 → 开场白 / 定稿答话两条早退路 → 窗口与基线 prompt →
//! 续写产物补写 → 首请求前的中止检查。产出的 [`Prepared`] 是后续阶段的**唯一
//! 状态来源**：请求阶段只读它、批处理阶段只改它，阶段之间不传第二份事实。
//!
//! 用户格**不在这里落**（T-225）：它先于任何判决，由编排层在 `drive` 之前写入，
//! 本阶段收到的是它的 seq（`TurnInput::user_seq`）。
//!
//! **纯结构拆分契约**：本文件体内的顺序、判据与副作用逐字来自
//! `run_with_tools` 的第一段；早退（定稿答话 / 空答兜底 / 首请求前中止）在
//! `PreparedFlow::Done` 里返回，语义与拆分前的 `return` 相同。

use super::*;

/// 轮首准备的两条出口：继续驱动，或本轮已收口（定稿答话 / 空答兜底 / 中止）。
pub(crate) enum PreparedFlow {
    Run(Box<Prepared>),
    Done(TurnOutcome),
}

/// 一次工具循环的**在途状态**（阶段①产出，阶段②③④读取/推进）。
pub(crate) struct Prepared {
    pub(crate) turn: u64,
    /// 本幕锚（`drive_seq`）：续写幕复用轮号，端口的逻辑 id 必须带它——只带轮号会与
    /// 首幕撞成同一笔（首幕已经有过 `sent`，幂等键命中之后派发会被拒）。
    ///
    /// 它同时是**本次 drive 的时间锚**：幕首落的那条唤醒事实就在这个 seq 上
    /// （见 [`Prepared::now_ms`]）。
    pub(crate) drive_seq: u64,
    /// 轮内**步号**（1 起）：第几次模型往返。模型端口的逻辑 id 带它——
    /// 一次往返一个 id，否则第二次的 `sent` 撞第一次的幂等键。
    pub(crate) step: u64,
    pub(crate) actor: ActorSpec,
    pub(crate) user_seq: u64,
    pub(crate) turn_root_id: String,
    pub(crate) started: std::time::Instant,
    pub(crate) base_messages: Vec<crate::symbio_core::PromptMessage>,
    pub(crate) exchange: Vec<crate::symbio_core::PromptMessage>,
    pub(crate) tool_round: usize,
    pub(crate) cost_ms: u64,
    pub(crate) artifact_no: u64,
    pub(crate) artifacts_landed: u64,
    pub(crate) all_tool_calls: Vec<ToolCallFact>,
    pub(crate) rounds: Vec<StreamNodeFacts>,
    pub(crate) full_snapshot: Vec<Event>,
    pub(crate) inject: Option<RoundInjector>,
    pub(crate) nodes: Option<NodeFactsFn>,
    /// 模型应答已落、工具批次尚未派发时的寄存位（T-233 拆步）。
    ///
    /// 放在 `Prepared` 而不是新开一份状态：它和消息 / 节点 / 成本同属**本轮草稿**，
    /// 引擎不读它；拆步只是把「一次 `step` 做两件事」拆成两个可分别调用的动作。
    pub(crate) pending_model: Option<LlmTurn>,
    /// 与 [`Self::pending_model`] 同一次模型往返的流式节点事实（工具落格要用）。
    pub(crate) pending_round: Option<StreamNodeFacts>,
}

impl Prepared {
    /// 本次 `drive` 的 `now`：幕锚那条**唤醒事实**的 `ts`（判据 12 / PL-016 §2.1）。
    ///
    /// 切片取准备段读到的那一份，而用户格（或恢复格）在进运行器之前就落了 ⇒
    /// 幕锚一定在里面。不在 ⇒ 0：宁可判「还没有任何时间条件成立」。
    pub(crate) fn now_ms(&self) -> i64 {
        super::super::loop_rows::wake_now_ms(&self.full_snapshot, self.drive_seq)
    }
}

pub(crate) async fn prepare_turn<S>(
    store: &S,
    llm: &dyn LlmAdapter,
    input: TurnInput,
    sink: std::sync::Arc<dyn crate::symbio_core::DeltaSink>,
) -> Result<PreparedFlow, crate::symbio_core::AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    let TurnInput {
        turn,
        user_seq,
        drive_seq,
        tier,
        window_turns,
        inject,
        resume,
        actor,
        prefix,
        final_reply,
        dialog,
        opening_text,
        opening_dialog,
        turn_root_id,
        nodes,
        abort,
        stages,
    } = input;
    // 本轮的 viewer：历史窗口只交**本主体看得见**的事件（plan/11 批1 ③）。
    let viewer = Some(actor.principal.as_str());
    // 本轮**新落**的产物格数（[ADR-067] 的「产物收束」判据）：`> 0` ⇒ 本轮已
    // 收束，发言闸再拒也不算未收束（`TurnOutcome::closure_denied` 与 C4 都认它）。
    let mut artifacts_landed = 0u64;
    // 1. 用户格**不在这里落**（T-225）：它已由编排层在判决之前落格，本函数收到的是
    // 它的 seq（`user_seq`）——运行器只拿它当溯源锚点，不再拥有「用户说了什么」。
    //
    // 1′. 一次性阶段的「已做」与判决事实（ADR-107 判据 2）：`classify` / `recall` /
    // `compress` 在本轮执行过 ⇒ 各落一格 `turn.stage`。放在用户格之后：断言类
    // 事件必须带溯源锚点（I2），而那只有落完用户格才有。
    //
    // 续写幕复用同一 turn 号、`request_round` 从 0 重来，召回 / 压缩会重跑——重跑
    // 时撞幂等键 ⇒ `land` 按「已做过」吸收（不重复落、也不失败）。
    super::super::stage_events::land(store, turn, user_seq, &actor.principal, &stages)?;
    // 1″. 本轮派发档（T-225）：用户格先于判决落，档位要到执行装配期才定 ⇒ 另立一格。
    // 幂等键带**幕号**：同一轮可以被恢复幕再执行一次，两幕的档位各记各的。
    store.append(
        Event::pending(
            format!("d-{turn}-{drive_seq}"),
            EVENT_TURN_DISPATCH,
            Entity::Turn,
            Verb::Asserted,
            turn,
            &actor.principal,
        )
        .with_produced_by(user_seq)
        .with_payload(serde_json::json!({
            "tier": tier.name(),
            "budget_ms": actor.budget_ms,
            "principal": actor.principal,
        })),
    )?;
    // 1a. 对话面开场白（T-056）：`Escalate` 首响——本轮随后**照进工具循环**，
    // 所以它落在 `turn × asserted`（不是收束格 N3「每轮至多一条」）。
    //
    // 放在用户格之后：断言类事件必须带溯源锚点（I2），而那只有落完用户格才有。
    // 空正文不落格（与汇报 / 收束同一条纪律）。落格即上线：节点 id 用落格载荷里
    // 那个（`dialog.id`），轮末同 id 再送一次时消费端按 id 合并成同一格。
    if let (Some(opening), Some(dialog)) = (opening_text.as_deref(), opening_dialog.as_ref()) {
        if !opening.trim().is_empty() {
            store.append(
                Event::pending(
                    format!("op-{turn}"),
                    EVENT_ASSISTANT_OPENING,
                    Entity::Turn,
                    Verb::Asserted,
                    turn,
                    &actor.principal,
                )
                .with_produced_by(user_seq)
                .with_payload(serde_json::json!({
                    "text": opening,
                    "id": dialog.id,
                    "reason": dialog.reason,
                    "exclude_from_context": dialog.exclude_from_context,
                })),
            )?;
            let mut node = crate::symbio_core::node_build::dialog_node(
                opening,
                &dialog.reason,
                dialog.exclude_from_context,
            );
            node.id = dialog.id.clone();
            sink.on_tool_frame(&node);
        }
    }
    // 1b. 定稿答话（缺口 5）：用户格照常开，答话照常收束，**模型不参与**。
    //
    // 为什么放在这里而不是循环里：这一轮一次请求都不发，循环的第一件事
    // （渲染 prompt、调模型）对它全都是白工。放在用户格之后是因为收束格的
    // 溯源必须指向本轮用户格（I2）——而那只有落完用户格才有。
    if let Some(reply) = final_reply.as_deref() {
        let facts = node_facts(&nodes);
        let produced = produced_payload(reply, "", &turn_root_id, &facts, &[], &[]);
        if reply.trim().is_empty() {
            return settle::append_fallback(
                store,
                &actor,
                turn,
                user_seq,
                "compose returned empty",
                0,
                produced,
                artifacts_landed,
            )
            .await
            .map(PreparedFlow::Done);
        }
        if let Err(why) = closure_gate(&actor, ClosureKind::Speech) {
            // 拒绝 ⇒ 不落收束格，但落否定事实（`authz.denied`）。
            deny(
                store,
                &actor,
                turn,
                user_seq,
                ClosureKind::Speech,
                EVENT_ASSISTANT_FINAL,
                &why,
            );
            crate::plugin_warn!(
                "actors",
                "[v2] 定稿答话被授权拒绝（{why}），本轮不入格（turn={turn}）"
            );
            return Ok(PreparedFlow::Done(TurnOutcome {
                turn,
                text: reply.to_string(),
                cost_ms: 0,
                usage: None,
                fell_back: false,
                aborted: false,
                awaits_user: false,
                closure_denied: artifacts_landed == 0,
                finish: crate::symbio_core::ModelFinishReason::Stop,
            }));
        }
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
            .with_payload(with_dialog(
                with_reason(
                    with_status(produced, "completed"),
                    crate::symbio_core::ClosureReason::Completed,
                ),
                dialog.as_ref(),
            )),
        )?;
        return Ok(PreparedFlow::Done(TurnOutcome {
            turn,
            text: reply.to_string(),
            cost_ms: 0,
            usage: None,
            fell_back: false,
            aborted: false,
            awaits_user: false,
            closure_denied: false,
            finish: crate::symbio_core::ModelFinishReason::Stop,
        }));
    }
    let full_snapshot = store.range(Seq::new(0));
    // 对话窗口：只把最近 N 个 turn 的事件交给 prompt（含当前轮）——窗口是**视图**
    // 问题，事实照常全量入格（append-only 不受影响）。
    let snapshot: Cow<'_, [Event]> = match window_turns {
        None => filter_visible(&full_snapshot, viewer),
        Some(keep) => window_by_turn(&full_snapshot, turn, keep, viewer),
    };
    // 基线：**结构化**消息数组（ADR-048a）。prefix（请求视图三段）作为最前的
    // 一条独立 system 消息插进去，而不是拼进第一段正文——理由见 `TurnInput::prefix`。
    let mut base_messages = crate::symbio_core::render_messages(&snapshot);
    if let Some(p) = prefix.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        base_messages.insert(
            0,
            crate::symbio_core::PromptMessage {
                role: "system".into(),
                text: p.to_string(),
                tool_call_id: None,
                tool: None,
                tool_calls: None,
            },
        );
    }
    // **双向完整性复核**（ADR-048 防线）：缺口只出声不抛错——prompt 已经发出去了，
    // 此时抛错只会把「模型看得不全」变成「这一轮直接失败」；而看得不全恰恰是静默的。
    {
        let declared = prefix.as_deref().map(str::trim).filter(|p| !p.is_empty());
        let rep = crate::symbio_core::verify_prompt_fidelity(&snapshot, &base_messages, declared);
        if !rep.is_complete() {
            crate::plugin_warn!(
                "actors",
                "[prompt-fidelity] 送出的消息与事实网格不完整：{}",
                rep.summary()
            );
        }
    }

    let started = std::time::Instant::now();
    // 本轮内已发生的工具交换（调用 + 结果），供下一次请求追加。
    let mut exchange: Vec<crate::symbio_core::PromptMessage> = Vec::new();
    // 本回合内已走完的**工具轮数**——只用于给合成 id 编号。
    let tool_round: usize = 0;
    // 实测耗时：跨轮累加（一次用户轮可能有多次 LLM 请求，ADR-044 的实测口径）。
    let cost_ms = 0u64;
    // 产物格的事件 id 需要在本轮内唯一（同一工具可被调用多次）。续写轮从**该轮
    // 已有产物数**起编号——等待轮可能已落过产物格，从 0 起会撞幂等键。
    let mut artifact_no = full_snapshot
        .iter()
        .filter(|e| e.turn == turn && e.kind == crate::symbio_core::EVENT_ARTIFACT_ADDED)
        .count() as u64;
    let mut all_tool_calls: Vec<ToolCallFact> = Vec::new();
    let rounds: Vec<StreamNodeFacts> = Vec::new();

    // 续写轮：恢复产生的工具交换是**本轮已发生的事实**——先落格再拼进 prompt。
    // 落格之后它固然进了投影，但本轮的 `base_prompt` 已在上方渲染完毕，故仍就地追加。
    if let Some(r) = &resume {
        // 产物闸（`ClosureKind::Artifact`）：不持 `produce.artifact` ⇒ 这一格不落。
        // 结果**照进 prompt**（模型看得见）+ 照记 `all_tool_calls`（请求发生过）。
        if closure_granted(&actor, ClosureKind::Artifact) {
            store.append(
                Event::pending(
                    format!("a-{turn}-{artifact_no}"),
                    crate::symbio_core::EVENT_ARTIFACT_ADDED,
                    Entity::Artifact,
                    Verb::Asserted,
                    turn,
                    &actor.principal,
                )
                .with_produced_by(user_seq)
                .with_payload(serde_json::json!({
                    "tool": r.call.name.clone().unwrap_or_default(),
                    "text": r.text,
                    "tool_call_id": r.call_id.clone().or_else(|| r.call.id.clone()),
                    "wire_id": r.call.wire_id,
                    "args": crate::symbio_core::node_build::tool_call_args_text(&r.call),
                    "result_id": r.result_id,
                    "success": true,
                    "turn_id": r.turn_id.clone().unwrap_or_else(|| turn_root_id.to_string()),
                })),
            )?;
            artifact_no += 1;
            artifacts_landed += 1;
            all_tool_calls.push(ToolCallFact {
                id: r
                    .call_id
                    .clone()
                    .or_else(|| r.call.id.clone())
                    .unwrap_or_default(),
                name: r.call.name.clone().unwrap_or_default(),
                args: crate::symbio_core::node_build::tool_call_args_text(&r.call),
                wire_id: r.call.wire_id.clone().unwrap_or_default(),
                result_id: r.result_id.clone(),
            });
        } else {
            crate::plugin_warn!(
                "actors",
                "[v2] 产物被授权拒绝（{} 缺 produce.artifact），本条结果不入格（turn={turn}）",
                actor.principal
            );
        }
        exchange.extend(exchange_messages(&r.call, &r.text));
    }

    // 1c. **闸门与首请求之间的中止**（ADR-052 决策 1）。检查点在这里：用户格已落、
    // 请求未发，已中止则直接走中止收束格，与 adapter 中途返回 Aborted 落的是同一格。
    if let Some(sig) = abort.as_ref() {
        if sig.is_aborted() {
            return settle::append_aborted(store, &actor, turn, user_seq, llm, &turn_root_id)
                .await
                .map(PreparedFlow::Done);
        }
    }

    Ok(PreparedFlow::Run(Box::new(Prepared {
        turn,
        drive_seq,
        step: 0,
        actor,
        user_seq,
        turn_root_id,
        started,
        base_messages,
        exchange,
        tool_round,
        cost_ms,
        artifact_no,
        artifacts_landed,
        all_tool_calls,
        rounds,
        full_snapshot,
        inject,
        nodes,
        pending_model: None,
        pending_round: None,
    })))
}
