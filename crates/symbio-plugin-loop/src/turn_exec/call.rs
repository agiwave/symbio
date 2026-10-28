//! 阶段② 开轮与驱动——幕首建草稿，随后把**一个环**跑完（· B2，DE-021 §5 动作 B2）。
//!
//! 两个入口，对应草稿的两个时刻（`D-222` 订正 3）：
//!
//! - [`open_turn`]：**幕首**——把 [`Assembled`] 摊成 [`TurnInput`] 交给
//!   `TurnRunner::begin`，产 [`Opened`]（已收束 / 有环要跑）。它一格「要等轮首那几行」
//!   的事实都不落。
//! - [`drive_opened`]：**轮首之后**——把轮首那一片状态（[`HeadEnv`]）与草稿交给
//!   [`super::drive`]，**唯一的环**从接缝行一直跑到收束（或判成答话）。
//!
//! 轮首四行、接缝、内环是同一张绑定表上的行：先后由 `prio` 表达，不由本模块的函数
//! 行序表达。本模块只负责把「环要借的那一片」交出去，并按 [`DriveEnd`] 分出两条
//! 出口（判成答话 / 跑完收束）。

use super::*;

/// 阶段② ⇒ 阶段③④ 的交付束：调用结果与桥、发射任务、落库还要用的一切。
///
/// `bridge` 与 `emitter` 按值走：阶段③ 的 `drop(bridge)` 是发射端退出的**唯一**
/// 信号，必须正好剩最后一个强引用——所以阶段② 内所有 `bridge.clone()` 都随
/// 注入闭包与分发方在本段内死去，最后一个原样搬进这里、由阶段③ 消费。
pub(crate) struct Running<'a> {
    pub(crate) session: &'a LoopSession,
    pub(crate) store: Arc<dyn Store<Event = Event>>,
    pub(crate) bridge: Arc<UiBridge>,
    pub(crate) emitter: tokio::task::JoinHandle<()>,
    pub(crate) outcome: super::super::turn_runner::TurnOutcome,
    pub(crate) produced: Vec<ChatNode>,
    pub(crate) derived: super::super::tool_dispatch::DerivedFacts,
    pub(crate) collected: Arc<std::sync::Mutex<Vec<ChatNode>>>,
    pub(crate) turn_no: u64,
    pub(crate) user_seq: u64,
    pub(crate) principal: String,
    pub(crate) to: String,
    pub(crate) user_text: &'a str,
    pub(crate) root_id: &'a str,
    pub(crate) presentation_tree: symbio_core::node_build::PresentationTree,
}

/// 幕首开轮的结果（[`open_turn`] 的出口）。
pub(crate) enum Opened {
    /// 本幕在开轮就收束了（空答兜底：定稿答话正文为空）：没有环可跑。
    ///
    /// 首请求前中止**不在这里**：它成了环的第一道守卫（`abort_before_first_request`），
    /// 因为草稿要先建好、被中止的幕才轮得到「轮首行一行都不跑」。
    Done(super::super::turn_runner::TurnOutcome),
    /// 有环要跑：草稿与驱动它要的端口。
    Run(OpenRun),
}

/// [`Opened::Run`] 的交付束。
///
/// `elimination_revoked` 在开轮前取一次（与接线前同一时刻）：淘汰闸的部署级判据由
/// session 折叠成**一个值**（`D-086 §4`：全量事件过墙是 P6「值注入」的反面）。
pub(crate) struct OpenRun {
    pub(crate) session: super::TurnSession,
    pub(crate) llm: std::sync::Arc<dyn LlmAdapter>,
    pub(crate) tok: FullModel,
    pub(crate) elimination_revoked: bool,
}

