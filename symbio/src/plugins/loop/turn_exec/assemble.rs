//! 阶段① 装配——`execute_turn` 拆分四段之首（ · B2，DE-021 §5 动作 B2）。
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
    pub(crate) window_turns: u64,
    pub(crate) tools: &'a [CapabilityMeta],
    pub(crate) supplements: Option<SupplementFn>,
    pub(crate) on_round: Option<RoundHookFn>,
    pub(crate) supplemental_no: Arc<std::sync::atomic::AtomicU64>,
    pub(crate) recalled: Option<&'a crate::symbio_core::RecallView>,
    pub(crate) skill_obs: &'a [(String, bool)],
    pub(crate) prefix: Option<String>,
    pub(crate) opening_text: Option<&'a str>,
    pub(crate) opening_dialog: Option<super::super::turn_runner::DialogFacts>,
    pub(crate) presentation_tree: crate::symbio_core::node_build::PresentationTree,
    // —— 阶段① 的读数与构造 ——
    pub(crate) store: Arc<dyn Store<Event = Event>>,
    pub(crate) turn_no: u64,
    pub(crate) user_seq: u64,
    pub(crate) drive_seq: u64,
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
        turn_no,
        user_seq,
        drive_seq,
        window_turns,
        tools,
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
    // 第二个各自 replay 的写者（D-084 §2）。
    let store = session.facts().ok_or_else(|| {
        PluginError::InternalError("v2 执行需要持久会话（临时会话无事实源）".into())
    })?;
    // **补落寄存的变更**：跨进程另一写者持令牌时，本进程的编辑 / 截断
    // 会得到 `NotTheWriter` ⇒ 该变更被寄存在会话侧。本函数是持有交付包的时刻，
    // 经端口动作让 session 在**它自己的令牌窗口**里补落。
    //
    // 位置在运行器落格之前：补落改的就是运行器即将读的那份切片（prompt 窗口 /
    // 产物编号），不先落完，本轮会站在一份**缺了那次编辑**的状态上做判断。
    if let Err(e) = session.run_op_unit(run_ops::DRAIN_PARKED, ()).await {
        crate::plugin_warn!("session", "[Turn] 补落寄存变更失败（本轮继续）: {e}");
    }

    // 一次性阶段（判决 / 召回 / 压缩）：**先落格、再问表**。反射行读的是
    // `turn_head.skill_hit`（召回阶段的结论）——它若还停在草稿里，视图看不见，
    // 快路永远不会被认领。写者令牌此刻在本段手里（`store`），落格与装配同一时刻。
    //
    // 定稿答话轮不经本段（`execute_final_reply_turn`），仍由运行器落；同一格
    // 重落会撞幂等键，`land` 按「已做过」吸收。
    let principal = crate::symbio_core::request_principal(ctx.as_ref());
    super::super::stage_events::land(store.as_ref(), turn_no, user_seq, &principal, &stages)
        .map_err(|e| {
            TurnExecExit::Failed(PluginError::InternalError(format!(
                "一次性阶段事实落格失败：{e:?}"
            )))
        })?;

    // 轮号与溯源锚由编排层在判决之前定下：用户格先落，这里只收三个
    // seq，不再从切片反推。
    // - 新开轮：`turn_no` = 第几条用户格，`user_seq` = 该格的 seq；
    // - 唤醒幕：`turn_no` = **已开未收束**的那一轮（审批 / 问答恢复），`user_seq`
    //   是它的原用户格 seq——唤醒**不新开用户格**，收束仍记在该轮上（C4 按 turn
    //   配对，另开新轮会把原轮变成永久假缺口）。

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
    // - **只对新开轮**：唤醒幕（审批 / 问答恢复）**不新开用户格**（调用方直给原轮的
    //   `user_seq`），而反射档的落格形状就是新开一轮（`u-{turn}`）⇒ 唤醒幕走它会撞幂等键
    //   （`AppendError::Duplicate` → Failed 出口），既完不成收束、也把本轮变成错误。
    //   语义上同样说不通：唤醒要跑的是**刚被用户批准的那次工具**，不是拿技能顶掉它。
    //   故唤醒幕一律不走快路（判定根本不算，不是算了再丢）。
    // 先做**一次**匹配，再问表"这一格要不要走快路"。判据在 `loop_rows` 的
    // `policy.reflex`（`!resumed ∧ skill_hit`）——两个输入都是事实（幕锚与召回
    // 阶段落下的 `result`），本处不再递交内存布尔。`hit` 是行命中之后才取用的
    // 执行体输入，表不接触它。
    let matched = super::super::skill_events::take_match(skill_hits, user_text);
    let slice = store.range(crate::symbio_core::Seq::new(0));
    let (hit, tier) = if super::super::loop_rows::owes(
        "policy.reflex",
        &slice,
        turn_no,
        super::super::loop_rows::wake_now_ms(&slice, drive_seq),
    ) {
        (matched, super::super::loop_rows::tier("policy.reflex"))
    } else {
        (None, LatencyTier::Deep)
    };

    // 档位由**命中的那条行**携带（`loop_rows::tier`），不由本处按「有没有命中」
    // 反推：判据与档位是同一条声明上的两个字段（DE-014 §4 机制 2）。
    // `tier` 与 `actor.budget_ms` 是**同一个事实**（一个入格、一个随主体走），
    // 故由前者派生后者——两处各写一遍迟早对不上（S11 §3 的 ActorSpec 行）。
    //
    // 读方（ 批①）：派生出的数值随主体走，`turn_runner` 开轮时以**数值**写入
    // 载荷 `budget_ms`，`invariants::declared_budget_ms` 优先读它 ⇒ 该字段不再是
    // "算了却看不见"。`tier` 仍入格（兜底率按层统计的口径锚），且作为缺 `budget_ms`
    // 时的回退路径保留——两条路径读的是同一条事实的两种形态。
    // 身份进事件（plan/11 批 1 ①）：
    // 本轮以**会话选定的那个 agent** 为主体，规格从**授权表行**取（`request_actor_spec`）：
    // 能力集与模式不再是 `trivial` 的空表平凡值——它们与写侧闸判的是同一张表
    // （判的对象 = 写的对象 = 表授予的对象）。未选智能体 ⇒ `agent:main`
    // （S08 §4 平凡值，与接线前逐字一致）。
    //
    // `principal` 已在落阶段事实时取过（同一取值点 `request_principal`）：
    // `actor` 随 `input` 被移动进运行器，轮末的记忆与学习写方（`record_learning`）
    // 仍要以它为主体——先取字符串，不依赖 `actor` 的存活。
    // 作用域（[]）：`agent_run` 派生的子会话挂父主体，顶层会话恒 `Root`。
    // 与身份同批取——两者都经 `ctx` / 元数据解析，分开取会让「主体是 A、作用域按 B 算」。
    let scope = session.scope();
    let actor = super::super::chat_loop::request_actor_spec(ctx.as_ref(), tier, scope);
    // 来源主体（D-067）：`commitment.*` 的 `to`。在此刻取而不是轮末——`ctx` 会被
    // 移进工具分发口（`SessionDispatchPort`），轮末已经拿不到它。
    let to = super::super::chat_loop::request_source_principal(ctx.as_ref());

    // ── 压缩入格 + 归档（S2.7-b2）─────────────────────────────────────────
    //
    // 必须在运行器落格**之前**：本轮的用户格已由编排层先落，先归档才能让
    // 「本轮之前的一切」被完整取代（否则快照与它后面的新事件之间会夹着被取代的
    // 旧事件，现行窗口不再自洽）。
    //
    // 归档移走前缀**不影响**续写锚：轮的「已开未收束」判据读完整史（D-085），
    // 等待轮被压进归档段后仍然成立——而且续写锚此刻已经由编排层按 seq 直给，
    // 本段不再从切片折它。
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
/// 调用方传 `range(0)`（含归档前缀），**不是**现行窗口（[D-085](../../session/README.md#d-085-压缩取代的是消息列表不是轮次生命周期恢复锚读完整史归档刀口不动 "symbio/src/plugins/session/README")）：
/// 压缩快照取代的是消息列表，不是轮次生命周期——等待用户动作的轮在被压缩落格归档后
/// 仍是「已开未收束」（C4 同样读完整史）。按窗口找会把它漏成「找不到未收束的轮次」
/// （：压缩恰好在它等审批期间落格）。
///
/// ## 认什么收束与在哪个范围找，是两件事
///
/// 与 core 的 `invariants::unresolved_turns`（C4）**刻意不同口径**：这里只认
/// final / fallback 收束格、**不认产物格**（C4 经 D-067 认）。原因：等待用户
/// 动作的那一轮（进展格不是收束格，但往往已落过产物格）的收束格刻意留给恢复幕
/// ——若这里也把「落过产物」当成已收束，恢复请求会报「找不到未收束的轮次」，
/// 审批 / 问答恢复整条链路断掉。C4 只负责「不再报它静默中断」，恢复锚仍按
/// final / fallback 找——两处口径的差异是 D-067 落地时的显式决策，不是漏改。
/// 本函数与 C4 的**范围**（完整史）由 D-085 对齐；差异只在收束种类。
/// 取**最后**一个：恢复请求总是续写最近一次停顿。被压缩掉的陈轮不会抢占锚点——
/// 它若已被 final / fallback 收束就不在候选集里；仍开着的轮按开盘序排在它后面。
pub(crate) fn last_open_turn(events: &[Event]) -> Option<(u64, u64)> {
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
