//! 阶段② 开轮与驱动——幕首建草稿、接缝落格、内环跑完（· B2，DE-021 §5 动作 B2）。
//!
//! 本模块有两条入口，对应草稿的两个时刻（`D-222` 订正 3）：
//!
//! - [`open_turn`]：**幕首**——把 [`Assembled`] 摊成 [`TurnInput`] 交给
//!   `TurnRunner::begin`，产 [`Opened`]（已收束 / 有环要跑）。它一格「要等轮首那几行」
//!   的事实都不落。
//! - [`drive_opened`]：**轮首之后**——接缝（压缩入格 / 定档 / 首响上线）→ 渲染基线 →
//!   填前缀与快路正文 → 内环（[`super::drive_steps`]），产 [`Running`]。
//!
//! 中间那一段（轮首派发点、答话收束、水位、根骨架上线路、请求视图）在
//! `chat_loop::inputs::land_head`，由调用方（`stage::run_stage`）把结果填进
//! [`AfterHead`] 交进来——本模块不知道判决 / 压缩 / 召回跑没跑，只看它们寄存下的值。

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
    pub(crate) recalled: Option<&'a super::super::recall::RecallView>,
    pub(crate) skill_obs: &'a [(String, bool)],
    pub(crate) root_id: &'a str,
    pub(crate) presentation_tree: symbio_core::node_build::PresentationTree,
}

/// 幕首开轮的结果（[`open_turn`] 的出口）。
pub(crate) enum Opened {
    /// 本幕在开轮就收束了（首请求前中止 / 空答兜底）：没有环可跑。
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

/// 轮首那几行跑完之后、交给 [`drive_opened`] 的**寄存物**。
///
/// 每一样都是「轮首的行跑完才知道」的：压缩改的就是消息列表，档位折自
/// `policy.reflex` 的出口格（接缝自己读，不在这里），开场白是判决行 `Escalate`
/// 的首响，前缀是请求视图的产物，召回与技能观测是本轮轮末要用的两个寄存位。
pub(crate) struct AfterHead<'a> {
    /// 请求视图的注入段（`build_request_sections` 的产物，**不落库**）。
    pub(crate) request_sections: &'a [ChatNode],
    /// 本轮之前发生的压缩（S2.7-b2）：`Some` ⇒ 接缝先落 `session.consolidated`。
    pub(crate) consolidation: Option<&'a [ChatNode]>,
    /// 对话面开场白（`Escalate` 首响）：正文与节点事实必须同时在场。
    pub(crate) opening_text: Option<&'a str>,
    pub(crate) opening_dialog: Option<&'a super::super::turn_runner::DialogFacts>,
    /// 快路命中的技能正文（`policy.reflex` 寄存）：`Some` ⇒ 本幕零模型往返。
    pub(crate) reflex: Option<&'a super::super::turn_runner::ReflexAnswer>,
    /// 长期记忆召回视图（轮末 `record_learning` 的入参）。
    pub(crate) recalled: Option<&'a super::super::recall::RecallView>,
    /// 技能路由观测（轮末落 `memory.recalled` 的入参）。
    pub(crate) skill_obs: &'a [(String, bool)],
}

