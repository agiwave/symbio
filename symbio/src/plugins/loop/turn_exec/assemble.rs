//! 阶段① 装配——`execute_turn` 拆分四段之首（T-179 · B2，PL-015 §5 动作 B2）。
//!
//! 拆交付包、取事实源、补落寄存变更、定轮号与续写锚、建收集器与流式桥（连同
//! 发射任务）、判反射命中、派生档位与身份、压缩入格——随后把一切装进
//! [`Assembled`] 原样交给阶段② [`super::call`]。本段不产任何模型调用。
//!
//! **纯结构拆分契约**：[`assemble_turn`] 体内自 `turn_exec.rs` 的
//! `execute_turn` 首段**逐字搬移**，语句顺序与副作用点位不变；跨模块引用仅做
//! `super::` → `super::super::` 的两跳改写与文档链接深度修正（胶水），零语义。

use super::*;

/// 阶段① ⇒ 阶段②③④ 的交付束：装配段读出/构造、且消费段还要用的一切按值带走。
///
/// 与 [`ExecTurn`] 同族（捆包而非长参数表）。刻意把「①用过但②③④也要用」的字段
/// 一并带上——回头重取就是第二个读数点，正是本拆分要避免的形状。
pub(crate) struct Assembled<'a> {
    // —— 交付包直通（阶段②③④ 消费）——
    pub(crate) session: &'a LoopSession,
    pub(crate) provider: Arc<dyn ModelProvider>,
    pub(crate) parent: Option<Arc<dyn Plugin>>,
    pub(crate) ctx: Arc<dyn crate::symbio_core::PluginInvokeRequest>,
    pub(crate) system_prompt: &'a str,
    pub(crate) abort: ExecAbortSignal,
    pub(crate) root_id: &'a str,
    pub(crate) sink: &'a ExecEventSink,
    pub(crate) user_text: &'a str,
    pub(crate) user_node_id: Option<String>,
    pub(crate) user_meta: Option<serde_json::Value>,
    pub(crate) window_turns: u64,
    pub(crate) tools: &'a [CapabilityMeta],
    pub(crate) supplements: Option<SupplementFn>,
    pub(crate) on_round: Option<RoundHookFn>,
    pub(crate) supplemental_no: Arc<std::sync::atomic::AtomicU64>,
    pub(crate) recalled: Option<&'a crate::symbio_core::RecallView>,
    pub(crate) skill_obs: &'a [(String, bool)],
    pub(crate) prefix: Option<String>,
    pub(crate) stages: Vec<super::super::stage_events::StageFact>,
    pub(crate) opening_text: Option<&'a str>,
    pub(crate) opening_dialog: Option<super::super::turn_runner::DialogFacts>,
    pub(crate) presentation_tree: crate::symbio_core::node_build::PresentationTree,
    // —— 阶段① 的读数与构造 ——
    pub(crate) store: Arc<dyn Store<Event = Event>>,
    pub(crate) turn_no: u64,
    pub(crate) resume_anchor: Option<TurnResume>,
    pub(crate) collected: Arc<std::sync::Mutex<Vec<ChatNode>>>,
    pub(crate) bridge: Arc<UiBridge>,
    pub(crate) emitter: tokio::task::JoinHandle<()>,
    pub(crate) hit: Option<&'a super::super::skill_events::SkillLlmHit>,
    pub(crate) tier: LatencyTier,
    pub(crate) actor: ActorSpec,
    pub(crate) principal: String,
    pub(crate) to: String,
}

