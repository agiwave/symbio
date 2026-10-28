//! 一幕的**直线**：幕首建草稿 → 轮首那几行 → 接缝 + 内环 → 收好产出
//! （`D-222` §二 裁决 1 / 订正 3）。
//!
//! ## 为什么是直线，不再是第四台 host
//!
//! 此前这一幕是一台 host：三条行（`prepare-turn` / `announce-turn` /
//! `dispatch-turn`）挂在 `turn` 端口上，由第四段驱动循环 `drive_stage` 按 `prio`
//! 选债、还债。那台 host 存在的唯一理由是 `turn` 是端口——而它不是（`D-222` §一：
//! 应答载荷为空、适配器就是内环自己、没有任何等待方）。端口一撤，三条行就没有
//! 「什么算欠」可写，循环也就失去了选债对象。
//!
//! 于是这一幕回到它本来的形状：一段**直线**。开一幕是交换点，不是环（`D-222`
//! 裁决 1）——「这一幕跑完了没有」不再靠留一格的形状去猜，它就是本函数的返回值。
//!
//! ## 与环的边界
//!
//! 幕**内**那一段仍然是环：`turn_exec::drive_steps` 是唯一的不动点，模型 / 工具两个
//! 端口的行由它选。本模块只做四件事，一件都不是调度：
//!
//! | 段 | 做什么 | 产出 |
//! |---|---|---|
//! | 幕首 | 环前纯读（提示词 / 工具 / 开销）+ 定根 id + 装配 + `begin`（`D-222` 订正 3） | 草稿；或「本幕已收束」 |
//! | 轮首 | `land_head`：判决 / 压缩 / 召回 / 快路 + 答话收束 + 水位 + 骨架 + 请求视图 | 注入段；或「本幕已收束」 |
//! | 接缝 + 内环 | `drive_opened`：压缩入格 / 定档 / 首响 → 渲染基线 → 填前缀与快路 → 内环 | 产出 / 中止 / 失败 |
//! | 收好产出 | 镜像并入 → 中止复查 → 定格 → 落库 → 封根 | 事实（收束格） |
//!
//! 四段的次序与搬到环上之前**逐字不变**：`finalize_turn_root` 必须晚于定格（Turn 是
//! 组合节点，终态跟随子树），而落库是收束的写投影。变的只是「谁来调」——此前是
//! 选债，现在是直线。
//!
//! ## 崩溃面没有变宽
//!
//! 撤掉 `turn` 端口同时撤掉了三个「崩在宣布与派出之间」的可判后果（派发行选中它、
//! `kill_dangling` 判死、下一幕不会以为已经派过）。那三个后果本来就是**空转**：
//! `awaited = false`，等这笔应答的 future 随进程一起没了，重试没有对象；恢复从来
//! 是**新开一幕**（新 `drive_seq`），不是接上旧的那一笔。删掉之后崩溃面没有变宽：
//! 一幕要么落下了收束格，要么没有——后者由 `next_stage_round` 读事实判定。
//!
//! ## 输入不进 `TurnState`
//!
//! 环前纯读的产物（[`TurnReads`]）的寿命就是这一幕（装配 → 消费），而 `TurnState`
//! 是整轮的容器。把它抬进去等于让「一次只用一次的中间值」拥有轮级寿命：本函数因此
//! 把它放在**局部变量**里，由四段的调用次序保证它只活在需要它的那一段。
//!
//! ## 一处行为变化（`D-222` 订正 3 明文认下的后果）
//!
//! 中止守卫随环前移到 `begin`：被中止的幕不再跑判决 / 压缩 / 召回。此前那几行排在
//! 开轮之前，中止只在「第一个请求发出之前」才被发现；现在草稿先建，1c 的中止检查
//! 一命中就收束，轮首那几行一行都不跑。

use std::sync::Arc;

use symbio_core::plugin_info;
use symbio_core::{Event, PluginInvokeRequest, Seq, Store};

use super::chat_loop::inputs::{collect_reads, land_head, TurnReads};
use super::chat_loop::state::{ChatOrchestrator, SessionContext, TurnExit, TurnState};
use super::turn_exec::{
    classify_turn, drive_opened, open_turn, persist_turn, ActorError, AfterHead, Classified,
    ExecTurn, ExecTurnResult, Opened, Running, TurnExecExit,
};

