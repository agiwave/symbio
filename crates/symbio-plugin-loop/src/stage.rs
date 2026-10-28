//! 一幕的**直线**：备输入 → 跑一轮 → 收好产出（`D-222` §二 裁决 1）。
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
//! 端口的行由它选。本模块只做三件事，一件都不是调度：
//!
//! | 段 | 做什么 | 产出 |
//! |---|---|---|
//! | 备输入 | `prepare_turn_inputs`（提示词 / 工具 / 请求视图的唯一收集点，末尾起轮首那台 host） | 本幕的 [`TurnInputs`]；或「本幕已收束」 |
//! | 跑一轮 | `execute_turn`（内环） | 产出 / 中止 / 失败 |
//! | 收好产出 | 镜像并入 → 中止复查 → 定格 → 落库 → 封根 | 事实（收束格） |
//!
//! 三段的次序与搬到环上之前**逐字不变**：`finalize_turn_root` 必须晚于定格（Turn 是
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
//! [`TurnInputs`] 的寿命就是这一幕（产出 → 消费 → 收尾读一次 `root_id`），而
//! `TurnState` 是整轮的容器。把它抬进去等于让「一次只用一次的中间值」拥有轮级
//! 寿命：本函数因此把它放在**局部变量**里，由三步的调用次序保证它只活在需要它的
//! 那一段。

use std::sync::Arc;

use symbio_core::plugin_info;
use symbio_core::{Event, PluginInvokeRequest, Seq, Store};

use super::chat_loop::inputs::TurnInputs;
use super::chat_loop::state::{ChatOrchestrator, SessionContext, TurnExit, TurnState};
use super::turn_exec::{ActorError, ExecTurnResult, TurnExecExit};

/// 一幕要的**全部**环境（一次借出，三段用）。
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
    /// 请求级配置快照（准备段要它：显式系统提示词与 `auto_compress` 都在里面）。
    pub(crate) req: &'a super::chat_loop::state::TurnRequest,
    /// 本轮用户发言（`execute_turn` 的技能命中判定与轮末学习用它）。
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

    // ── 一：备输入 ──────────────────────────────────────────────────────
    let Some(inputs) = prepare(orchestrator, ctx, sink, context, turn, req, &store).await? else {
        // 定稿答话轮：准备段走完时本幕已经收束，这一幕**没有**往返。
        turn.facts = store.range(Seq::new(0));
        return Ok(None);
    };

    // ── 二：跑一轮 ──────────────────────────────────────────────────────
    // 压缩的寄存位**在这一刻取走**（S2.7-b2）：`take` 而不是 `clone`——同一份快照
    // 只该落一次。位置也必须在准备段之后：压缩是准备段跑的（`policy.compress` 由
    // 轮首那台 host 选中），在它之前取会取到空值，压缩事实于是静默消失
    // （e2e t8 / t11 / t45 抓到的正是这一条）。
    let consolidation = context.consolidated.take();
    let outcome = match run_turn(TurnParts {
        orchestrator,
        orchestrator_arc,
        ctx,
        sink,
        context,
        turn,
        inputs: &inputs,
        user_text,
        consolidation: consolidation.as_deref(),
    })
    .await
    {
        // ── 三：收好产出 ────────────────────────────────────────────────
        Ok(res) => settle(orchestrator, sink, context, turn, &inputs.root_id, res).await?,
        // 没跑成：这一笔的结局就是本幕的中断。中止 ≠ 失败，两条各自保留原义，
        // 只是汇成一个出口（`D-123` 决策 5：执行体不交回产出）。
        Err(TurnExecExit::Aborted(partial)) => Some(TurnExit::Aborted(partial)),
        Err(TurnExecExit::Failed(e)) => Some(TurnExit::Failed(e)),
    };
    // 幕间判据与这一轮的收尾都读最新切片：本幕落的格必须在 `turn.facts` 里看得见。
    turn.facts = store.range(Seq::new(0));
    Ok(outcome)
}