/// 轮首那一片**可控状态**：环要借、而本模块只拿到一份借用权的那些。
///
/// `orchestrator` / `context` / `turn` 都不归本模块所有（它们是编排层与调用方的
/// 本轮状态），`fresh_root` 是幕首定下的一个事实。环把它们逐圈再借给动作
/// （[`super::HeadParts`]）——`&mut` 与 `&` 分开成形，因为 `context` / `turn`
/// 每圈要重新借一次。
///
/// 接缝与轮首四行读的全是这一片；轮内那几行不看它。
pub(crate) struct HeadEnv<'e> {
    pub(crate) orchestrator: &'e super::super::chat_loop::state::ChatOrchestrator,
    pub(crate) context: &'e mut super::super::chat_loop::state::SessionContext,
    pub(crate) turn: &'e mut super::super::chat_loop::state::TurnState,
    pub(crate) fresh_root: bool,
}

/// [`drive_opened`] 的两条出口：环判成「就此答话」，或跑完收束。
pub(crate) enum OpenedFlow<'a> {
    /// 判决判成「就此答话」：本幕**没有发请求**，环在选债之前就收工了。
    /// 草稿、桥、分发方都已收掉（出臂即释放）；
    /// 收束格由调用方落（`chat_loop::land_answered_turn`）——它要的是编排层的
    /// 状态，不在本模块手里。
    Answered,
    /// 环跑到收束格出现：交回阶段③④ 要的交付束。
    ///
    /// 装箱只为让两条出口的尺寸可比：`Answered` 无载荷，不装箱时整个枚举按 312 字节
    /// 的交付束分配，而这一支每幕至多构造一次。
    Running(Box<Running<'a>>),
}

