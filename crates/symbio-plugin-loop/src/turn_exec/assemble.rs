//! 阶段① 装配——`execute_turn` 拆分四段之首（· B2，DE-021 §5 动作 B2）。
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
    pub(crate) ctx: Arc<dyn symbio_core::PluginInvokeRequest>,
    pub(crate) system_prompt: &'a str,
    pub(crate) abort: ExecAbortSignal,
    pub(crate) root_id: &'a str,
    pub(crate) sink: &'a ExecEventSink,
    pub(crate) user_text: &'a str,
    pub(crate) window_turns: u64,
    pub(crate) tools: &'a [CapabilityMeta],
    pub(crate) supplements: Option<SupplementFn>,
    pub(crate) report: Option<super::super::report_exec::ReportPort>,
    pub(crate) supplemental_no: Arc<std::sync::atomic::AtomicU64>,
    pub recalled: Option<&'a super::super::recall::RecallView>,
    pub(crate) skill_obs: &'a [(String, bool)],
    pub(crate) prefix: Option<String>,
    pub(crate) opening_text: Option<&'a str>,
    pub(crate) opening_dialog: Option<super::super::turn_runner::DialogFacts>,
    pub(crate) presentation_tree: symbio_core::node_build::PresentationTree,
    // —— 阶段① 的读数与构造 ——
    pub(crate) store: Arc<dyn Store<Event = Event>>,
    pub(crate) turn_no: u64,
    pub(crate) user_seq: u64,
    pub(crate) drive_seq: u64,
    pub(crate) collected: Arc<std::sync::Mutex<Vec<ChatNode>>>,
    pub(crate) bridge: Arc<UiBridge>,
    pub(crate) emitter: tokio::task::JoinHandle<()>,
    /// 本幕快路已作答的技能正文（`Some` ⇒ 本轮零模型往返，D-163）。
    pub(crate) reflex: Option<&'a super::super::turn_runner::ReflexAnswer>,
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
        report,
        supplemental_no,
        skill_obs,
        reflex,
        prefix,
        consolidation,
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
        symbio_core::plugin_warn!("session", "[Turn] 补落寄存变更失败（本轮继续）: {e}");
    }

    // 一次性阶段**不在这里落**：环上的每一行（判决 / 召回 / 压缩）由它自己的执行体
    // 当场入日志（D-159 裁决 4）——装配段不是任何一行的出口。
    let principal = symbio_core::request_principal(ctx.as_ref());
    // 节点身份的同一份主体：流式帧与落库副本必须说同一个人（D-147）。
    let session_principal = symbio_core::principal::Principal::from_legacy(&principal);

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
    let principal_owned = session_principal.clone();
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
                        principal: Some(principal_owned.clone()),
                        msg_type: Some(NodeKind::Body),
                        content: Some(MessageContent::Text(text)),
                        status: Some(NodeStatus::Streaming),
                        ..Default::default()
                    });
                    symbio_core::node_build::detach_if_flat(
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
                        principal: Some(principal_owned.clone()),
                        msg_type: Some(NodeKind::Body),
                        content: Some(MessageContent::Text(text)),
                        status: Some(NodeStatus::Completed),
                        ..Default::default()
                    });
                    symbio_core::node_build::detach_if_flat(
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

    // 本轮档位的来源是**轮首已经选中快路**这件事本身（D-163）：`policy.reflex`
    // 的执行体在轮首 host 落了出口格（`turn.stage{stage:"reflex"}`）并把正文寄进
    // 本轮状态，`ExecTurn::reflex` 是那件事的携带面。本段不再问表「要不要走快路」，
    // 也不重复匹配——判据（逐字命中 ∧ 只对新开轮）与档位是同一条声明上的两个字段
    // （DE-014 §4 机制 2），这一侧只消费结果。
    //
    // 档位由**命中的那条行**携带（`loop_rows::tier`），不由本处按「有没有正文」
    // 反推：判据已在轮首问过表，这里只把那条行的 tier 取出来。
    let tier = if reflex.is_some() {
        super::super::loop_rows::tier("policy.reflex")
    } else {
        LatencyTier::Deep
    };

    // `tier` 与 `actor.budget_ms` 是**同一个事实**（一个入格、一个随主体走），
    // 故由前者派生后者——两处各写一遍迟早对不上（S11 §3 的 ActorSpec 行）。
    //
    // 读方：派生出的数值随主体走，`turn_runner` 开轮时以**数值**写入
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
            symbio_core::plugin_warn!("session", "[Turn] 压缩事实落格失败（压缩已生效）: {e}");
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
        report,
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
        reflex,
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
/// 调用方传 `range(0)`（含归档前缀），**不是**现行窗口（[D-085](../../../symbio-plugin-session/README.md#d-085-压缩取代的是消息列表不是轮次生命周期恢复锚读完整史归档刀口不动 "crates/symbio-plugin-session/src/README")）：
/// 压缩快照取代的是消息列表，不是轮次生命周期——等待用户动作的轮在被压缩落格归档后
/// 仍是「已开未收束」（C4 同样读完整史）。按窗口找会把它漏成「找不到未收束的轮次」
/// （压缩恰好在它等审批期间落格）。
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
                EVENT_CHAT_ASSISTANT_FINAL | EVENT_CHAT_ASSISTANT_FALLBACK
            )
        {
            open.retain(|(turn, _)| *turn != e.turn);
        }
    }
    open.last().copied()
}