/// 一轮的执行（阶段①）：装配——`execute_turn` 首段原样，产 [`Assembled`]。
pub(crate) async fn assemble_turn<'a>(req: ExecTurn<'a>) -> Result<Assembled<'a>, TurnExecExit> {
    let ExecTurn {
        session,
        provider,
        parent,
        ctx,
        system_prompt,
        abort,
        root_id,
        sink,
        user_text,
        user_node_id,
        user_meta,
        window_turns,
        tools,
        resume,
        recalled,
        supplements,
        on_round,
        supplemental_no,
        skill_obs,
        skill_hits,
        prefix,
        consolidation,
        stages,
        opening_text,
        opening_dialog,
        presentation_tree,
    } = req;
    // 事实源：本会话的**唯一句柄**（由交付包按值带来、按引用分发）。
    // 与运行器落格、收件箱补落、轮外压缩进的是同一份实例——进程内不存在
    // 第二个各自 replay 的写者（ADR-084 §2）。
    let store = session.facts().ok_or_else(|| {
        PluginError::InternalError("v2 执行需要持久会话（临时会话无事实源）".into())
    })?;
    // **补落寄存的变更**（T-055）：跨进程另一写者持令牌时，本进程的编辑 / 截断
    // 会得到 `NotTheWriter` ⇒ 该变更被寄存在会话侧。本函数是持有交付包的时刻，
    // 经端口动作让 session 在**它自己的令牌窗口**里补落。
    //
    // 位置在 `snapshot` 之前：本函数后面的所有判据（turn 号 / 续写锚 / 幂等键）
    // 都从 `store.range(...)` 折，补落改的就是那份切片 ⇒ 必须在读切片之前落完，
    // 否则本轮会站在一份**缺了那次编辑**的状态上做判断。
    if let Err(e) = session.run_op_unit(run_ops::DRAIN_PARKED, ()).await {
        crate::plugin_warn!("session", "[Turn] 补落寄存变更失败（本轮继续）: {e}");
    }
    let snapshot = store.range(Seq::new(0));

    // turn 号与续写锚点：
    // - 新开轮：号 = WAL 内既有 `user.message` 计数；
    // - 续写轮：号 = **已开未收束**的那一轮（审批 / 问答恢复），并取它的用户格 seq
    //   作溯源锚点——续写**不新开用户格**，收束仍记在该轮上（C4 按 turn 配对，
    //   另开新轮会把原轮变成永久假缺口，见 core `TurnResume`）。
    let (turn_no, resume_anchor) = match &resume {
        None => (count_user_messages(&snapshot), None),
        Some(r) => {
            // 未收束的那一轮在**完整史**里找（含归档前缀，ADR-085）：快照取代的是
            // 消息列表，不是轮次生命周期——等待用户动作的轮在压缩落格后照样「已开
            // 未收束」（C4 也读完整史）。按现行窗口找会把它漏成 `InternalError`
            // （T-169：压缩恰好在它等审批期间落格）。轮号与锚点同读一份切片。
            let (turn, user_seq) = last_open_turn(&snapshot).ok_or_else(|| {
                PluginError::InternalError(
                    "续写轮找不到未收束的轮次（事实源与恢复路径不一致）".into(),
                )
            })?;
            (
                turn,
                Some(TurnResume {
                    user_seq,
                    call: TurnToolCallInfo {
                        id: None,
                        wire_id: None,
                        name: Some(r.name.clone()),
                        arguments: r.args.clone(),
                        parse_error: None,
                        overflow_notice: None,
                    },
                    text: r.text.clone(),
                    // 节点身份（调用 / 轮根 / 结果）随恢复方直给的取值入格——续写轮的
                    // `artifact.added` 复用它们，重建才把结果挂回**原来那张卡**之下。
                    call_id: Some(r.call_id.clone()),
                    turn_id: Some(r.turn_id.clone()),
                    result_id: Some(r.result_id.clone()),
                }),
            )
        }
    };

    // 折进的补充消息的收集器（缺口 3）：注入闭包（`'static`）往它 push，
    // 执行完随 `ExecTurnResult` 回灌进 `context.messages`（落库动作就是
    // `context.messages.push(merged)`）。不回灌就只活在这一次请求里，
    // 重启即丢，而 t19 等的正是落库那条。
    let collected: Arc<std::sync::Mutex<Vec<ChatNode>>> =
        Arc::new(std::sync::Mutex::new(Vec::new()));

    // 流式桥：分帧侧（同步回调）与发射侧（异步任务）经 unbounded 通道解耦。
    // 通道在桥 drop（生成结束）后关闭，接收端自然退出。
    let tx = Arc::new(FrameQueue::new(UI_FRAME_CAPACITY));
    let bridge = Arc::new(UiBridge {
        tx: tx.clone(),
        root_id: root_id.to_string(),
        node: Arc::new(Mutex::new(None)),
        reasoning: Arc::new(Mutex::new(None)),
    });
    let emit_sink = sink.clone();
    let root_id_owned = root_id.to_string();
    let emitter = tokio::spawn(async move {
        while let Some(frame) = tx.pop().await {
            let message = match frame {
                UiFrame::Snapshot {
                    id,
                    parent,
                    text,
                    kind,
                } => {
                    let mut node = kind.shape(ChatNode {
                        id,
                        parent_id: Some(parent),
                        role: Some(NodeRole::Assistant),
                        msg_type: Some(NodeKind::Body),
                        content: Some(MessageContent::Text(text)),
                        status: Some(NodeStatus::Streaming),
                        ..Default::default()
                    });
                    crate::symbio_core::node_build::detach_if_flat(
                        &mut node,
                        &root_id_owned,
                        presentation_tree,
                    );
                    node
                }
                UiFrame::Delta { id, text } => ChatNode {
                    id,
                    delta: Some(text),
                    ..Default::default()
                },
                UiFrame::Finalize {
                    id,
                    parent,
                    text,
                    kind,
                } => {
                    let mut node = kind.shape(ChatNode {
                        id,
                        parent_id: Some(parent),
                        role: Some(NodeRole::Assistant),
                        msg_type: Some(NodeKind::Body),
                        content: Some(MessageContent::Text(text)),
                        status: Some(NodeStatus::Completed),
                        ..Default::default()
                    });
                    crate::symbio_core::node_build::detach_if_flat(
                        &mut node,
                        &root_id_owned,
                        presentation_tree,
                    );
                    node
                }
                UiFrame::Barrier(done) => {
                    let _ = done.send(());
                    continue;
                }
                // 工具调用快照：model 插件转写面的事实原样到达，这里不做任何改写
                // （改写 = 第二份身份形状，分发方与落库用的是同一份）。
                UiFrame::Tool(m) => *m,
            };
            emit_sink.emit(message).await;
        }
    });

    // 本轮是否走**反射档**（S11 快路）：命中一条已编译技能 ⇒ 以技能正文收束、
    // 一次模型调用都不发生。两关缺一不可：
    //
    // - **逐字命中**（`take_match`：判据是数据，更宽的匹配接进来时改的是它）；
    // - **只对新开轮**：续写轮**不新开用户格**（运行器复用 `resume.user_seq`），而
    //   反射档的落格形状就是新开一轮（`u-{turn}`）⇒ 续写轮走它会撞幂等键
    //   （`AppendError::Duplicate` → Failed 出口），既完不成收束、也把本轮变成错误。
    //   语义上同样说不通：续写要跑的是**刚被用户批准的那次工具**，不是拿技能顶掉它。
    //   故 `resume_anchor` 非空时一律不走快路（判定根本不算，不是算了再丢）。
    // 先做**一次**匹配，再问表"这一格要不要走快路"。判据在
    // `loop_registry` 的 `policy.reflex`（`resume_allowed ∧ has_match`），
    // 本处只提供两个布尔并执行命中结果——`hit` 全程留在本地，表不接触它。
    let matched = super::super::skill_events::take_match(skill_hits, user_text);
    let reflex_state =
        super::super::loop_registry::LoopState::reflex(resume_anchor.is_none(), matched.is_some());
    let reflex_hit = super::super::loop_registry::builtin()
        .decide(&reflex_state)
        .hit();
    if let Some(hit) = reflex_hit {
        hit.trace();
    }

    // 档位由**命中的那条行**携带（`effects.tier`），不由本处按「有没有命中」
    // 反推：判据与档位是同一条命中上的两个字段（PL-014 §4 机制 2）。
    // `tier` 与 `actor.budget_ms` 是**同一个事实**（一个入格、一个随主体走），
    // 故由前者派生后者——两处各写一遍迟早对不上（S11 §3 的 ActorSpec 行）。
    //
    // 读方（T-082 批①）：派生出的数值随主体走，`turn_runner` 开轮时以**数值**写入
    // 载荷 `budget_ms`，`invariants::declared_budget_ms` 优先读它 ⇒ 该字段不再是
    // "算了却看不见"。`tier` 仍入格（兜底率按层统计的口径锚），且作为缺 `budget_ms`
    // 时的回退路径保留——两条路径读的是同一条事实的两种形态。
    //
    // 其余四条臂属于别的格（`Capsule::owner`）：此处显式不接——穷尽列举是让
    // 「加一条臂」变成编译错误，而不是靠 catch-all 静默放行（PL-015 §4.2）。
    let (hit, tier) = match reflex_hit.map(|h| (h.capsule, h.effects.tier)) {
        Some((super::super::loop_registry::Capsule::Reflex, tier)) => (matched, tier),
        Some((
            super::super::loop_registry::Capsule::Decide
            | super::super::loop_registry::Capsule::Report
            | super::super::loop_registry::Capsule::Compress
            | super::super::loop_registry::Capsule::Recall,
            _,
        ))
        | None => (None, LatencyTier::Deep),
    };
    // 身份进事件（[plan/11 批 1](../../../../../docs/004-plan/009-多执行器与多主体加固实施方案.md "PL-009-多执行器与多主体加固实施方案") ①）：
    // 本轮以**会话选定的那个 agent** 为主体，规格从**授权表行**取（`request_actor_spec`）：
    // 能力集与模式不再是 `trivial` 的空表平凡值——它们与写侧闸判的是同一张表
    // （判的对象 = 写的对象 = 表授予的对象）。未选智能体 ⇒ `agent:main`
    // （S08 §4 平凡值，与接线前逐字一致）。
    //
    // 另存一份 `principal`：`actor` 随 `input` 被移动进运行器，轮末的记忆与学习
    // 写方（`record_learning`）仍要以它为主体——先取字符串，不依赖 `actor` 的存活。
    // 作用域（[T-105]）：`agent_run` 派生的子会话挂父主体，顶层会话恒 `Root`。
    // 与身份同批取——两者都经 `ctx` / 元数据解析，分开取会让「主体是 A、作用域按 B 算」。
    let scope = session.scope();
    let actor = super::super::chat_loop::request_actor_spec(ctx.as_ref(), tier, scope);
    let principal = actor.principal.clone();
    // 来源主体（ADR-067）：`commitment.*` 的 `to`。在此刻取而不是轮末——`ctx` 会被
    // 移进工具分发口（`SessionDispatchPort`），轮末已经拿不到它。
    let to = super::super::chat_loop::request_source_principal(ctx.as_ref());

    // ── 压缩入格 + 归档（S2.7-b2）─────────────────────────────────────────
    //
    // **必须在 `(turn_no, resume_anchor)` 之后**：两者的判据都要求「本轮自己的
    // 用户格还没落」——轮号若把本轮数进去会从 `u-{n}` 跳到 `u-{n+1}`。归档移走
    // 前缀**不影响**续写锚：`last_open_turn` 读完整史（ADR-085），等待轮被压进
    // 归档段后仍是「已开未收束」。
    //
    // 也必须**在运行器之前**：本轮自己的用户格由运行器落，先归档才能让
    // 「本轮之前的一切」被完整取代（否则快照与它后面的新事件之间会夹着被取代的旧事件，
    // 现行窗口不再自洽）。
    if let Some(msgs) = consolidation {
        if let Err(e) = session
            .run_op_unit(
                run_ops::CONSOLIDATE,
                ConsolidateRequest {
                    messages: msgs.to_vec(),
                },
            )
            .await
        {
            crate::plugin_warn!("session", "[Turn] 压缩事实落格失败（压缩已生效）: {e}");
        }
    }
    Ok(Assembled {
        session,
        provider,
        parent,
        ctx,
        system_prompt,
        abort,
        root_id,
        sink,
        user_text,
        user_node_id,
        user_meta,
        window_turns,
        tools,
        supplements,
        on_round,
        supplemental_no,
        recalled,
        skill_obs,
        prefix,
        stages,
        opening_text,
        opening_dialog,
        presentation_tree,
        store,
        turn_no,
        resume_anchor,
        collected,
        bridge,
        emitter,
        hit,
        tier,
        actor,
        principal,
        to,
    })
}