/// 幕首开轮（[`TurnRunner::begin`]）：只建草稿，不落任何「要等轮首那几行」的格。
///
/// 首请求前的中止由派发器的第一圈守卫接住（[`super::drive`] 的 `abort_before_first_request`，
/// 落中止收束格、交回 [`super::DriveEnd::Finished`]）——它排在任何一行轮首行**之前**，
/// 所以被中止的幕不再跑判决 / 压缩 / 召回（`D-222` 订正 3 认下的行为变化）。
///
pub(crate) async fn open_turn(a: &Assembled<'_>) -> Result<Opened, TurnExecExit> {
    // 深度档：完整推理（含工具内环）。快路的**判定**已经不在这一层：它由环上的
    // `policy.reflex` 行选中，出口格（`stage:"reflex"`）与正文随本轮状态到接缝；
    // 本侧只把正文交给收束动作，不自己落 `final`（D-163）。
    let llm: std::sync::Arc<dyn LlmAdapter> = std::sync::Arc::new(ProviderLlmAdapter::for_turn(
        a.provider.clone(),
        a.system_prompt,
        a.abort.clone(),
    ));
    let tok = TokenIssuer::issue_deep();
    let input = TurnInput {
        turn: a.turn_no,
        user_seq: a.user_seq,
        drive_seq: a.drive_seq,
        window_turns: Some(a.window_turns),
        actor: a.actor.clone(),
        // 轮内注入（缺口 3）：用户中途说的话折进**本轮**。
        //
        // core 只知道「加消息」——抽干、合并、**落格**全在这里（插件侧）。
        // core 不该知道「收件箱补充」这个概念（队列、`merge_supplements`、
        // `InboxItem` 都在 `plugins/session`），而落格也不需要 core 参与：
        // 本函数自己就开着 `store`。
        //
        // 汇报（缺口 4）**不在这里**：批次边界那件事成了 `policy.report` 行，
        // 由同一个环按 `prio` 选中、由它自己的执行体落格上线（D-163）。闭包只留
        // 「抽干」这一件事——它仍然是 core 唯一的轮边界挂点。
        inject: injector(a.supplements.clone()).map(|drain| {
            let store = a.store.clone();
            let actor = a.actor.clone();
            let turn_no = a.turn_no;
            let supplemental_no = a.supplemental_no.clone();
            // 折进的消息往收集器 push（末尾随 `ExecTurnResult` 回灌进落库镜像）。
            let sink_msgs = a.collected.clone();
            std::sync::Arc::new(
                move |anchor: Option<u64>| -> std::pin::Pin<
                    Box<dyn std::future::Future<Output = Vec<symbio_core::PromptMessage>> + Send>,
                > {
                    // `async move` 块按值捕获，所以这里先各 clone 一份——
                    // 否则第一次调用就把捕获物搬走，闭包退化成 `FnOnce`。
                    let drain = drain.clone();
                    let store = store.clone();
                    let actor = actor.clone();
                    let sink_msgs = sink_msgs.clone();
                    let supplemental_no = supplemental_no.clone();
                    Box::pin(async move {
                        // 抽干口本身就是异步的（会话动作面），直接 await——
                        // 搬家前那套 `blocking_read` + `block_in_place` 的桥接
                        // 随队列一起留在了 session，这里不再需要判 runtime flavor。
                        let batch = drain().await;
                        supplement_into_request(
                            batch,
                            anchor,
                            store.as_ref(),
                            &actor,
                            turn_no,
                            &sink_msgs,
                            &supplemental_no,
                        )
                    })
                },
            ) as super::super::turn_runner::RoundInjector
        }),
        // 本路**恒**走生成：定稿答话轮（缺口 5）不经这里——它一个模型请求都不发，
        // 由 `execute_final_reply_turn` 单独入口落格（见该函数）。
        final_reply: None,
        final_cost_ms: 0,
        // 本路恒走生成 ⇒ 没有对话面节点（定稿答话轮不走这里）。
        dialog: None,
        // 落格身份：本轮 Turn 根 id（载荷 `turn_id`）+ 流式节点取数口。用户格
        // 已由编排层先落，运行器只收它的 seq。取数口是**快照闭包**：
        // 运行器每轮收束 / 进展 / 兜底时问一次「此刻桥上攒着哪两个节点」，
        // 中途正文与思考因此逐轮入格（半截中止时它们是唯一幸存者）。
        turn_root_id: a.root_id.to_string(),
        nodes: Some(std::sync::Arc::new({
            let bridge = a.bridge.clone();
            move || {
                let (text_id, text) = bridge
                    .current_node()
                    .map(|(id, acc)| (Some(id), Some(acc)))
                    .unwrap_or((None, None));
                let (reasoning_id, reasoning) = bridge
                    .current_reasoning()
                    .map(|(id, acc)| (Some(id), Some(acc)))
                    .unwrap_or((None, None));
                super::super::turn_runner::StreamNodeFacts {
                    text_id,
                    text,
                    reasoning_id,
                    reasoning,
                }
            }
        }) as super::super::turn_runner::NodeFactsFn),
        // 中途中止信号（D-052 决策 1）：闸门与首请求之间的空隙由运行器
        // 接管——落完用户格、发第一个请求前检查，已中止则走中止收束格。
        abort: Some(a.abort.clone()),
    };
    // 淘汰闸的部署级判据：取失败按「未收回」继续并出声——与「读不出的 WAL 跳过」的
    // 既有口径一致。取值点与接线前一致：开轮之前（轮首那几行不参加这个判断）。
    let elimination_revoked = a
        .session
        .run_op::<bool>(run_ops::ELIMINATION_REVOKED, ())
        .await
        .unwrap_or_else(|e| {
            symbio_core::plugin_warn!(
                "session",
                "[Turn] 淘汰闸判据读取失败（按未收回处理，本轮继续）: {e}"
            );
            false
        });
    match super::TurnRunner::begin(a.store.as_ref(), input).await {
        // 落格失败与动作失败走**同一个出口**（`ActorError` 的语义，这里折成
        // `TurnExecExit::Failed`）：调用方只认这一条通道，不为「开轮」单开一条。
        Err(e) => Err(TurnExecExit::Failed(PluginError::InternalError(format!(
            "v2 运行器落格失败：{e:?}"
        )))),
        // 本幕在开轮就收束了（首请求前中止 / 空答兜底）：没有环可跑，产出就是它。
        Ok(super::TurnSessionBegin::Done(outcome)) => Ok(Opened::Done(outcome)),
        Ok(super::TurnSessionBegin::Run(session)) => Ok(Opened::Run(OpenRun {
            session,
            llm,
            tok,
            elimination_revoked,
        })),
    }
}