/// 一幕要的**全部**环境（一次借出，四段用）。
///
/// 八样东西一起构成「这一幕现在长什么样」：编排器（模型 / 树）、请求句柄（能力口）、
/// 会话上下文（消息镜像 + 句柄）、本轮状态（幕锚与寄存位）、请求级配置快照。
/// 分开传时同一个 `u64` / `&str` 很容易写反，而写反的后果是**静默**的——结构体
/// 字面量把「谁是谁」写在字段名上。
pub(crate) struct Stage<'a> {
    pub(crate) orchestrator: &'a ChatOrchestrator,
    /// 编排器的共享句柄：批次边界的能力口（[`super::chat_loop::report_port`]）要它。
    pub(crate) orchestrator_arc: &'a Arc<ChatOrchestrator>,
    pub(crate) ctx: &'a Arc<dyn PluginInvokeRequest>,
    pub(crate) sink: &'a symbio_core::ExecEventSink,
    pub(crate) context: &'a mut SessionContext,
    pub(crate) turn: &'a mut TurnState,
    /// 请求级配置快照（环前纯读要它：显式系统提示词与 `auto_compress` 都在里面）。
    pub(crate) req: &'a super::chat_loop::state::TurnRequest,
    /// 本轮用户发言（工具轮末的学习与判决 Q1 用它；事实与 prompt 都从用户格出）。
    pub(crate) user_text: &'a str,
}