/// **已开未收束**的轮次（`turn × opened` 无同号收束）：返回 `(turn, 用户格 seq)`。
///
/// ## 输入必须是完整史
///
/// 调用方传 `range(0)`（含归档前缀），**不是**现行窗口（[ADR-085](../../../../../docs/010-adr/085.md "ADR-085")）：
/// 压缩快照取代的是消息列表，不是轮次生命周期——等待用户动作的轮在被压缩落格归档后
/// 仍是「已开未收束」（C4 同样读完整史）。按窗口找会把它漏成「找不到未收束的轮次」
/// （T-169：压缩恰好在它等审批期间落格）。
///
/// ## 认什么收束与在哪个范围找，是两件事
///
/// 与 core 的 `invariants::unresolved_turns`（C4）**刻意不同口径**：这里只认
/// final / fallback 收束格、**不认产物格**（C4 经 ADR-067 认）。原因：等待用户
/// 动作的那一轮（进展格不是收束格，但往往已落过产物格）的收束格刻意留给恢复幕
/// ——若这里也把「落过产物」当成已收束，恢复请求会报「找不到未收束的轮次」，
/// 审批 / 问答恢复整条链路断掉。C4 只负责「不再报它静默中断」，恢复锚仍按
/// final / fallback 找——两处口径的差异是 ADR-067 落地时的显式决策，不是漏改。
/// 本函数与 C4 的**范围**（完整史）由 ADR-085 对齐；差异只在收束种类。
/// 取**最后**一个：恢复请求总是续写最近一次停顿。被压缩掉的陈轮不会抢占锚点——
/// 它若已被 final / fallback 收束就不在候选集里；仍开着的轮按开盘序排在它后面。
pub(super) fn last_open_turn(events: &[Event]) -> Option<(u64, u64)> {
    let mut open: Vec<(u64, u64)> = Vec::new();
    for e in events {
        if e.entity == Entity::Turn && e.verb == Verb::Opened && e.kind == EVENT_USER_MESSAGE {
            if let Some(seq) = e.seq {
                open.push((e.turn, seq.value()));
            }
        }
        if e.entity == Entity::Turn
            && e.verb == Verb::Closed
            && matches!(
                e.kind.as_str(),
                EVENT_ASSISTANT_FINAL | EVENT_ASSISTANT_FALLBACK
            )
        {
            open.retain(|(turn, _)| *turn != e.turn);
        }
    }
    open.last().copied()
}