/// 把唯一的环跑完：工具分发方 → 轮首四行 / 接缝 / 内环（同一个 `drive`）。
///
/// ## 次序不再由本函数表达
///
/// 这里只做「把环要借的那一片交出去」：接缝（压缩 / 定档 / 首响）与轮首四行都是
/// 绑定表上的行，先后由 `prio` 说；`run_seam` 那一行自己完成「压缩 → 请求视图 →
/// 派发 / 首响 → 渲染基线 → 填前缀与快路」这条硬次序。本函数的行序因此只剩一件事：
/// 分发方先建好（工具批的动作要它）。
///
/// ## 两个生命周期
///
/// `Assembled` 活到本函数结束（它持有会话句柄与事实源），`head` 借的是调用方的
/// 本轮状态（`'e` 与返回值的 `'a` 无关）——`Running` 按**短的那个**成形（`'s: 'a`）：
/// 它只活到本轮收束，收束一完调用方就要写回 `turn`。
///
/// ## 判成答话
///
/// [`DriveEnd::Answered`] 时本幕没有发请求：草稿（含注入闭包）、分发方、汇报落地面
/// 与桥都要先丢掉，发射端才会在队列关闭后退出。收束格不在这里落
/// （[`OpenedFlow::Answered`] 的文档说了为什么）。
pub(crate) async fn drive_opened<'s, 'a, 'e>(
    a: Assembled<'s>,
    open: OpenRun,
    head: HeadEnv<'e>,
) -> Result<OpenedFlow<'a>, TurnExecExit>
where
    's: 'a,
{
    let Assembled {
        session: loop_session,
        parent,
        ctx,
        abort,
        root_id,
        sink,
        user_text,
        tools,
        report,
        supplemental_no,
        presentation_tree,
        store,
        turn_no,
        user_seq,
        drive_seq,
        collected,
        bridge,
        emitter,
        actor,
        principal,
        to,
        ..
    } = a;
    let OpenRun {
        mut session,
        llm,
        tok,
        elimination_revoked,
    } = open;
    // 工具分发：契约与实现都在插件侧（D-118 决策 5）——它持插件宿主、
    // 请求上下文、转写出口与工具存档目录（core 认识这些即违 E-009）。
    let dispatcher = std::sync::Arc::new(super::super::tool_dispatch::SessionDispatchPort::new(
        parent,
        ctx.clone(),
        sink.clone(),
        loop_session.tool_archive_dir().to_path_buf(),
        store.clone(),
        abort.clone(),
        root_id.to_string(),
        turn_no,
        user_seq,
        drive_seq,
        bridge.clone(),
        presentation_tree,
        elimination_revoked,
    ));
    // 汇报的落格面（事实源 / 实时桥 / 落库镜像 / 序号游标）：`policy.report` 的执行体
    // 读它。四样都是本轮已建好的句柄，环里没有第二处能造出同一组。
    let report_landing = super::super::report_exec::ReportLanding {
        collected: collected.clone(),
        live: bridge.clone(),
        no: supplemental_no,
    };
    let mut head_parts = Some(super::HeadParts {
        orchestrator: head.orchestrator,
        request_ctx: &ctx,
        loop_session,
        context: head.context,
        turn: head.turn,
        bridge: &bridge,
        event_sink: sink,
        root_id,
        fresh_root: head.fresh_root,
        actor: &actor,
        report_port: report.as_ref(),
        report_landing: &report_landing,
    });
    let end = super::drive(
        &mut session,
        &store,
        llm,
        tok,
        bridge.clone(),
        std::sync::Arc::from(tools),
        Some(dispatcher.clone()),
        &mut head_parts,
        super::super::loop_rows::ACTORS,
    )
    .await
    .map_err(|e| {
        TurnExecExit::Failed(PluginError::InternalError(format!(
            "v2 运行器落格失败：{e:?}"
        )))
    })?;
    // 分发方产生的消息（工具轮）——先取走再释放，避免它随分发方一起消失。
    let produced = dispatcher.take_produced();
    // 收束派生事实出参同样先取走（承诺 / 任务表 / 熔断）——轮末随收束落格。
    let derived = dispatcher.take_derived();
    drop(dispatcher);
    let outcome = match end {
        super::DriveEnd::Finished(outcome) => outcome,
        super::DriveEnd::Answered => {
            // 草稿与汇报落地面各持一份桥：先丢它们，`drop(bridge)` 才是最后一个强引用。
            drop(session);
            drop(report_landing);
            drop(bridge);
            // 发射任务没排在收束前退出只影响实时面的收束帧，而本幕**已经**收束：
            // 丢弃即可，不把它升级成错误。
            if let Err(e) = emitter.await {
                symbio_core::plugin_warn!(
                    "session",
                    "[Turn] 流式发射任务未正常退出（本幕已收束，继续）: {e}"
                );
            }
            return Ok(OpenedFlow::Answered);
        }
    };
    Ok(OpenedFlow::Running(Box::new(Running {
        session: loop_session,
        store,
        bridge,
        emitter,
        outcome,
        produced,
        derived,
        collected,
        turn_no,
        user_seq,
        principal,
        to,
        user_text,
        root_id,
        presentation_tree,
    })))
}