/// 跑一幕。`Ok(None)` = 这一幕跑完了；`Ok(Some(exit))` = 没跑完（中止 / 失败）。
///
/// 「跑完」与「没跑完」的分野不含流向（`D-222` 裁决 3）：幕跑完之后是收工还是再开
/// 一幕，读的是事实（`turn_head.closure` + 本幕轮次），不是这个返回值。
pub(crate) async fn run_stage(stage: Stage<'_>) -> Result<Option<TurnExit>, ActorError> {
    let Stage {
        orchestrator,
        orchestrator_arc,
        ctx,
        sink,
        context,
        turn,
        req,
        user_text,
    } = stage;
    let Some(store) = context.session.facts() else {
        return Ok(Some(TurnExit::Failed(
            symbio_core::PluginError::InternalError(
                "执行一幕需要持久会话（临时会话无事实源）".into(),
            ),
        )));
    };

    // ── 一：幕首 ────────────────────────────────────────────────────────
    //
    // 环前纯读（①②）+ 定根 + 装配 + 开轮。这里**一格**「要等轮首那几行」的事实都
    // 不落：草稿先建（`D-222` 订正 3），判决 / 压缩 / 召回随后在已经存在的草稿上跑。
    let reads: TurnReads = collect_reads(ctx, req, turn).await;
    // 一幕一个根：续写幕复用被续写轮的根（骨架已经发过，再发会把已完成的卡打回
    // 运行态），新幕现分配。「定 id」在幕首（`begin` 的输入），「把骨架发给流式桥」
    // 仍在轮首那几行的 ④（**压缩之后**：压不动时不该留半截 Turn 根）。
    let (root_id, fresh_root) = match turn.resume_root.take() {
        Some(root) => (root, false),
        None => (symbio_core::llm_short_id(), true),
    };
    // 会话窄句柄：草稿与装配束都借它，而 `context` 在轮首段还要可变借用——先克隆。
    let session = context.session.clone();
    let supplements: super::turn_exec::SupplementFn = {
        // 轮内注入（缺口 3）：把本会话的收件箱抽干口交给 v2 的工具循环，
        // 让「用户中途插话」**折进本轮**。抽干住在 session（队列与配置都在那边），
        // 驱动器经端口取——异步，因为动作面是异步的。
        let session = session.clone();
        Arc::new(move || {
            let session = session.clone();
            let fut: std::pin::Pin<
                Box<
                    dyn std::future::Future<Output = Option<symbio_core::DrainedSupplement>> + Send,
                >,
            > = Box::pin(async move {
                let supplements: Option<symbio_core::DrainedSupplement> = session
                    .run_op(symbio_core::run_ops::TAKE_SUPPLEMENTS, ())
                    .await
                    .unwrap_or_else(|e| {
                        symbio_core::plugin_warn!(
                            "session",
                            "[Loop] 取出用户补充失败（本轮继续）: {e}"
                        );
                        None
                    });
                supplements
            });
            fut
        }) as super::turn_exec::SupplementFn
    };
    // 批次边界汇报（缺口 4）的能力口：**只产措辞**——判定与落格都不在这里
    // （见 `report_port` 的文档与 D-163）。`None` = compose 产线关着，批次
    // 边界就不装派发点（关掉的开关不该在日志里每批边界留一串「有账没人还」）。
    let report = orchestrator.compose_enabled.then(|| {
        super::chat_loop::report_port(
            orchestrator_arc.clone(),
            ctx.clone(),
            context.messages.clone(),
            turn.now_ms,
        )
    });
    let assembled = match super::turn_exec::assemble_turn(ExecTurn {
        session: session.as_ref(),
        provider: orchestrator.provider.clone(),
        parent: orchestrator.parent.clone(),
        ctx: ctx.clone(),
        system_prompt: &reads.system_prompt,
        abort: turn.abort.clone(),
        root_id: &root_id,
        sink,
        user_text,
        turn_no: turn.turn_no,
        user_seq: turn.user_seq,
        drive_seq: turn.drive_seq,
        window_turns: orchestrator.session.context_window(),
        tools: &reads.tools,
        supplements: Some(supplements),
        report,
        supplemental_no: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        presentation_tree: orchestrator.presentation_tree,
    })
    .await
    {
        Ok(assembled) => assembled,
        Err(exit) => {
            turn.facts = store.range(Seq::new(0));
            return Ok(Some(stage_exit(exit)));
        }
    };
    let opened = match open_turn(&assembled).await {
        Ok(opened) => opened,
        Err(exit) => {
            turn.facts = store.range(Seq::new(0));
            return Ok(Some(stage_exit(exit)));
        }
    };

    // ── 二：轮首那几行 → 接缝 → 内环 ───────────────────────────────────
    //
    // 归类与落格**在臂内**收掉：接缝借的是轮首那几行取走的寄存物（请求视图段 /
    // 压缩快照 / 开场白 / 快路正文）与 `turn` 上的两处寄存位，它们活到内环跑完为止。
    // 出臂之前 [`Running`] 必须被消费（[`classify_and_persist`]）——否则那些借用会
    // 活到第三段 `settle` 的 `&mut turn` 上，两段借同一片状态，借用检查器（正确地）
    // 拒绝。
    let res = match opened {
        // 开轮就收束（首请求前中止）：轮首那几行**不再跑**（`D-222` 订正 3）。
        Opened::Done(outcome) => {
            let running =
                assembled.into_running(outcome, turn.recall_view.as_ref(), &turn.skill_route);
            classify_and_persist(running).await
        }
        Opened::Run(run) => {
            let request_sections = match land_head(
                orchestrator,
                ctx,
                context,
                sink,
                turn,
                &reads.tools,
                &root_id,
                fresh_root,
            )
            .await
            {
                Ok(sections) => sections,
                // 定稿答话轮：本幕在轮首就地收束（`land_answered_turn` 已落收束格），
                // 没有往返，也没有产出。
                Err(TurnExit::Completed) => {
                    // 草稿必须先丢：它里面的注入闭包也持着一份桥的强引用，不先丢，
                    // 发射任务等不到队列关闭（`Assembled::discard` 的文档）。
                    drop(run);
                    assembled.discard().await;
                    land_inputs(&store, ctx, turn, "closed")?;
                    // 取走即消费：这一幕没有新根，续写幕不该再开一支。
                    turn.retry_of.take();
                    turn.facts = store.range(Seq::new(0));
                    return Ok(None);
                }
                // 轮首读失败（会话端口的一次读失败）：端口面上没有一笔账可关，交回
                // 编排层走 `finish_turn(Failed)` 的既有路径（`ActorError::Read` 的文档）。
                Err(TurnExit::Failed(e)) => return Err(ActorError::Read(e)),
                // 其余出口不该从轮首段冒出来：它们各有自己的入口（中止在开轮 1c、
                // 边界中止在闸门、恢复完成在循环之前）。硬造一格只会把「怎么到这里」
                // 这条信息抹掉。
                Err(other) => {
                    return Err(ActorError::Read(symbio_core::PluginError::InternalError(
                        format!("轮首段返回了不属于它的出口：{other:?}"),
                    )));
                }
            };
            land_inputs(&store, ctx, turn, "ready")?;
            land_retry_branch(&store, ctx, turn, &root_id)?;
            // 轮首那几行的寄存位**在这里取走**：压缩快照只该落一次（S2.7-b2），
            // 开场白与快路正文也只属于这一幕。
            let consolidation = context.consolidated.take();
            let opening_text = turn.opening_text.take();
            let opening_dialog = turn.opening_dialog.take();
            let after = AfterHead {
                request_sections: &request_sections,
                consolidation: consolidation.as_deref(),
                opening_text: opening_text.as_deref(),
                opening_dialog: opening_dialog.as_ref(),
                reflex: turn.head_reflex.as_ref(),
                recalled: turn.recall_view.as_ref(),
                skill_obs: &turn.skill_route,
            };
            match drive_opened(assembled, run, after).await {
                Ok(running) => classify_and_persist(running).await,
                Err(exit) => Err(exit),
            }
        }
    };
    let res = match res {
        Ok(v) => v,
        Err(exit) => {
            turn.facts = store.range(Seq::new(0));
            return Ok(Some(stage_exit(exit)));
        }
    };

    // ── 三：收好产出 ────────────────────────────────────────────────────
    //
    // 归类（中止 / 授权拒 / 正常）已在上一段收掉——`res` 是**跑成的那一轮**才有的
    // 结果；没跑成（中止 / 失败）已在上面按原义交回本幕的出口。这里只剩
    // 「镜像并入 → 定格 → 落库 → 封根」这一串（[`settle`]）。
    let outcome = settle(orchestrator, sink, context, turn, &root_id, res).await?;
    // 幕间判据与这一轮的收尾都读最新切片：本幕落的格必须在 `turn.facts` 里看得见。
    turn.facts = store.range(Seq::new(0));
    Ok(outcome)
}

