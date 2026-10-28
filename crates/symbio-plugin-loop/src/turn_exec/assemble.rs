//! 阶段① 装配——一轮执行四段之首（· B2，DE-021 §5 动作 B2）。
//!
//! 拆交付包、取事实源、补落寄存变更、建收集器与流式桥（连同发射任务）、定主体与
//! 来源主体——随后把一切装进 [`Assembled`] 原样交给阶段② [`super::call`]。
//! 本段不产任何模型调用，也**不落任何「要等轮首那几行」的格**（压缩 / 档位 / 首响）：
//! 那些是接缝的事，[`land_seam`] 管。
//!
//! ## 为什么分幕首与接缝两段
//!
//! 草稿要排到幕首（`D-222` 订正 3：`Ctx.session` 不是 `Option`），而三格事实要等
//! `policy.compress` / `policy.reflex` / 判决那几行跑完才有（`D-223` 订正 1 / 订正 2）。
//! 两件事落在同一段里，就必然有一头是错的：要么草稿建在格之后（搬不到幕首），要么格
//! 落在草稿之前（档位读不到）。于是幕首段只做「不需要等」的事，接缝段专管「要等」的
//! 四件——两段之间的边界就是 `TurnRunner::begin`。
//!
//! **纯结构拆分契约**：[`assemble_turn`] 与 [`land_seam`] 的体内自 `turn_exec.rs` 的
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
    pub(crate) presentation_tree: symbio_core::node_build::PresentationTree,
    // —— 阶段① 的读数与构造 ——
    pub(crate) store: Arc<dyn Store<Event = Event>>,
    pub(crate) turn_no: u64,
    pub(crate) user_seq: u64,
    pub(crate) drive_seq: u64,
    pub(crate) collected: Arc<std::sync::Mutex<Vec<ChatNode>>>,
    pub(crate) bridge: Arc<UiBridge>,
    pub(crate) emitter: tokio::task::JoinHandle<()>,
    pub(crate) actor: ActorSpec,
    pub(crate) principal: String,
    pub(crate) to: String,
}

/// 接缝要的**全部**环境（幕首已产出、接缝才消费）。
///
/// 与 [`Assembled`] 分开成形而不是直接把 `&Assembled` 传下去：接缝只需要这十样，
/// 而 `Assembled` 是「①用过但②③④也要用」的全集——传全集等于让接缝顺手能读
/// 它不该读的东西（例如 `provider` / `parent`）。
pub(crate) struct Seam<'a> {
    pub(crate) session: &'a LoopSession,
    pub(crate) store: &'a dyn Store<Event = Event>,
    pub(crate) bridge: &'a UiBridge,
    pub(crate) turn_no: u64,
    pub(crate) user_seq: u64,
    pub(crate) drive_seq: u64,
    pub(crate) actor: &'a ActorSpec,
    pub(crate) consolidation: Option<&'a [ChatNode]>,
    pub(crate) opening_text: Option<&'a str>,
    pub(crate) opening_dialog: Option<&'a super::super::turn_runner::DialogFacts>,
}