impl<'s> Assembled<'s> {
    /// 本幕在开轮就收束（[`Opened::Done`]）：没有环，也没有产出的消息 / 派生事实。
    ///
    /// 与 [`drive_opened`] 同一条出口（[`Running`]），因为「中止是一轮的状态」与
    /// 「正常收束」在阶段③④ 走同一段归类与落库——差别只在 `outcome` 说了什么。
    pub(crate) fn into_running<'a>(
        self,
        outcome: super::super::turn_runner::TurnOutcome,
    ) -> Running<'a>
    where
        's: 'a,
    {
        let Assembled {
            session,
            store,
            bridge,
            emitter,
            turn_no,
            user_seq,
            principal,
            to,
            user_text,
            root_id,
            presentation_tree,
            collected,
            ..
        } = self;
        Running {
            session,
            store,
            bridge,
            emitter,
            outcome,
            produced: Vec::new(),
            derived: Default::default(),
            collected,
            turn_no,
            user_seq,
            principal,
            to,
            user_text,
            root_id,
            presentation_tree,
        }
    }
}

/// 轮边界注入的唯一挂点：抽干口（`None` = 本会话没装收件箱）。
///
/// 汇报不挂在这里：它是 `policy.report` 行（D-163），判定归行表、落格与上线归行
/// 自己的执行体。这里只剩「用户有没有中途插话」这一件 core 认得的事。
fn injector(supplements: Option<SupplementFn>) -> Option<SupplementFn> {
    supplements
}