/// 备输入：`prepare_turn_inputs` 的**唯一调用点**。
///
/// 它是提示词 / 工具 / 请求视图的唯一收集点（收口 ②），末尾起轮首那台 host
/// （判决 / 压缩 / 召回 / 快路）。`Ok(None)` = 本幕在准备段就收束了（定稿答话轮：
/// 收束格由 `land_answered_turn` 落），那一档没有输入，也不该有往返。
async fn prepare(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn PluginInvokeRequest>,
    sink: &symbio_core::ExecEventSink,
    context: &mut SessionContext,
    turn: &mut TurnState,
    req: &super::chat_loop::state::TurnRequest,
    store: &Arc<dyn Store<Event = Event>>,
) -> Result<Option<TurnInputs>, ActorError> {
    let prepared =
        super::chat_loop::inputs::prepare_turn_inputs(orchestrator, ctx, context, sink, turn, req)
            .await;
    let round = turn.request_round;
    let (result, inputs) = match prepared {
        Ok(inputs) => ("ready", Some(inputs)),
        Err(TurnExit::Completed) => ("closed", None),
        // 轮首读失败（会话端口的一次读失败）：端口面上没有一笔账可关，交回编排层
        // 走 `finish_turn(Failed)` 的既有路径（`ActorError::Read` 的文档）。
        Err(TurnExit::Failed(e)) => return Err(ActorError::Read(e)),
        // 其余出口不该从准备段冒出来：它们各有自己的入口（中止在轮内 1c、边界中止
        // 在闸门、恢复完成在循环之前）。硬造一格只会把「怎么到这里」这条信息抹掉。
        Err(other) => {
            return Err(ActorError::Read(symbio_core::PluginError::InternalError(
                format!("准备段返回了不属于它的出口：{other:?}"),
            )));
        }
    };
    super::stage_events::land(
        &**store,
        turn.turn_no,
        turn.user_seq,
        &symbio_core::request_principal(ctx.as_ref()),
        &[super::stage_events::StageFact::inputs(
            round,
            turn.drive_seq,
            result,
        )],
    )
    .map_err(ActorError::Append)?;
    // 重试开出的**新支在这里登记**（[D-173](../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
    //
    // 为什么是这一格而不是唤醒那一处：`turn.branch` 的 `into` 是**新支的 Turn 根
    // id**，而那个 id 到本幕的输入备好才存在（一幕一个根，`prepare_turn_inputs`
    // 是它唯一的创建点）。`retry_of` 由 `RetrySubtree` 寄存、`take()` 即消费，
    // 幕间续写的第二幕因此不会再开一支。
    //
    // 旧支不删：它降成「同一问题的另一个候选」，藏与不藏由选中态说（读侧一处
    // `branch_hide_unselected`，只挡进模型的那条路）。
    if let (Some(of), Some(inputs)) = (turn.retry_of.take(), inputs.as_ref()) {
        let seq = store
            .append(
                symbio_core::Event::pending(
                    format!("branch-{}", inputs.root_id),
                    symbio_plugin_api::events::EVENT_TURN_BRANCH,
                    symbio_core::Entity::Turn,
                    symbio_core::Verb::Asserted,
                    turn.turn_no,
                    symbio_core::request_principal(ctx.as_ref()),
                )
                .with_payload(serde_json::json!({ "of": of, "into": inputs.root_id }))
                .with_produced_by(turn.user_seq),
            )
            .map_err(ActorError::Append)?;
        plugin_info!(
            "session",
            "[Branch] 新支 {} 挂到 {} 之下（seq={}）",
            inputs.root_id,
            of,
            seq.value()
        );
        // 收尾时按这一格补发组内的标注帧（见 `TurnState::branch_root`）。
        turn.branch_root = Some(inputs.root_id.clone());
    }
    Ok(inputs)
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

/// 一次 `turn` 本体要借的**八样**（[`Stage`] 去掉配置快照之后的投影）。
///
/// 单独成形是为了让 [`run_stage`] 能把它们整体交给 [`run_turn`]，而把 `outcome`
/// 那个借位留在手上——拆成值传递才成立。
struct TurnParts<'a> {
    orchestrator: &'a ChatOrchestrator,
    orchestrator_arc: &'a Arc<ChatOrchestrator>,
    ctx: &'a Arc<dyn PluginInvokeRequest>,
    sink: &'a symbio_core::ExecEventSink,
    context: &'a mut SessionContext,
    turn: &'a mut TurnState,
    inputs: &'a TurnInputs,
    user_text: &'a str,
    consolidation: Option<&'a [symbio_core::chat_node::ChatNode]>,
}

