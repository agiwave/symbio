//! 阶段② 调用——`execute_turn` 四段之二（T-179 · B2，PL-015 §5 动作 B2）。
//!
//! 两档取其一：反射档经 [`TurnRunner::run_reflex`]（零模型调用），深度档装配
//! [`TurnInput`]（含轮内注入闭包）后经 [`SessionDispatchPort`](super::super::tool_dispatch::SessionDispatchPort)
//! 走 [`TurnRunner::run_with_tools`]。产 [`Running`] 交阶段③ [`super::outcome`]。
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
    pub(crate) recalled: Option<&'a crate::symbio_core::RecallView>,
    pub(crate) skill_obs: &'a [(String, bool)],
    pub(crate) root_id: &'a str,
    pub(crate) presentation_tree: crate::symbio_core::node_build::PresentationTree,
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
        on_round,
        supplemental_no,
        recalled,
        skill_obs,
        prefix,
        opening_text,
        opening_dialog,
        presentation_tree,
        store,
        turn_no,
        user_seq,
        drive_seq,
        resume_anchor,
        collected,
        bridge,
        emitter,
        hit,
        tier,
        actor,
        principal,
        to,
    } = a;
    let mut produced = Vec::new();
    // 工具执行层交回的收束派生事实（承诺 / 任务表 / 熔断）——只有工具轮（深度档）才有，
    // 反射档与无工具轮保持默认空值。轮末随收束落格（写方 = `derived_facts::record_derived`，
    // 见下方）。
    let mut derived = super::super::tool_dispatch::DerivedFacts::default();
    let outcome = match &hit {
        // ── 反射档（S11 §2）：**没有 `llm` 形参**的那条路 ────────────────────
        // "反射档调模型"在类型上写不出来（`run_reflex` 只收 `RuleOnly`）——
        // 这正是 `docs/004-plan/verify/latency_gate.rs::assemble(Reflex)` 的落地形态
        // （反射档装不进 LLM 字段），而不是"装配了模型但约定不调它"。
        Some(h) => {
            crate::plugin_info!(
                "session",
                "[turn-exec] 技能快路命中（skill_id={}），本轮不调模型",
                h.skill_id
            );
            let tok = TokenIssuer::issue_reflex();
            TurnRunner
                .run_reflex(
                    store.as_ref(),
                    &tok,
                    turn_no,
                    user_seq,
                    drive_seq,
                    &actor,
                    &h.content,
                    bridge.clone(),
                    // 阶段事实已由装配段落格（反射行要读 `skill_hit`，它必须
                    // 先成事实才能被认领）；这里不再重落。
                    &[],
                )
                .await
        }
        // ── 深度档：完整推理（与接线前逐字同路）─────────────────────────────
        None => {
            let llm = ProviderLlmAdapter::for_turn(provider, system_prompt, abort.clone());
            let tok = TokenIssuer::issue_deep();
            let input = TurnInput {
                turn: turn_no,
                user_seq,
                drive_seq,
                tier,
                window_turns: Some(window_turns),
                resume: resume_anchor,
                actor: actor.clone(),
                prefix,
                // 一次性阶段已由装配段落格（I2 的溯源锚那时已在）：运行器不再重落。
                stages: Vec::new(),
                // 轮内注入（缺口 3）：用户中途说的话折进**本轮**。
                //
                // core 只知道「加消息」——抽干、合并、**落格**全在这里（插件侧）。
                // core 不该知道「收件箱补充」这个概念（队列、`merge_supplements`、
                // `InboxItem` 都在 `plugins/session`），而落格也不需要 core 参与：
                // 本函数自己就开着 `store`。
inject: super::super::round_hook::round_hook(supplements, on_round).map(|(drain, report)| {
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
                                        Output = Vec<crate::symbio_core::PromptMessage>,
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

                            // 轮边界汇报（缺口 4）：判定与措辞在 chat_loop 那边（它有编排器），
                            // **落格在这里**（本函数开着 store）。所以回调交回来的是一份
                            // **草稿**，不是已落的事实——落格这一半不外包出去。
                            //
                            // 交出去的那个时刻就是「本批抽到了补充」：静默时钟由它归零，
                            // 于是「先抽干、后汇报」不是一句写在注释里的纪律，而是数据。
                            let supplemented_at =
                                (!injected.is_empty()).then(crate::symbio_core::clock_now_ms);
                            if let Some(report) = report {
                                // **落格即上线**（T-136 的另一半）：这句话的设计目的就是「本轮还要
                                // 跑很久，先报一句」，只落格不下发就等于把它压到轮末、把目的抵消。
                                // 节点由 `to_message()` **现造一次**，id 同写进落格载荷——重建侧
                                // （`rebuild::reported_node` 走 `event_payload_node_id`）读的就是它，
                                // 于是实时面与重建面是同一个身份，轮末落库镜像再送一次时按 id 合并，
                                // 不多一张卡（与 `op-*` 那条开场白同纪律）。
                                if let Some(draft) = report(supplemented_at).await {
                                    match anchor {
                                        Some(anchor) => {
                                            let no = supplemental_no.fetch_add(
                                                1,
                                                std::sync::atomic::Ordering::Relaxed,
                                            );
                                            let node = draft.to_message();
                                            if let Err(e) = store.append(
                                                crate::symbio_core::Event::pending(
                                                    format!("rp-{turn_no}-{no}"),
                                                    crate::symbio_core::EVENT_ASSISTANT_REPORTED,
                                                    crate::symbio_core::Entity::Turn,
                                                    crate::symbio_core::Verb::Asserted,
                                                    turn_no,
                                                    &actor.principal,
                                                )
                                                .with_produced_by(anchor)
                                                .with_payload(serde_json::json!({
                                                    "text": draft.text,
                                                    "tool_rounds": draft.tool_rounds,
                                                    "quiet_ms": draft.quiet_ms,
                                                    "id": node.id.clone(),
                                                })),
                                            ) {
                                                crate::plugin_warn!(
                                                    "session",
                                                    "[Turn] 汇报落格失败（仍回灌落库镜像，下一轮起模型看不见）: {e:?}"
                                                );
                                            }
                                            if let Ok(mut sink) = sink_msgs.lock() {
                                                sink.push(node.clone());
                                            }
                                            live.on_tool_frame(&node);
                                        }
                                        None => crate::plugin_warn!(
                                            "session",
                                            "[Turn] 汇报无溯源锚点，本轮丢弃（I2：断言类事件必须带溯源）"
                                        ),
                                    }
                                }
                            }

                            injected
                        })
                    }) as super::super::turn_runner::RoundInjector
                }),
                // 本路**恒**走生成：定稿答话轮（缺口 5）不经这里——它一个模型请求都不发，
                // 由 `execute_final_reply_turn` 单独入口落格（见该函数）。
                final_reply: None,
                // 本路恒走生成 ⇒ 没有对话面节点（定稿答话轮不走这里）。
                dialog: None,
                // 对话面开场白（T-056，`Escalate` 首响）：由 `chat_loop::apply_verdict`
                // 产出、随 `ExecTurn` 递进来，运行器据它落 `chat.assistant.opening` 格
                // （开场白后面照常进工具循环，故不占收束格）。
                opening_text: opening_text.map(str::to_string),
                opening_dialog,
                // 落格身份：本轮 Turn 根 id（载荷 `turn_id`）+ 流式节点取数口。用户格
                // 已由编排层先落（T-225），运行器只收它的 seq。取数口是**快照闭包**：
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
                // 中途中止信号（ADR-052 决策 1）：闸门与首请求之间的空隙由运行器
                // 接管——落完用户格、发第一个请求前检查，已中止则走中止收束格。
                abort: Some(abort.clone()),
            };
            // 工具分发：core 只认契约（`DispatchPort`），实现是插件侧——它持插件宿主、
            // 请求上下文、转写出口与工具存档目录（core 认识这些即违 E-009）。
            //
            // 淘汰闸的部署级判据由 session 折叠成**一个值**（`ADR-086 §4`：全量事件
            // 过墙是 P6「值注入」的反面）；取失败按「未收回」继续并出声——与
            // 「读不出的 WAL 跳过」的既有口径一致。
            let elimination_revoked = session
                .run_op::<bool>(run_ops::ELIMINATION_REVOKED, ())
                .await
                .unwrap_or_else(|e| {
                    crate::plugin_warn!(
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
                bridge.clone(),
                presentation_tree,
                elimination_revoked,
            );
            let dispatch_ref: &dyn DispatchPort = &dispatcher;
            let r = TurnRunner
                .run_with_tools(
                    store.as_ref(),
                    &llm,
                    &tok,
                    input,
                    bridge.clone(),
                    tools,
                    Some(dispatch_ref),
                )
                .await;
            // 分发方产生的消息（工具轮）——先取走再释放，避免它随分发方一起消失。
            produced = dispatcher.take_produced();
            // 收束派生事实出参同样先取走（承诺 / 任务表 / 熔断）——轮末随收束落格。
            derived = dispatcher.take_derived();
            drop(dispatcher);
            r
        }
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
    batch: Option<crate::symbio_core::DrainedSupplement>,
    anchor: Option<u64>,
    store: &dyn Store<Event = Event>,
    actor: &ActorSpec,
    turn_no: u64,
    sink_msgs: &Arc<std::sync::Mutex<Vec<ChatNode>>>,
    no_counter: &std::sync::atomic::AtomicU64,
) -> Vec<crate::symbio_core::PromptMessage> {
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
        crate::plugin_warn!(
            "session",
            "[Turn] 补充无溯源锚点，本轮丢弃（见本函数「拿不到溯源锚点」处）"
        );
        return Vec::new();
    };
    let no = no_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if let Err(e) = store.append(
        crate::symbio_core::Event::pending(
            format!("s-{turn_no}-{no}"),
            crate::symbio_core::EVENT_TURN_SUPPLEMENTED,
            crate::symbio_core::Entity::Turn,
            crate::symbio_core::Verb::Asserted,
            turn_no,
            &actor.principal,
        )
        .with_produced_by(anchor)
        // 载荷带上**节点身份**（T-052 的原则：节点身份是事实的一部分）：重建按 `id`
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
        crate::plugin_warn!(
            "session",
            "[Turn] 补充落格失败（仍注入本轮，下一轮起不可见）: {e:?}"
        );
    }
    let msg = ChatNode {
        // 沿用第一条补充的 id（见 `DrainedSupplement::id` 的说明）
        id: drained.id.clone(),
        role: Some(NodeRole::User),
        content: Some(MessageContent::Text(text.clone())),
        meta: Some(serde_json::json!({
            crate::symbio_core::META_SUPPLEMENT: true,
            crate::symbio_core::META_SUPPLEMENT_COUNT: count,
            crate::symbio_core::META_SUPPLEMENT_IDS: ids,
        })),
        ..Default::default()
    };
    // 锁拿不到就**跳过回灌**，不 panic：这条补充**已经落格、已经进了本轮请求**，
    // 用户看得见——丢的只是落库镜像里的一条（重启后少它）。为这个 panic 不值。
    if let Ok(mut sink) = sink_msgs.lock() {
        sink.push(msg);
    }
    vec![crate::symbio_core::PromptMessage {
        role: "user".into(),
        text,
        tool_call_id: None,
        tool: None,
        tool_calls: None,
    }]
}
