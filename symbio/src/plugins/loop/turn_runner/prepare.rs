//! 阶段① 轮首准备（· B3，DE-021 §5 动作 B3）。
//!
//! 阶段事实与派发档入格 → 开场白 / 定稿答话两条早退路 → 窗口与基线 prompt →
//! 续写产物补写 → 首请求前的中止检查。产出的 [`Prepared`] 是后续阶段的**唯一
//! 状态来源**：请求阶段只读它、批处理阶段只改它，阶段之间不传第二份事实。
//!
//! 用户格**不在这里落**：它先于任何判决，由编排层在 `drive` 之前写入，
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

/// 待收束的**结局**（c）：模型往返的三种收法各带自己的载荷。
///
/// 收束格只由 `close` 执行体写（装载期的依赖图要求两个执行体写的格不相交），
/// 所以「这一轮怎么收」不能由模型往返那一步就地落格，只能**寄存**在这里，
/// 由收束动作来落。
///
/// 三种收法各带自己的载荷，因为它们落的格子与载荷形状都不同：
/// [`Self::Answer`] 落 `chat.assistant.final`（答完了），
/// [`Self::Failed`] 落 `chat.assistant.fallback`（模型没答成），
/// [`Self::Aborted`] 也落 `chat.assistant.final`（中止是**状态**，不是另一种事实）。
pub(crate) enum PendingClose {
    /// 答完了：应答与本轮的流式节点事实（收束动作据此落 `chat.assistant.final`）。
    Answer { rt: LlmTurn, facts: StreamNodeFacts },
    /// 模型没答成：失败原因（人话，只给人读）与失败前已产出的半截产出。
    Failed {
        detail: String,
        cost_ms: u64,
        produced: serde_json::Value,
    },
    /// 中止：中止前的半截产出、实测耗时。
    Aborted {
        cost_ms: u64,
        produced: serde_json::Value,
        text: String,
    },
    /// 快路作答：技能正文（D-163）。这一轮**没有模型往返**，收束动作把正文
    /// 直接落成 `chat.assistant.final`（`model = "reflex"`），并上桥。
    Reflex { text: String },
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
    /// 本轮已经落格的**模型输出事实**的逻辑 id（`response` / `failed`，不含内部 `via`）。
    ///
    /// 收束 / 进展 / 兜底格只存**轮级结局**：正文 / 思考归那一次往返自己的事实，
    /// 这里记的是「本轮有哪些往返」——重建与转写按它把正文折回来。
    pub(crate) responses: Vec<String>,
    /// 最近一次模型往返的**产出事实**（正文 / 思考与节点 id），由应答 / 失败载荷
    /// 那一侧**取一次**之后寄存。
    ///
    /// 取数口是桥的只读快照，但它的读数是「这一次往返的固定形状」——每轮多读一次
    /// 只会读到同一份，却让取数口的消费方（测试的计数式假桥）多算一次。
    pub(crate) last_output_facts: StreamNodeFacts,
    /// 最近一次模型往返「说了什么」的节点身份（`answer_id`）。
    ///
    /// 收束 / 进展格要写它，而它必须与那一次 `response` 事实里算出的**同一个**
    /// ——由 `output_fields` 一次算定后带过来，读侧不再自己推。
    pub(crate) last_answer_id: String,
    pub(crate) full_snapshot: Vec<Event>,
    pub(crate) inject: Option<RoundInjector>,
    pub(crate) nodes: Option<NodeFactsFn>,
    /// 模型应答已落、工具批次尚未派发时的寄存位（拆步）。
    ///
    /// 放在 `Prepared` 而不是新开一份状态：它和消息 / 节点 / 成本同属**本轮草稿**，
    /// 引擎不读它；拆步只是把「一次 `step` 做两件事」拆成两个可分别调用的动作。
    pub(crate) pending_model: Option<LlmTurn>,
    /// 已**宣布**、还没派出的那一次模型往返（D-166）。
    ///
    /// 与 [`Prepared::pending_model`] 不是一回事：那一格是「应答已落、工具批还没派」，
    /// 这一格是「请求已落、模型还没打」。两格之间的那一步（`dispatch-model` 行）由
    /// 另一个执行体跑，所以交接件只能放在草稿里——它就是那两个动作之间唯一的凭据。
    pub(crate) pending_round: Option<super::super::port::Call>,
    /// 创建动作开出的端口账（b）：创建 → 授权 → 派发三个动作之间**唯一**
    /// 的交接件。放在草稿里而不是各自重开：`ToolPortCall` 的 `granted` / `sent` /
    /// `deny_why` 是三个动作依次写入的同一份判定结果，分两次构造就会各说一套。
    pub(crate) pending_calls: Option<Vec<super::super::tool_dispatch::ToolPortCall>>,
    /// 应答已落、**收束还没发生**时的寄存位。
    ///
    /// 与上面两格分开放是刻意的：这一格的形状是「本轮没有工具调用」，而
    /// [`Prepared::pending_model`] 的形状是「本轮有工具调用、还没派发」——
    /// 两批债各由自己的行（`close-turn` / `open-tools`）选中，揉在一起会让收束幕
    /// 去派发一批不存在的工具。收束归 `close-turn` 行的执行体，它从这里取结局，
    /// 落 `chat.assistant.final` / `fallback`。
    pub(crate) closing: Option<PendingClose>,
    /// 本轮**收束动作的产出**（D-123 决策 5）：落收束 / 停机格的那一步把它寄存进草稿，
    /// 派发器在账本上看到停机格后从这里取走交回调用方。
    ///
    /// 为什么放草稿而不是让动作交回来：执行体不判断「本轮结束了没有」——那一半判据在
    /// 日志上（账本里出现停机格），交回 `Option<TurnOutcome>` 等于让它自己宣布收束。
    pub(crate) settled: Option<TurnOutcome>,
}

