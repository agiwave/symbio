//! 阶段② 调用——`execute_turn` 四段之二（· B2，DE-021 §5 动作 B2）。
//!
//! 本模块只走一条路：装配 [`TurnInput`]（含轮内注入闭包，快路正文随
//! `reflex` 进来）后经 `SessionDispatchPort` 走 [`TurnRunner::run_with_tools`]
//! （内环由 [`super::drive_steps`] 驱动）。产 [`Running`] 交阶段③ [`super::outcome`]。
//!
//! **纯结构拆分契约**：[`call_model`] 体内自 `turn_exec.rs` 的 `execute_turn`
//! 第二段逐字搬移；[`supplement_into_request`] 同批搬入（它是注入闭包的落格半程，
//! 消费方就在本段）。跨模块引用仅做 `super::` → `super::super::` 两跳改写。

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
    pub recalled: Option<&'a super::super::recall::RecallView>,
    pub(crate) skill_obs: &'a [(String, bool)],
    pub(crate) root_id: &'a str,
    pub(crate) presentation_tree: symbio_core::node_build::PresentationTree,
}

/// 一轮的执行（阶段②）：调用——消费 [`Assembled`]，产 [`Running`]。
pub(crate) async fn call_model<'a>(a: Assembled<'a>) -> Result<Running<'a>, TurnExecExit> {
    let Assembled {
        session,
        provider,
        parent,
        ctx,
        system_prompt,
        abort,
        root_id,
        sink,
        user_text,
        window_turns,
        tools,
        supplements,
        report,
        supplemental_no,
        recalled,
        skill_obs,
        prefix,
        reflex,
        opening_text,
        opening_dialog,
        presentation_tree,
        store,
        turn_no,
        user_seq,
        drive_seq,
        collected,
        bridge,
        emitter,
        tier,
        actor,
        principal,
        to,
    } = a;
    // 深度档：完整推理（含工具内环）。快路的**判定**已经不在这一层：`policy.reflex`
    // 是轮首 host 的一行，它的出口格（`stage:"reflex"`）与正文（`reflex`）随本轮
    // 状态到这里；本侧只把正文交给收束动作，不自己落 `final`（D-163）。
    let (outcome, produced, derived) = {
            let llm = ProviderLlmAdapter::for_turn(provider, system_prompt, abort.clone());
            let tok = TokenIssuer::issue_deep();
            let input = TurnInput {
                turn: turn_no,
                user_seq,
                drive_seq,
                tier,
                // 快路正文：`Some` ⇒ 本幕已在轮首作答，收束动作直落正文（零往返）。
                reflex: reflex.cloned(),
                window_turns: Some(window_turns),
                actor: actor.clone(),
                prefix,
                // 轮内注入（缺口 3）：用户中途说的话折进**本轮**。
                //
                // core 只知道「加消息」——抽干、合并、**落格**全在这里（插件侧）。
                // core 不该知道「收件箱补充」这个概念（队列、`merge_supplements`、
                // `InboxItem` 都在 `plugins/session`），而落格也不需要 core 参与：
                // 本函数自己就开着 `store`。
inject: injector(supplements, report).map(|(drain, report)| {
                    let store = store.clone();
                    let actor = actor.clone();
                    // 折进的消息往收集器 push（末尾随 `ExecTurnResult` 回灌进落库镜像）。
                    let sink_msgs = collected.clone();
                    // 汇报「产生即上线」借本轮那条流式桥送帧（`UiBridge::on_tool_frame`）。
                    // 克隆而非搬走：`bridge` 之后还要交给运行器，而函数末尾的 `drop(bridge)`
                    // 是发射端退出**唯一**的信号——多养一份它，等于把那个信号推迟到闭包死。
                    // 闭包随 `TurnInput` / `drop(dispatcher)` 在本轮内死去，因此那个信号
                    // 不会被推过收束点；这条时序是 hang 级别的（每个 v2 工具轮都要过它），
                    // 由 `round_report_reaches_the_live_surface_before_turn_end` 钉住。
                    let live = bridge.clone();
                    std::sync::Arc::new(
                        move |anchor: Option<u64>|
                            -> std::pin::Pin<
                                Box<
                                    dyn std::future::Future<
                                        Output = Vec<symbio_core::PromptMessage>,
                                    > + Send,
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
                // 对话面开场白（`Escalate` 首响）：由 `chat_loop::apply_verdict`
                // 产出、随 `ExecTurn` 递进来，运行器据它落 `chat.assistant.opening` 格
                // （开场白后面照常进工具循环，故不占收束格）。
                opening_text: opening_text.map(str::to_string),
                opening_dialog,
                // 落格身份：本轮 Turn 根 id（载荷 `turn_id`）+ 流式节点取数口。用户格
                // 已由编排层先落，运行器只收它的 seq。取数口是**快照闭包**：
                // 运行器每轮收束 / 进展 / 兜底时问一次「此刻桥上攒着哪两个节点」，
                // 中途正文与思考因此逐轮入格（半截中止时它们是唯一幸存者）。
                turn_root_id: root_id.to_string(),
                nodes: Some(std::sync::Arc::new({
                    let bridge = bridge.clone();
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
                abort: Some(abort.clone()),
            };
            // 工具分发：契约与实现都在插件侧（D-118 决策 5）——它持插件宿主、
            // 请求上下文、转写出口与工具存档目录（core 认识这些即违 E-009）。
            //
            // 淘汰闸的部署级判据由 session 折叠成**一个值**（`D-086 §4`：全量事件
            // 过墙是 P6「值注入」的反面）；取失败按「未收回」继续并出声——与
            // 「读不出的 WAL 跳过」的既有口径一致。
            let elimination_revoked = session
                .run_op::<bool>(run_ops::ELIMINATION_REVOKED, ())
                .await
                .unwrap_or_else(|e| {
                    symbio_core::plugin_warn!(
                        "session",
                        "[Turn] 淘汰闸判据读取失败（按未收回处理，本轮继续）: {e}"
                    );
                    false
                });
            let dispatcher = super::super::tool_dispatch::SessionDispatchPort::new(
                parent,
                ctx,
                sink.clone(),
                session.tool_archive_dir().to_path_buf(),
                store.clone(),
                abort.clone(),
                root_id.to_string(),
                turn_no,
                user_seq,
                drive_seq,
                bridge.clone(),
                presentation_tree,
                elimination_revoked,
            );
            let dispatch_ref: &dyn super::super::tool_dispatch::ToolDispatch = &dispatcher;
            // 步循环在**执行器**里，不在 `turn_runner` 里。
            let r = super::drive_steps(
                store.as_ref(),
                &llm,
                &tok,
                input,
                bridge.clone(),
                tools,
                Some(dispatch_ref),
                super::super::loop_rows::ACTORS,
            )
            .await;
            // 分发方产生的消息（工具轮）——先取走再释放，避免它随分发方一起消失。
            let produced = dispatcher.take_produced();
            // 收束派生事实出参同样先取走（承诺 / 任务表 / 熔断）——轮末随收束落格。
            let derived = dispatcher.take_derived();
            drop(dispatcher);
            r.map(|r| (r, produced, derived))
        }
    .map_err(|e| PluginError::InternalError(format!("v2 运行器落格失败：{e:?}")))?;
    Ok(Running {
        session,
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
        recalled,
        skill_obs,
        root_id,
        presentation_tree,
    })
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
            symbio_core::EVENT_TURN_SUPPLEMENTED,
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