/// 本轮草稿的**归类与落格**：提前了结（中止 / 授权拒）原样交回，正常收束落派生事实。
///
/// 单列一个函数只为一件事：它必须与 `Running` 的借用**同寿**——调用它的地方（幕的
/// 第二段）同时持有接缝借来的寄存物，而第三段的 [`settle`] 要可变借回本轮状态。
async fn classify_and_persist(running: Running<'_>) -> Result<ExecTurnResult, TurnExecExit> {
    match classify_turn(running).await? {
        Classified::Finished(v) => v,
        Classified::Settled(settled) => persist_turn(settled),
    }
}

/// 阶段函数的退出类型 → 本幕的出口。
///
/// 两者只差「这一轮还往不往下走」的语义：阶段函数不判流向，原样交回主循环
/// （`D-222` 裁决 3）。
fn stage_exit(exit: TurnExecExit) -> TurnExit {
    match exit {
        TurnExecExit::Aborted(partial) => TurnExit::Aborted(partial),
        TurnExecExit::Failed(e) => TurnExit::Failed(e),
    }
}

/// 把「备输入」的出口落成阶段事实（`turn.stage{stage:"inputs"}`）。
///
/// `result` 二值：`ready` = 这一幕要跑（请求视图已备好）；`closed` = 定稿答话轮在
/// 轮首就收束了。它给事实面留下「这一幕备到哪一步」的唯一记号——`Ok(None)` 那个
/// 返回值不落日志，只落这一格。
fn land_inputs(
    store: &Arc<dyn Store<Event = Event>>,
    ctx: &Arc<dyn PluginInvokeRequest>,
    turn: &TurnState,
    result: &str,
) -> Result<(), ActorError> {
    super::stage_events::land(
        store.as_ref(),
        turn.turn_no,
        turn.user_seq,
        &symbio_core::request_principal(ctx.as_ref()),
        &[super::stage_events::StageFact::inputs(
            turn.request_round,
            turn.drive_seq,
            result,
        )],
    )
    .map_err(ActorError::Append)
}