/// 幕首开轮（[`TurnRunner::begin`]）：只建草稿，不落任何「要等轮首那几行」的格。
///
/// 首请求前的中止由 `prepare_turn` 的 1c 接住（落中止收束格、返回 [`Opened::Done`]）
/// ——它排在本函数里、轮首那几行**之前**，所以被中止的幕不再跑判决 / 压缩 / 召回
/// （`D-222` 订正 3 认下的行为变化）。
///
pub(crate) async fn open_turn(a: &Assembled<'_>) -> Result<Opened, TurnExecExit> {
    // 深度档：完整推理（含工具内环）。快路的**判定**已经不在这一层：`policy.reflex`
    // 是轮首 host 的一行，它的出口格（`stage:"reflex"`）与正文随本轮状态到接缝；
    // 本侧只把正文交给收束动作，不自己落 `final`（D-163）。
    let llm: std::sync::Arc<dyn LlmAdapter> = std::sync::Arc::new(ProviderLlmAdapter::for_turn(
        a.provider.clone(),
        a.system_prompt,
        a.abort.clone(),
    ));
    let tok = TokenIssuer::issue_deep();
    // 汇报口随注入闭包进 `TurnInput`（`ReportPort` 是可克隆的能力句柄）。
    let report = a.report.clone();
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
        inject: injector(a.supplements.clone(), report).map(|(drain, report)| {
            let store = a.store.clone();
            let actor = a.actor.clone();
            let turn_no = a.turn_no;
            let supplemental_no = a.supplemental_no.clone();
            // 折进的消息往收集器 push（末尾随 `ExecTurnResult` 回灌进落库镜像）。
            let sink_msgs = a.collected.clone();
            // 汇报「产生即上线」借本轮那条流式桥送帧（`UiBridge::on_tool_frame`）。
            // 克隆而非搬走：`bridge` 之后还要交给运行器，而函数末尾的 `drop(bridge)`
            // 是发射端退出**唯一**的信号——多养一份它，等于把那个信号推迟到闭包死。
            // 闭包随 `TurnInput` / `drop(dispatcher)` 在本轮内死去，因此那个信号
            // 不会被推过收束点；这条时序是 hang 级别的（每个 v2 工具轮都要过它），
            // 由 `round_report_reaches_the_live_surface_before_turn_end` 钉住。
            let live = a.bridge.clone();
            std::sync::Arc::new(
                move |anchor: Option<u64>|
                    -> std::pin::Pin<
                        Box<
                            dyn std::future::Future<Output = Vec<symbio_core::PromptMessage>>
                                + Send,
                        >,
                    > {
                    // `async move` 块按值捕获，所以这里先各 clone 一份——
                    // 否则第一次调用就把捕获物搬走，闭包退化成 `FnOnce`。
                    let drain = drain.clone();
                    let report = report.clone();
                    let store = store.clone();
                    let actor = actor.clone();
                    let sink_msgs = sink_msgs.clone();
                    let supplemental_no = supplemental_no.clone();
                    let live = live.clone();
                    Box::pin(async move {
                        // 抽干口本身就是异步的（会话动作面），直接 await——
                        // 搬家前那套 `blocking_read` + `block_in_place` 的桥接
                        // 随队列一起留在了 session，这里不再需要判 runtime flavor。
                        let batch = drain().await;
                        // 顺序：**先抽干、后汇报**（同一条纪律）——抽干会把静默时钟
                        // 归零，反过来就会出现「用户刚说完话，助手抢着报了一句进度」。
                        let injected = supplement_into_request(
                            batch,
                            anchor,
                            store.as_ref(),
                            &actor,
                            turn_no,
                            &sink_msgs,
                            &supplemental_no,
                        );

                        // 批次边界的**派发点**（D-163）：这一刻「跑完了一批工具」
                        // 交给引擎，该不该说由行 `policy.report` 自己判——静默 /
                        // 配额 / 轮数三格都在 `report_due` v4 的输出里。本侧不再
                        // 问一句「欠不欠」，也不再自己数批次（旧的 `AtomicUsize`
                        // 与 `RoundHookFn` 就这样退的场）。
                        //
                        // 「先抽干、后汇报」仍是顺序纪律：上面那次抽干已经把补充格
                        // 落进日志，静默锥随之前移（判据 12）。
                        // 汇报的**落格与上线面**（D-163）就在这个闭包里：事实源
                        // （store）、实时桥（`live`）、落库镜像（`sink_msgs`）与
                        // 序号游标。三台 host 各自手里有自己那片，这里恰好是批次边界。
                        if let Some(port) = report.as_ref() {
                            match anchor {
                                Some(anchor) => {
                                    let landing = super::super::report_exec::ReportLanding {
                                        collected: sink_msgs.clone(),
                                        live: live.clone(),
                                        no: supplemental_no.clone(),
                                    };
                                    if let Err(e) = super::super::report_exec::drive_report(
                                        store.as_ref(),
                                        turn_no,
                                        port,
                                        anchor,
                                        &actor.principal,
                                        &landing,
                                    )
                                    .await
                                    {
                                        // 汇报落格失败不升级成整轮失败（与定稿答话轮同一条
                                        // 口径）：用户已经少听了一句，不该再多一个错误。
                                        symbio_core::plugin_warn!(
                                            "session",
                                            "[Turn] 汇报派发失败（本轮继续，账仍开着）: {e:?}"
                                        );
                                    }
                                }
                                None => symbio_core::plugin_warn!(
                                    "session",
                                    "[Turn] 批次边界没有溯源锚点，本批不派发汇报（I2：断言类事件必须带溯源）"
                                ),
                            }
                        }

                        injected
                    })
                }) as super::super::turn_runner::RoundInjector
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
        })
            as super::super::turn_runner::NodeFactsFn),
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
    match super::TurnRunner::begin(a.store.as_ref(), llm.as_ref(), input).await {
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

/// 接缝之后把草稿跑完：工具分发方 → 接缝落格 → 渲染基线 → 填前缀与快路 → 内环。
///
/// ## 次序是硬的
///
/// - 分档与首响都在接缝上（`land_seam`），它们读的是轮首那几行的出口格；
/// - 基线 prompt 在接缝**之后**渲染：压缩改的就是消息列表，而压缩刚在上一行落格；
/// - 三个「填」也有序：先渲染，再插前缀（前缀往基线最前面插一条），最后寄存快路正文。
///
/// ## 两个生命周期
///
/// `Assembled` 活到本函数结束（它持有会话句柄与事实源），而 `after` 借的是调用方的
/// 本轮状态（寄存位）——两者寿命不同，`Running` 按**短的那个**成形（`'s: 'a`）：
/// 它只活到本轮收束，收束一完调用方就要写回 `turn`。
pub(crate) async fn drive_opened<'s, 'a>(
    a: Assembled<'s>,
    open: OpenRun,
    after: AfterHead<'a>,
) -> Result<Running<'a>, TurnExecExit>
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
        supplemental_no: _,
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
        ctx,
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
    // 接缝（`D-223` 订正 1 / 订正 2）：压缩入格 → 定档 → 首响上线。
    // 它在 `begin` 之后、任何选债之前——三格事实都要等轮首那几行跑完，
    // 而草稿要先于它们存在（`D-222` 订正 3），两件事只在这一点上交汇。
    super::assemble::land_seam(super::assemble::Seam {
        session: loop_session,
        store: store.as_ref(),
        bridge: bridge.as_ref(),
        turn_no,
        user_seq,
        drive_seq,
        actor: &actor,
        consolidation: after.consolidation,
        opening_text: after.opening_text,
        opening_dialog: after.opening_dialog,
    })
    .await
    .map_err(|e| {
        TurnExecExit::Failed(PluginError::InternalError(format!(
            "v2 运行器落格失败：{e:?}"
        )))
    })?;
    // 基线 prompt 在接缝**之后**渲染：压缩改的就是消息列表，而压缩刚在上一行落格。
    // 三个「填」的次序也是硬的：先渲染，再插前缀（前缀往基线最前面插一条），最后
    // 寄存快路正文（快路正文不进 prompt，它只决定环先选中 `close-turn`）。
    session.render_base(store.as_ref());
    session.set_prefix(super::super::chat_loop::request_view_prefix(
        after.request_sections,
    ));
    session.set_reflex(after.reflex.cloned());
    // 步循环在**执行器**里，不在 `turn_runner` 里。
    let outcome = super::drive_steps(
        &mut session,
        &store,
        llm,
        tok,
        bridge.clone(),
        std::sync::Arc::from(tools),
        Some(dispatcher.clone()),
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
    Ok(Running {
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
        recalled: after.recalled,
        skill_obs: after.skill_obs,
        root_id,
        presentation_tree,
    })
}

impl<'s> Assembled<'s> {
    /// 本幕在开轮就收束（[`Opened::Done`]）：没有环，也没有产出的消息 / 派生事实。
    ///
    /// 与 [`drive_opened`] 同一条出口（[`Running`]），因为「中止是一轮的状态」与
    /// 「正常收束」在阶段③④ 走同一段归类与落库——差别只在 `outcome` 说了什么。
    pub(crate) fn into_running<'a>(
        self,
        outcome: super::super::turn_runner::TurnOutcome,
        recalled: Option<&'a super::super::recall::RecallView>,
        skill_obs: &'a [(String, bool)],
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
            recalled,
            skill_obs,
            root_id,
            presentation_tree,
        }
    }

    /// 本幕**一个环都不跑**（定稿答话轮在轮首就地收束）：关桥、等发射端排空。
    ///
    /// 开轮已经建了桥与发射任务（`D-222` 订正 3：草稿排在轮首之前），而那一幕不会
    /// 有任何一个模型往返来关闭它们。发射端只在队列关闭后退出，队列又要等**所有**
    /// 桥的强引用消失——包括草稿里注入闭包那一份。所以调用方必须先丢掉草稿
    /// （`drop(run)`）再调本函数，否则这里会永远等下去。
    pub(crate) async fn discard(self) {
        let Assembled {
            bridge, emitter, ..
        } = self;
        drop(bridge);
        // 发射任务没排在收束前退出只影响实时面的收束帧，而本幕**已经**收束（答话轮
        // 在轮首就地结束）：把一次 UI 桥的时序问题升级成整轮失败，换来的是用户既拿到
        // 正确答案又多一个错误。与汇报落格失败同一条口径。
        if let Err(e) = emitter.await {
            symbio_core::plugin_warn!(
                "session",
                "[Turn] 流式发射任务未正常退出（本幕已收束，继续）: {e}"
            );
        }
    }
}

/// 一轮的执行：事实原生入格 + 生成（含工具循环）+ 流式回 UI。
/// 轮边界注入的两个可选口打成一个闭包（`None` = 两个都没装）。
///
/// 与退役的 `round_hook::round_hook` 同一件事，只是第二个口从「回答要不要说」的
/// 回调换成了「产一句什么话」的能力包（D-163）：判定已经归行表，这一侧只提供时机
/// 与能力。写成两个 `map` 嵌套会得到四种组合，其中「两个都没有」本来就该退化成
/// `None`——挂一个永远返回空的闭包只会让 core 每轮白问一次。
fn injector(
    supplements: Option<SupplementFn>,
    report: Option<super::super::report_exec::ReportPort>,
) -> Option<(SupplementFn, Option<super::super::report_exec::ReportPort>)> {
    if supplements.is_none() && report.is_none() {
        return None;
    }
    let drain: SupplementFn = supplements.unwrap_or_else(|| Arc::new(|| Box::pin(async { None })));
    Some((drain, report))
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