/// **接缝**：轮首四行跑完、任何请求之前（`D-223` 订正 1 / 订正 2）。
///
/// 四件事都是「轮首那几行跑完才知道」的，因此都不在幕首段（[`assemble_turn`]）里：
///
/// | 动作 | 为什么要等那几行 | 落成什么 |
/// |---|---|---|
/// | 压缩入格 | `policy.compress` 是本幕的前置动作，它落的事实必须早于本轮派发格 | `session.consolidated` + 归档 |
/// | 定档 | 折自 `policy.reflex` 的出口格（`turn_head.reflex_answered`） | `turn.dispatch`（`tier` / `budget_ms` / `principal`） |
/// | 首响上线 | 判决行 `Escalate` 的那句话 | `chat.assistant.opening` + 一帧实时面 |
/// | （基线 prompt 与两次填） | 压缩改的就是消息列表 | 见 `turn_exec::drive_opened` 那一处 |
///
/// 前两件与第三件的**次序**是硬的：压缩早于派发（`consolidation_is_forwarded_before_the_turn_runs`
/// 钉着），首响早于基线 prompt 的渲染（渲染在调用方、紧接本函数之后）。
///
/// ## 为什么不在这里渲染 prompt
///
/// 渲染要往草稿里写（`TurnSession::render_base`），而草稿是阶段② 用 `begin` 建出来的
/// ——本函数跑在 `begin` 之后，但手里只有 `&LoopSession`。渲染因此留在调用方，紧随本
/// 函数；「压缩先落、prompt 后渲染」这条次序由调用点的行序保证，不需要把草稿传进来。
pub(crate) async fn land_seam(seam: Seam<'_>) -> Result<(), ActorError> {
    let Seam {
        session,
        store,
        bridge,
        turn_no,
        user_seq,
        drive_seq,
        actor,
        consolidation,
        opening_text,
        opening_dialog,
    } = seam;
    // ── 压缩入格 + 归档（S2.7-b2）─────────────────────────────────────────
    //
    // 必须在派发格**之前**：本轮的用户格已由编排层先落，先归档才能让「本轮之前的
    // 一切」被完整取代（否则快照与它后面的新事件之间会夹着被取代的旧事件，现行窗口
    // 不再自洽）。
    //
    // 归档移走前缀**不影响**续写锚：轮的「已开未收束」判据读完整史（D-085），
    // 等待轮被压进归档段后仍然成立——而且续写锚此刻已经由编排层按 seq 直给。
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
    // ── 定档：折自快路那一行的出口格（`D-223` 订正 1）─────────────────────
    //
    // 读**视图**而不是执行侧的副本（`reflex.is_some()`）：视图与那一行的判定同源，
    // 而「命中集为空、行自己关账」那一支（`result = miss`）只有视图分得出来。
    let facts_now = store.range(Seq::new(0));
    let now = super::super::loop_rows::wake_now_ms(&facts_now, drive_seq);
    let tier = super::super::loop_rows::turn_tier(&facts_now, turn_no, now);
    super::super::chat_loop::land_dispatch(store, turn_no, user_seq, drive_seq, tier, actor)
        .map_err(ActorError::Append)?;
    // ── 首响落格 + 上线 ──────────────────────────────────────────────────
    super::super::chat_loop::land_opening(
        store,
        bridge,
        turn_no,
        user_seq,
        &actor.principal,
        opening_text,
        opening_dialog,
    )
    .map_err(ActorError::Append)?;
    Ok(())
}

/// 一幕的幕首（阶段①）：装配，产 [`Assembled`]。
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
        supplements,
        report,
        supplemental_no,
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

    // 档位与两格事实**不在这里落**（`D-223` 订正 1 / 订正 2）：档位折自
    // `policy.reflex` 的出口格，开场白是判决行 `Escalate` 的首响——两者都要等轮首
    // 那几行跑完。落点是接缝（[`land_seam`]），由阶段② 在 `begin` 之后调。
    //
    // 本段（幕首）只做「不需要等那几行」的事：取事实源、补落寄存变更、建收集器与
    // 流式桥、定主体与来源主体。
    //
    // 身份进事件（plan/11 批 1 ①）：本轮以**会话选定的那个 agent** 为主体，规格从
    // **授权表行**取（`request_actor_spec`）——能力集与模式不再是 `trivial` 的空表
    // 平凡值，它们与写侧闸判的是同一张表（判的对象 = 写的对象 = 表授予的对象）。
    // 未选智能体 ⇒ `agent:main`（S08 §4 平凡值，与接线前逐字一致）。
    //
    // 主体**不吃档位**（`D-223` 订正 2）：它要在幕首就建（`D-222` 订正 3），而档位要
    // 等轮首四行跑完——两者互相派生就成环。规格里的 `budget_ms` 是主体的**声明档**，
    // 本幕的预算归接缝上那一格 `turn.dispatch`。
    //
    // `principal` 已在落阶段事实时取过（同一取值点 `request_principal`）：
    // `actor` 随 `input` 被移动进运行器，轮末的记忆与学习写方（`record_learning`）
    // 仍要以它为主体——先取字符串，不依赖 `actor` 的存活。
    // 作用域（[]）：`agent_run` 派生的子会话挂父主体，顶层会话恒 `Root`。
    // 与身份同批取——两者都经 `ctx` / 元数据解析，分开取会让「主体是 A、作用域按 B 算」。
    let scope = session.scope();
    let actor = super::super::chat_loop::request_actor_spec(ctx.as_ref(), scope);
    // 来源主体（D-067）：`commitment.*` 的 `to`。在此刻取而不是轮末——`ctx` 会被
    // 移进工具分发口（`SessionDispatchPort`），轮末已经拿不到它。
    let to = super::super::chat_loop::request_source_principal(ctx.as_ref());

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