impl Prepared {
    /// 本次 `drive` 的 `now`：幕锚那条**唤醒事实**的 `ts`（判据 12 / DE-015 §2.1）。
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
        actor,
        prefix,
        final_reply,
        final_cost_ms,
        reflex,
        dialog,
        opening_text,
        opening_dialog,
        turn_root_id,
        nodes,
        abort,
    } = input;
    // 本轮的 viewer：历史窗口只交**本主体看得见**的事件（plan/11 批1 ③）。
    let viewer = Some(actor.principal.as_str());
    // 本轮**新落**的产物格数（[D-067] 的「产物收束」判据）：`> 0` ⇒ 本轮已
    // 收束，发言闸再拒也不算未收束（`TurnOutcome::closure_denied` 与 C4 都认它）。
    let artifacts_landed = 0u64;
    // 1. 用户格**不在这里落**：它已由编排层在判决之前落格，本函数收到的是
    // 它的 seq（`user_seq`）——运行器只拿它当溯源锚点，不再拥有「用户说了什么」。
    //
    // 1′. 一次性阶段（`turn.stage`）同样**不在这里落**：环上的每一行（判决 / 召回 /
    // 压缩）由它自己的执行体当场入日志，溯源锚就是那一刻已在的用户格 seq（I2）。
    // 1″. 本轮派发档：用户格先于判决落，档位要到执行装配期才定 ⇒ 另立一格。
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
    // 1a. 对话面开场白：`Escalate` 首响——本轮随后**照进工具循环**，
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
                &crate::symbio_core::principal::Principal::from_legacy(&actor.principal),
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
    //
    // **这是方案允许的形状，不是遗留缺口**（[D-125](../../../../../docs/003-design/015-义务环.md#d-125-定稿答话轮不进轮内驱动循环环覆盖的是欠着的往返收束格由轮首准备就地落 "DE-015-义务环")）：
    // 环调度的是「还没还的债」，而这一轮欠的只有判决那一行——它在轮首 host 跑完就把结论落成
    // `answered`，本轮就地收束，**轮内驱动循环压根没起来**。模板产线出话时零往返，措辞产线
    // 出话时那一次由 `compose` 在轮首之前发起并已入账（`via=compose/wording`），
    // 它不是本轮的债。所以收束格由这里就地落，`close` 执行体的「唯一写方」限定在
    // **驱动循环内**。别改成「寄存结局返回 `Run`」：措辞那一类没有可供 `PendingClose`
    // 使用的本轮往返（`rt`），硬接只能靠外形猜前件，零往返的同类轮会因此多打一次模型。
    if let Some(reply) = final_reply.as_deref() {
        let facts = node_facts(&nodes);
        // 零往返轮：正文与思考都只能随收束格入格（没有可挂的 `response` 事实）。
        let produced = with_reasoning(
            produced_payload(
                reply,
                "",
                &turn_root_id,
                facts.text_id.as_deref(),
                &answer_id_of(
                    reply,
                    facts.reasoning.as_deref().unwrap_or_default(),
                    &facts,
                    &turn_root_id,
                ),
                &[],
            ),
            &facts,
        );
        if reply.trim().is_empty() {
            return settle::append_fallback(
                store,
                &actor,
                turn,
                user_seq,
                "compose returned empty",
                final_cost_ms,
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
                cost_ms: final_cost_ms,
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
            .with_cost_ms(final_cost_ms)
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
            cost_ms: final_cost_ms,
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
    // 基线：**结构化**消息数组（D-048a）。prefix（请求视图三段）作为最前的
    // 一条独立 system 消息插进去，而不是拼进第一段正文——理由见 `TurnInput::prefix`。
    let mut base_messages = render_messages(&snapshot);
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
    // **双向完整性复核**（D-048 防线）：缺口只出声不抛错——prompt 已经发出去了，
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
    let exchange: Vec<crate::symbio_core::PromptMessage> = Vec::new();
    // 本回合内已走完的**工具轮数**——只用于给合成 id 编号。
    let tool_round: usize = 0;
    // 实测耗时：跨轮累加（一次用户轮可能有多次 LLM 请求，D-044 的实测口径）。
    let cost_ms = 0u64;
    // 产物格的事件 id 需要在本轮内唯一（同一工具可被调用多次）。续写轮从**该轮
    // 已有产物数**起编号——等待轮可能已落过产物格，从 0 起会撞幂等键。
    let artifact_no = full_snapshot
        .iter()
        .filter(|e| e.turn == turn && e.kind == crate::symbio_core::EVENT_ARTIFACT_ADDED)
        .count() as u64;
    let responses: Vec<String> = Vec::new();
    let last_output_facts = StreamNodeFacts::default();
    let last_answer_id = String::new();

    // 唤醒幕（审批 / 问答恢复）**不在这里续写交换**：用户动作落成唤醒请求
    // （`chat_loop::wake`），授权与派发由行表承担，工具交换由派发动作自己产生
    // （`stream::dispatch_round`）——本段不再私自拼一段 prompt。

    // 1c. **闸门与首请求之间的中止**（D-052 决策 1）。检查点在这里：用户格已落、
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
        responses,
        last_output_facts,
        last_answer_id,
        full_snapshot,
        inject,
        nodes,
        pending_model: None,
        pending_round: None,
        pending_calls: None,
        // 快路：正文寄到收束动作（D-163）。零模型往返，驱动环的第一个债就是
        // `close-turn`（55 < 60）——本段照常备好派发格与 prompt，只是不发请求。
        closing: reflex.map(|r| PendingClose::Reflex { text: r.text }),
        settled: None,
    })))
}

/// prompt 的**结构化渲染入口**（D-048a）：事件切片 → 消息数组。
///
/// 渲染是**纯投影**（不碰任何主体状态）：历史来自同一份事实源，不另存副本
/// （D-044 同族纪律）。进来的切片已由 `facts.rs` 过读侧闸并按轮开窗，这一层
/// 只回答「怎么把事件变成消息」。
///
/// **拍成一段文本**会把 `role: tool` 降级成散文里的纯文本，模型就看不到
/// 「这是工具结果」、也拿不到 `tool_call_id`——那是已判定的退步，不是排版偏好。
/// 本轮刚发生的工具交换快照取在它落格**之前**，由调用方就地追加到基线上。
pub(crate) fn render_messages(events: &[Event]) -> Vec<crate::symbio_core::PromptMessage> {
    // `read_all` 说的是**口径**（as-of 一切已发生、不设扫描窗口），不是作用域。
    crate::symbio_core::transcript()
        .read_all(events)
        .value
        .to_messages()
}