/// 抽干结果 → **落格 + 构造注入消息**（缺口 3 的全部逻辑，闭包之外）。
///
/// 为什么抽出来：闭包在 P3c 里已经 90 行，且一半是「抽干 / 落格 / 造节点」的直线代码。
/// 缺口 4 要往同一个闭包里再加一段汇报逻辑，不抽出来的话那个闭包会奔到 200 行——
/// 而它的可读性靠的正是「一眼看到这一轮边界上依次发生了什么」。
///
/// 返回**空 vec** = 本轮无可注入（没有补充 / 正文为空 / 拿不到溯源锚点）。三种情形
/// 都**不消耗任何配额**、不留半截状态，与 `apply_verdict` 拿不到措辞时同一条方向。
#[allow(clippy::too_many_arguments)] // 抽出来之后参数就是它的全部依赖，多一个都该重新想
pub(crate) fn supplement_into_request(
    batch: Option<symbio_core::DrainedSupplement>,
    anchor: Option<u64>,
    store: &dyn Store<Event = Event>,
    actor: &ActorSpec,
    turn_no: u64,
    sink_msgs: &Arc<std::sync::Mutex<Vec<ChatNode>>>,
    no_counter: &std::sync::atomic::AtomicU64,
) -> Vec<symbio_core::PromptMessage> {
    let Some(drained) = batch else {
        return Vec::new();
    };
    let (text, count, ids) = (drained.text, drained.count, drained.ids);
    if text.trim().is_empty() {
        return Vec::new();
    }
    // **落格**：成为一格事实（`turn × asserted`）。不落格的话它只活在这一次请求里，
    // 下一轮又消失了——而「用户说过的话没有变成事实」正是缺口 3 本身。
    //
    // 事件 id 带序号（`s-{turn}-{no}`）：一次工具循环里可能折进多条，不带序号会撞
    // 幂等键（`Duplicate`）。
    //
    // 拿不到溯源锚点 ⇒ **不落也不注入**：I2 要求断言类事件带溯源，而无溯源的断言
    // 事件比不落更难查。
    let Some(anchor) = anchor else {
        symbio_core::plugin_warn!(
            "session",
            "[Turn] 补充无溯源锚点，本轮丢弃（见本函数「拿不到溯源锚点」处）"
        );
        return Vec::new();
    };
    let no = no_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if let Err(e) = store.append(
        symbio_core::Event::pending(
            format!("s-{turn_no}-{no}"),
            symbio_plugin_api::events::EVENT_TURN_SUPPLEMENTED,
            symbio_core::Entity::Turn,
            symbio_core::Verb::Asserted,
            turn_no,
            &actor.principal,
        )
        .with_produced_by(anchor)
        // 载荷带上**节点身份**（原则：节点身份是事实的一部分）：重建按 `id`
        // 复刻这条用户消息的身份、按 `ids` 复刻 `meta.supplement_ids`。缺了它们，
        // 重建只能合成 id（每次重开都换一批），且合并标记里的原始条目 id 无处可寻
        // ——而「合并后原始条目只此一处留痕」（见 `transcript::supplements` 的模块头）。
        .with_payload(serde_json::json!({
            "text": text,
            "count": count,
            "id": drained.id.clone(),
            "ids": ids.clone(),
        })),
    ) {
        // 落格失败**不静默**：消息仍会进这一次请求，但事实网格里没有它 ⇒ 下一轮起
        // 不可见。所以出声。
        //
        // `{:?}` 不是 `{e}`：`AppendError` 只 derive 了 `Debug`。
        symbio_core::plugin_warn!(
            "session",
            "[Turn] 补充落格失败（仍注入本轮，下一轮起不可见）: {e:?}"
        );
    }
    let msg = ChatNode {
        // 沿用第一条补充的 id（见 `DrainedSupplement::id` 的说明）
        id: drained.id.clone(),
        principal: Some(symbio_core::principal::Principal::from_legacy(
            symbio_core::PRINCIPAL_USER,
        )),
        content: Some(MessageContent::Text(text.clone())),
        meta: Some(serde_json::json!({
            symbio_core::META_SUPPLEMENT: true,
            symbio_core::META_SUPPLEMENT_COUNT: count,
            symbio_core::META_SUPPLEMENT_IDS: ids,
        })),
        ..Default::default()
    };
    // 锁拿不到就**跳过回灌**，不 panic：这条补充**已经落格、已经进了本轮请求**，
    // 用户看得见——丢的只是落库镜像里的一条（重启后少它）。为这个 panic 不值。
    if let Ok(mut sink) = sink_msgs.lock() {
        sink.push(msg);
    }
    vec![symbio_core::PromptMessage {
        role: "user".into(),
        text,
        tool_call_id: None,
        tool: None,
        tool_calls: None,
    }]
}