/// 一次 `turn` 的本体：把借用面摊成 [`super::turn_exec::ExecTurn`]。
async fn run_turn(parts: TurnParts<'_>) -> Result<ExecTurnResult, TurnExecExit> {
    let TurnParts {
        orchestrator,
        orchestrator_arc,
        ctx,
        sink,
        context,
        turn,
        inputs,
        user_text,
        consolidation,
    } = parts;
    super::turn_exec::execute_turn(super::turn_exec::ExecTurn {
        session: &context.session,
        provider: orchestrator.provider.clone(),
        parent: orchestrator.parent.clone(),
        ctx: ctx.clone(),
        system_prompt: &inputs.system_prompt,
        abort: turn.abort.clone(),
        root_id: &inputs.root_id,
        sink,
        user_text,
        turn_no: turn.turn_no,
        user_seq: turn.user_seq,
        drive_seq: turn.drive_seq,
        window_turns: orchestrator.session.context_window(),
        tools: &inputs.tools,
        // 记忆与学习的入参：轮次事实由 v2 原生记账，这两项**不在其中**
        // （它们的写方是 `derived_facts::record_learning`，见其文档）。
        recalled: turn.recall_view.as_ref(),
        skill_obs: &turn.skill_route,
        // 快路正文（S11 执行半边）：`Some` ⇒ 轮首 host 已选中 `policy.reflex`
        // 并落了出口格，本轮不调模型（D-163）。
        reflex: turn.head_reflex.as_ref(),
        // 请求级前缀 = 请求级注入段的三段（见 `chat_loop::request_view_prefix`）。
        prefix: super::chat_loop::request_view_prefix(&inputs.request_sections),
        // 压缩的寄存位（S2.7-b2）：`None` = 本轮没压过（常态）。
        consolidation,
        // 轮内注入（缺口 3）：把本会话的收件箱抽干口交给 v2 的工具循环，
        // 让「用户中途插话」**折进本轮**。抽干住在 session（队列与配置都在那边），
        // 驱动器经端口取——异步，因为动作面是异步的。
        supplements: Some({
            let session = context.session.clone();
            Arc::new(move || {
                let session = session.clone();
                let fut: std::pin::Pin<
                    Box<
                        dyn std::future::Future<Output = Option<symbio_core::DrainedSupplement>>
                            + Send,
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
        }),
        supplemental_no: Arc::new(std::sync::atomic::AtomicU64::new(0)),
        // 批次边界汇报（缺口 4）的能力口：**只产措辞**——判定与落格都不在这里
        // （见 `report_port` 的文档与 D-163）。`None` = compose 产线关着，批次
        // 边界就不装派发点（关掉的开关不该在日志里每批边界留一串「有账没人还」）。
        report: orchestrator.compose_enabled.then(|| {
            super::chat_loop::report_port(
                orchestrator_arc.clone(),
                ctx.clone(),
                context.messages.clone(),
                turn.now_ms,
            )
        }),
        // 对话面开场白：轮首判决执行体的 `Escalate` 首响。落格在 `turn_exec`
        // （唯一持写者令牌处），这里只递「说了什么 / 节点身份」。
        opening_text: turn.opening_text.as_deref(),
        opening_dialog: turn.opening_dialog.take(),
        presentation_tree: orchestrator.presentation_tree,
    })
    .await
}

#[cfg(test)]
#[path = "stage.test.rs"]
mod tests;
