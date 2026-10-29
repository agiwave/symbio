//! 阶段① 轮首准备（· B3，DE-021 §5 动作 B3）。
//!
//! 定稿答话 / 空答兜底两条早退路 → 窗口与基线 prompt → 续写产物补写。产出的
//! [`Prepared`] 是后续阶段的**唯一状态来源**：请求阶段只读它、批处理阶段只改它，
//! 阶段之间不传第二份事实。首请求前的中止由环的第一道守卫判
//! （[`super::TurnSession::abort_before_first_request`]）——草稿先建好，环再看。
//!
//! 用户格**不在这里落**：它先于任何判决，由编排层在 `drive` 之前写入，
//! 本阶段收到的是它的 seq（`TurnInput::user_seq`）。
//!
//! 早退（定稿答话 / 空答兜底 / 首请求前中止）经 [`PreparedFlow::Done`] 交回结局，
//! 不把半备好的草稿交出去——幕首之后只有一种状态在流转（[`Prepared`]）。

use super::*;
use crate::control_decl::{CHAT_VOCAB, MESSAGE_VOCAB};

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
    /// 本轮中止信号（`D-052` 决策 1）。检查点是环的**第一道守卫**——
    /// [`super::TurnSession::abort_before_first_request`] 在首圈、任何行之前读它。
    pub(crate) abort: Option<symbio_core::ExecAbortSignal>,
    pub(crate) user_seq: u64,
    pub(crate) turn_root_id: String,
    pub(crate) started: std::time::Instant,
    /// 基线 prompt（**结构化**消息数组，D-048a）。建草稿时是**空的**，由
    /// [`Prepared::render_base`] 在接缝上填——压缩改的就是消息列表，而压缩是环里的一行
    /// （`D-222` 订正 3：草稿先于任何动作存在），建草稿那一刻它还不在事实源里。
    pub(crate) base_messages: Vec<symbio_core::PromptMessage>,
    pub(crate) exchange: Vec<symbio_core::PromptMessage>,
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
    /// 建草稿时那一次 prompt 的窗口（`TurnInput::window_turns` 的副本）。
    ///
    /// 它唯一的读者是 [`Prepared::set_prefix`] 里的 D-048 复核：前缀在草稿建好之后
    /// 才到，复核要按**同一个窗口**重裁快照才与原实现逐字同输入。不存裁好的切片
    /// 是为了不把窗口那份事件再存一遍（`full_snapshot` 已经是全量）。
    pub(crate) window_turns: Option<u64>,
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
    /// 基线 prompt 的**渲染时刻**：轮首那几行跑完、任何请求之前。
    ///
    /// ## 为什么不在建草稿时渲染
    ///
    /// 压缩（`policy.compress`）改的就是消息列表——它落 `session.consolidated` 之后，
    /// 转写投影才读得到那份摘要。而草稿在**幕首**就建（`D-222` 订正 3），压缩是环里的
    /// 一行、在它之后才跑。照幕首那一份渲染，模型读到的就是**没压过**的历史：事实照常
    /// 入格、界面照常变短，只有 prompt 里那段摘要不存在。
    ///
    /// ## 为什么重取切片
    ///
    /// [`Self::full_snapshot`] 在幕首取过一次（产物编号要用），那一份**不含**压缩格。
    /// 这里重取并覆盖它：渲染与 [`Self::set_prefix`] 的 D-048 复核必须站在**同一份**
    /// 切片上，否则复核会对着「prompt 里有摘要、切片里没有」报一条假的完整性告警。
    /// 两个读点各有各的口径，不是两份真相——后一次是前一次的超集。
    pub(crate) fn render_base(&mut self, store: &dyn Store<Event = Event>) {
        self.full_snapshot = store.range(Seq::new(0));
        let viewer = Some(self.actor.principal.as_str());
        let snapshot: Cow<'_, [Event]> = match self.window_turns {
            None => filter_visible(&self.full_snapshot, viewer),
            Some(keep) => window_by_turn(&self.full_snapshot, self.turn, keep, viewer),
        };
        self.base_messages = render_messages(&snapshot);
    }

    /// 本次 `drive` 的 `now`：幕锚那条**唤醒事实**的 `ts`（判据 12 / DE-015 §2.1）。
    ///
    /// 切片取准备段读到的那一份，而用户格（或恢复格）在进运行器之前就落了 ⇒
    /// 幕锚一定在里面。不在 ⇒ 0：宁可判「还没有任何时间条件成立」。
    pub(crate) fn now_ms(&self) -> i64 {
        super::super::loop_rows::wake_now_ms(&self.full_snapshot, self.drive_seq)
    }

    /// 把**请求级前缀**插进基线 prompt 的最前一条（[`super::TurnSession::set_prefix`] 的本体）。
    ///
    /// ## 为什么复核在这里而不在构造期
    ///
    /// D-048 的双向完整性复核（「送出的消息与事实网格不完整」只出声不抛错）判的是
    /// `declared` 与 `base_messages` 是否一致。前缀没插进去时 `declared` 必须是
    /// `None`，否则构造期就会报一条假的「声明了却没插」；而前缀在构造期确实还拿不到
    /// （它由 `prepare_request_sections` 的 ⑥ 在轮首那几行之后收集）。于是复核跟着前缀走。
    ///
    /// 快照按**建草稿时那一份**（[`Self::full_snapshot`]）与同一个窗口重裁：与原实现
    /// 里 `prepare_turn` 用的那一份逐字相同，复核的输入没有变，只是时机从「构造期」
    /// 挪到了「填前缀期」。
    ///
    /// ## 空串 / 纯空白 = 不加
    ///
    /// 与进 `TurnInput` 的时代同一条判据：`None`、空串、纯空白都不插——插一条空的
    /// system 消息会让模型读到一段空白，而它本该什么都不读。
    pub(crate) fn set_prefix(&mut self, prefix: Option<String>) {
        let Some(p) = prefix.as_deref().map(str::trim).filter(|p| !p.is_empty()) else {
            return;
        };
        self.base_messages.insert(
            0,
            symbio_core::PromptMessage {
                role: "system".into(),
                text: p.to_string(),
                tool_call_id: None,
                tool: None,
                tool_calls: None,
            },
        );
        let viewer = Some(self.actor.principal.as_str());
        let snapshot: Cow<'_, [Event]> = match self.window_turns {
            None => filter_visible(&self.full_snapshot, viewer),
            Some(keep) => window_by_turn(&self.full_snapshot, self.turn, keep, viewer),
        };
        let rep = symbio_core::verify_prompt_fidelity(
            &snapshot,
            &self.base_messages,
            Some(p),
            &CHAT_VOCAB,
            &MESSAGE_VOCAB,
        );
        if !rep.is_complete() {
            symbio_core::plugin_warn!(
                "actors",
                "[prompt-fidelity] 送出的消息与事实网格不完整：{}",
                rep.summary()
            );
        }
    }
}