/// 重试开出的**新支在这里登记**（[D-173](../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
///
/// 为什么是这一格而不是唤醒那一处：`turn.branch` 的 `into` 是**新支的 Turn 根
/// id**，而那个 id 到本幕的根定下才存在（一幕一个根，`run_stage` 的幕首是它唯一的
/// 创建点）。`retry_of` 由 `RetrySubtree` 寄存、`take()` 即消费，幕间续写的第二幕
/// 因此不会再开一支。
///
/// 旧支不删：它降成「同一问题的另一个候选」，藏与不藏由选中态说（读侧一处
/// `branch_hide_unselected`，只挡进模型的那条路）。
fn land_retry_branch(
    store: &Arc<dyn Store<Event = Event>>,
    ctx: &Arc<dyn PluginInvokeRequest>,
    turn: &mut TurnState,
    root_id: &str,
) -> Result<(), ActorError> {
    let Some(of) = turn.retry_of.take() else {
        return Ok(());
    };
    let seq = store
        .append(
            symbio_core::Event::pending(
                format!("branch-{root_id}"),
                symbio_plugin_api::events::EVENT_TURN_BRANCH,
                symbio_core::Entity::Turn,
                symbio_core::Verb::Asserted,
                turn.turn_no,
                symbio_core::request_principal(ctx.as_ref()),
            )
            .with_payload(serde_json::json!({ "of": of, "into": root_id }))
            .with_produced_by(turn.user_seq),
        )
        .map_err(ActorError::Append)?;
    plugin_info!(
        "session",
        "[Branch] 新支 {} 挂到 {} 之下（seq={}）",
        root_id,
        of,
        seq.value()
    );
    // 收尾时按这一格补发组内的标注帧（见 `TurnState::branch_root`）。
    turn.branch_root = Some(root_id.to_string());
    Ok(())
}

/// 派出去那一幕的**产出处理**。
///
/// 四件事的次序与本函数成形之前逐字一致：
///
/// 1. 镜像并入：v2 原生产生的消息（工具轮：助手中途正文 + ToolCall + 工具结果）并进
///    落库权威镜像——`persist_messages` 只写这份的尾部增量，不并入就只看得见、
///    不落库（重启即丢）。按 id 并入而不是整段 extend：续写幕的调用卡已经在历史里，
///    新结果还要替换旧结果子节点（见 `wake::merge_runner_messages`）。
/// 2. 中止复查：请求结束后才置位的 abort 同样向上冒泡。本轮已完整产出（镜像已并入、
///    随落库写入），没有半截要补 ⇒ `partial` 空。
/// 3. 定格 + 落库：`settle_reasoning` 之后 `persist_turn` 落增量，并把
///    [`TurnState::last_saved`] 推到最新（`finish_turn` 的落库因此是 no-op）。
/// 4. 封根：子树收敛之后全流程**唯一**一处发 Turn 的终态帧。
async fn settle(
    orchestrator: &ChatOrchestrator,
    sink: &symbio_core::ExecEventSink,
    context: &mut SessionContext,
    turn: &mut TurnState,
    root_id: &str,
    res: ExecTurnResult,
) -> Result<Option<TurnExit>, ActorError> {
    let session = context.session.clone();
    super::chat_loop::merge_runner_messages(&session, context, turn, res.messages)
        .await
        .map_err(ActorError::Read)?;
    if turn.abort.is_aborted() {
        return Ok(Some(TurnExit::Aborted(Vec::new())));
    }
    let finish =
        super::chat_loop::settle_reasoning(orchestrator, context, sink, root_id, res.output).await;
    super::chat_loop::persist_turn(sink, context, turn, finish).await;
    super::chat_loop::finalize_turn_root(sink, context, root_id).await;
    Ok(None)
}

#[cfg(test)]
#[path = "stage.test.rs"]
mod tests;