pub(crate) async fn prepare_turn<S>(
    store: &S,
    input: TurnInput,
) -> Result<PreparedFlow, symbio_core::AppendError>
where
    S: Store<Event = Event> + ?Sized,
{
    let TurnInput {
        turn,
        user_seq,
        drive_seq,
        window_turns,
        inject,
        actor,
        final_reply,
        final_cost_ms,
        dialog,
        turn_root_id,
        nodes,
        abort,
    } = input;
    // 本轮**新落**的产物格数（[D-067] 的「产物收束」判据）：`> 0` ⇒ 本轮已
    // 收束，发言闸再拒也不算未收束（`TurnOutcome::closure_denied` 与 C4 都认它）。
    let artifacts_landed = 0u64;
    // 1. 用户格**不在这里落**：它已由编排层在判决之前落格，本函数收到的是
    // 它的 seq（`user_seq`）——运行器只拿它当溯源锚点，不再拥有「用户说了什么」。
    //
    // 1′. 一次性阶段（`turn.stage`）同样**不在这里落**：环上的每一行（判决 / 召回 /
    // 压缩）由它自己的执行体当场入日志，溯源锚就是那一刻已在的用户格 seq（I2）。
    //
    // 1″. 派发档与开场白**也不在这里落**（`D-223` 订正 1）：两者的内容要到轮首四行跑完
    // 才存在（档位折自 `policy.reflex` 的出口格，开场白是判决行 `Escalate` 首响），
    // 落点在接缝上（`chat_loop::land_dispatch` / `land_opening`）。草稿在幕首就建
    // （`D-222` 订正 3），那一刻它们一格都还没有。
    // 1b. 定稿答话（缺口 5）：用户格照常开，答话照常收束，**模型不参与**。
    //
    // 为什么放在这里而不是循环里：这一轮一次请求都不发，循环的第一件事
    // （渲染 prompt、调模型）对它全都是白工。放在用户格之后是因为收束格的
    // 溯源必须指向本轮用户格（I2）——而那只有落完用户格才有。
    //
    // **这是方案允许的形状，不是遗留缺口**（[D-125](../../../../docs/003-design/015-义务环.md#d-125-定稿答话轮不进轮内驱动循环环覆盖的是欠着的往返收束格由轮首准备就地落 "DE-015-义务环")）：
    // 环调度的是「还没还的债」，而这一轮欠的只有判决那一行——它跑完就把结论落成
    // `answered`，本轮就地收束，**轮内驱动循环压根没起来**。模板产线出话时零往返，措辞产线
    // 出话时那一次由 `compose` 在轮首之前发起并已入账（`via=compose/wording`），
    // 它不是本轮的债。所以收束格由这里就地落——但**构造点仍只有一个**：这条路径经
    // `settle::land_final` 落格，「谁有权写」由 `loop_rows::ACTOR_CLOSE.writes` 回答，
    // 「从哪里写出去」仍是那一处（`SP-033` `TC-4`）。别改成「寄存结局返回 `Run`」：措辞那一类没有可供 `PendingClose`
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
                EVENT_CHAT_ASSISTANT_FINAL,
                &why,
            );
            symbio_core::plugin_warn!(
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
                finish: symbio_core::ModelFinishReason::Stop,
            }));
        }
        settle::land_final(
            store,
            &actor,
            turn,
            user_seq,
            final_cost_ms,
            with_dialog(
                with_reason(
                    with_status(produced, "completed"),
                    symbio_core::ClosureReason::Completed,
                ),
                dialog.as_ref(),
            ),
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
            finish: symbio_core::ModelFinishReason::Stop,
        }));
    }
    let full_snapshot = store.range(Seq::new(0));
    // 2. 基线 prompt **不在这里渲染**：它改的是「模型读到什么历史」，而压缩（环里的一行）
    // 改的正是那份历史——建草稿那一刻压缩还没落。入口是 [`Prepared::render_base`]，
    // 调用点在接缝上（轮首那几行跑完、任何请求之前）。本函数只把**切片**留下来：产物
    // 编号要用它，且它同时是「幕首这一刻的事实面」。
    //
    // 请求级前缀也不在这里插（`D-222` 订正 3）：它由 `prepare_request_sections` 的 ⑥ 在轮首
    // 那几行**之后**收集。入口是 [`Prepared::set_prefix`]，D-048 的双向完整性复核跟着它
    // 走——`declared` 只有在前缀真被插进去的那一刻才有值。

    let started = std::time::Instant::now();
    // 本轮内已发生的工具交换（调用 + 结果），供下一次请求追加。
    let exchange: Vec<symbio_core::PromptMessage> = Vec::new();
    // 本回合内已走完的**工具轮数**——只用于给合成 id 编号。
    let tool_round: usize = 0;
    // 实测耗时：跨轮累加（一次用户轮可能有多次 LLM 请求，D-044 的实测口径）。
    let cost_ms = 0u64;
    // 产物格的事件 id 需要在本轮内唯一（同一工具可被调用多次）。续写轮从**该轮
    // 已有产物数**起编号——等待轮可能已落过产物格，从 0 起会撞幂等键。
    let artifact_no = full_snapshot
        .iter()
        .filter(|e| e.turn == turn && e.kind == symbio_plugin_api::events::EVENT_ARTIFACT_ADDED)
        .count() as u64;
    let responses: Vec<String> = Vec::new();
    let last_output_facts = StreamNodeFacts::default();
    let last_answer_id = String::new();

    // 唤醒幕（审批 / 问答恢复）**不在这里续写交换**：用户动作落成唤醒请求
    // （`chat_loop::wake`），授权与派发由行表承担，工具交换由批尾收尾自己产生
    // （`stream::land_batch`）——本段不再私自拼一段 prompt。

    // 1c. **闸门与首请求之间的中止**（D-052 决策 1）不在这里判：它成了环的第一道守卫
    // （`turn_exec::drive` 首圈）。草稿先建好、把信号存进去，**被中止的幕因此也不再跑
    // 判决 / 压缩 / 召回**（`D-222` 订正 3 认下的行为变化）——环首圈先看中止，再看
    // 任何一行欠不欠。

    Ok(PreparedFlow::Run(Box::new(Prepared {
        turn,
        drive_seq,
        step: 0,
        actor,
        abort,
        user_seq,
        turn_root_id,
        started,
        // 空基线：接缝上的 [`Prepared::render_base`] 才渲染（压缩之后那一份）。
        base_messages: Vec::new(),
        exchange,
        tool_round,
        cost_ms,
        artifact_no,
        artifacts_landed,
        responses,
        last_output_facts,
        last_answer_id,
        full_snapshot,
        window_turns,
        inject,
        nodes,
        pending_model: None,
        pending_round: None,
        pending_calls: None,
        // 快路：正文由 `policy.reflex` 的执行体经 [`super::TurnSession::set_reflex`]
        // 寄存（`D-222` 订正 3：那一行是环的债，它的产物在草稿建好之后才到）。
        // 零模型往返时驱动环的第一个债就是 `close-turn`（55 < 60）——本段照常备好
        // 派发格与 prompt，只是不发请求。
        closing: None,
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
pub(crate) fn render_messages(events: &[Event]) -> Vec<symbio_core::PromptMessage> {
    // `read_all` 说的是**口径**（as-of 一切已发生、不设扫描窗口），不是作用域。
    symbio_core::transcript(CHAT_VOCAB, MESSAGE_VOCAB)
        .read_all(events)
        .value
        .to_messages()
}
