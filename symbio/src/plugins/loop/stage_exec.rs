//! **外环 host** 的执行体与派发点（DE-015 的 D-168 落地）。
//!
//! ## 这一台 host 管的是哪两笔债
//!
//! 一个幕里「这一次 `turn` 往返还没宣布 / 还没派出」那两笔：
//! `announce-turn`（落 `request(port=turn)`）与 `dispatch-turn`（落 `sent`，跑出一幕）。
//! 两笔的形状与模型端口**逐字同源**（D-166）：宣布与派出是两个环步，两步之间唯一的
//! 交接件是 [`StageStep::call`]。
//!
//! 状态从前是 `chat_loop` 里的一段直线代码：自己建 `Call`、自己 `port::announce`、
//! 自己 `port::dispatch`。直线代码没有「这一笔该谁跑」这句话——崩在两步之间，那笔
//! `request` 留在日志里没人认领，等着它的 future 已经跟着进程没了。搬到环上之后
//! 它至少有三个可判的后果：派发行会选中它、[`super::port::kill_dangling`] 会判死它、
//! 下一幕不会因为一条没人接的请求而以为「已经派过了」。
//!
//! ## 三个环步（D-168 的接回顺序）
//!
//! | 环步 | 行 | 交接件 |
//! |---|---|---|
//! | 准备 | `prepare-turn` | 产出本幕的 [`TurnInputs`]，放进 [`StageStep::inputs`] |
//! | 宣布 | `announce-turn` | 产出那一笔 `Call`，放进 [`StageStep::call`] |
//! | 派出 | `dispatch-turn` | 取走前两样，产出幕的结局，放进 [`StageStep::outcome`] |
//!
//! ## host 是借用面
//!
//! [`StageStep`] 借走编排器 + 会话上下文 + 本轮状态 + **本幕已备好的输入**
//! （`prepare_turn_inputs` 的产物）：最后这一样是它与轮内那台的根本差别——
//! `execute_turn` 的入参里有六样来自它，而轮内那张草稿一件都拿不到。
//!
//! ## 派发点不做写入封闭（D-168 的边界）
//!
//! 这一行动作的窗口里嵌着一台**完整的 host**（轮内），它的每一行在自己的派发点已经
//! 按名单对过账。在这一层再对一次，名单只能把那张表整个抄过来（两处 owner）——
//! 名单的 owner 在行上，不在调用链上。所以这里只做**零格告警**：动作一格没写
//! 说明它没干活，那是这一层能判的那件事。

use std::sync::Arc;

use crate::symbio_core::{Event, PluginInvokeRequest, Seq, Store};

use super::chat_loop::inputs::TurnInputs;
use super::chat_loop::state::{ChatOrchestrator, SessionContext, TurnState};
use super::port::{Call, DispatchError};
use super::turn_exec::{ActorError, ExecTurnResult, StepFuture, TurnExecExit};

/// 一次外环动作需要的**全部**环境。
///
/// 两个环步之间的交接件是 [`StageStep::call`]：宣布那一步放进去，派出那一步取走。
/// [`StageStep::outcome`] 是反方向的交接件——派出跑完的结果（含失败）放在这里，
/// 由环外的编排层消费：执行体不交回产出（D-123 决策 5），但它**产出**的东西必须
/// 留一个不靠返回值的去处。
pub(crate) struct StageStep<'a> {
    pub(crate) orchestrator: &'a ChatOrchestrator,
    /// 编排器的共享句柄：批次边界的能力口（[`super::chat_loop::report_port`]）要它。
    pub(crate) orchestrator_arc: &'a Arc<ChatOrchestrator>,
    pub(crate) ctx: &'a Arc<dyn PluginInvokeRequest>,
    pub(crate) store: &'a Arc<dyn Store<Event = Event>>,
    pub(crate) sink: &'a crate::symbio_core::ExecEventSink,
    pub(crate) context: &'a mut SessionContext,
    pub(crate) turn: &'a mut TurnState,
    /// 请求级配置快照（`prepare_turn_inputs` 的入参之一）。
    pub(crate) req: &'a super::chat_loop::state::TurnRequest,
    /// 本幕已备好的输入（`prepare_turn_inputs` 的产物）——**两个环步之间的交接件**，
    /// 与 [`StageStep::call`] 同一层意思：准备放进去，派出取走。
    ///
    /// 它不在 `TurnState` 里：它的寿命就是这一幕（产出 → 消费 → 收尾读一次
    /// `root_id`），而 `TurnState` 是整轮的容器。把它抬进去等于让「一次只用一次的
    /// 中间值」拥有轮级寿命——D-168 的诚实划界 2 说的正是这件事还没做完。
    pub(crate) inputs: &'a mut Option<TurnInputs>,
    /// 本轮用户发言（`execute_turn` 的技能命中判定与轮末学习用它）。
    pub(crate) user_text: &'a str,
    /// 两个环步之间的交接件：已宣布、还没派出的那一笔。
    pub(crate) call: &'a mut Option<Call>,
    /// 派出的结果（成功 / 中止 / 失败），由编排层在环之后取。
    pub(crate) outcome: &'a mut Option<Result<ExecTurnResult, DispatchError<TurnExecExit>>>,
}

/// 外环绑定表里的动作类型（与前两台同形状，借的面不同）。
pub(crate) type StageAction = for<'a> fn(&'a mut StageStep<'a>) -> StepFuture<'a>;

/// `prepare-turn`：为本幕备好一次请求的输入（D-168 裁决 3）。
///
/// 本体就是 [`super::chat_loop::inputs::prepare_turn_inputs`]——它是提示词 / 工具 /
/// 请求视图的**唯一收集点**（收口 ②），而在它的末尾起着轮首那台 host（判决 / 压缩 /
/// 召回 / 快路）。搬到环上之后这些一件都没变，变的是「做完了」这件事有了事实形态：
/// 动作落 `turn.stage{stage:"inputs"}`，下一问问表读的就是那一格。
///
/// ## 两种结局都要落格
///
/// - `Ok(inputs)` ⇒ `result: "ready"`，输入放进交接件；
/// - `Err(TurnExit::Completed)` ⇒ 本幕在准备段就收束了（判决判成「就此答话」，收束格
///   由 `land_answered_turn` 落），`result: "closed"`。这一档**没有**输入，也不该有往返。
///
/// 只落 `ready` 的话，收束的那一幕在事实面上与「还没准备」同形——派发点会重来一遍，
/// 而第二遍已经没有寄存的那体答话，于是它照常返回输入、照常派出一次往返：一个已经
/// 答完的轮会再答一次。
pub(crate) fn run_prepare_turn<'a>(step: &'a mut StageStep<'a>) -> StepFuture<'a> {
    Box::pin(async move {
        let prepared = super::chat_loop::inputs::prepare_turn_inputs(
            step.orchestrator,
            step.ctx,
            &mut *step.context,
            step.sink,
            &mut *step.turn,
            step.req,
        )
        .await;
        let round = step.turn.request_round;
        let (result, inputs) = match prepared {
            Ok(inputs) => ("ready", Some(inputs)),
            Err(super::chat_loop::state::TurnExit::Completed) => ("closed", None),
            // 轮首读失败（会话端口的一次读失败）：端口面上没有一笔账可关，交回编排层
            // 走 `finish_turn(Failed)` 的既有路径（`ActorError::Read` 的文档）。
            Err(super::chat_loop::state::TurnExit::Failed(e)) => {
                return Err(ActorError::Read(e));
            }
            // 其余出口不该从准备段冒出来：它们各有自己的入口（中止在轮内 1c、
            // 边界中止在闸门、恢复完成在循环之前）。硬造一格只会把「怎么到这里
            // 的」这条信息抹掉。
            Err(other) => {
                return Err(ActorError::Read(
                    crate::symbio_core::PluginError::InternalError(format!(
                        "准备段返回了不属于它的出口：{other:?}"
                    )),
                ));
            }
        };
        super::stage_events::land(
            &**step.store,
            step.turn.turn_no,
            step.turn.user_seq,
            &crate::symbio_core::request_principal(step.ctx.as_ref()),
            &[super::stage_events::StageFact::inputs(
                round,
                step.turn.drive_seq,
                result,
            )],
        )
        .map_err(ActorError::Append)?;
        *step.inputs = inputs;
        Ok(())
    })
}

/// `announce-turn`：宣布本幕的 `turn` 往返（落 `request(port=turn)`），把这一笔留在
/// 交接件里。
///
/// 逻辑 id 含**幕锚**（`drive_seq`）与请求轮次：续写幕复用同一轮号且 `request_round`
/// 从 0 重来——只带轮号与往返号会与首幕撞成同一个 `id`，而首幕已经有过 `sent`
/// （幂等键命中之后派发行不欠，整轮在派发处被拒）。
pub(crate) fn run_announce_turn<'a>(step: &'a mut StageStep<'a>) -> StepFuture<'a> {
    Box::pin(async move {
        let call = Call::new(
            "turn",
            format!(
                "turn-{}-d{}-r{}",
                step.turn.turn_no, step.turn.drive_seq, step.turn.request_round
            ),
            1,
        );
        super::port::announce(&**step.store, &call, step.turn.turn_no, step.turn.user_seq)
            .map_err(ActorError::Append)?;
        *step.call = Some(call);
        Ok(())
    })
}

/// `dispatch-turn`：跑一次**已宣布**的 `turn` 往返（一整幕）。
///
/// 没有交接件（上一段生命在宣布与派出之间结束，或本任务接管前留下的遗留请求）⇒
/// 不跑，把最老的那一笔判死（[`super::port::kill_dangling`]）：等它的 future 已经
/// 没了，重试也没有对象。**不按幕锚收窄**——遗留请求恰恰落在本幕锚之前。
pub(crate) fn run_dispatch_turn<'a>(step: &'a mut StageStep<'a>) -> StepFuture<'a> {
    Box::pin(async move {
        let Some(call) = step.call.take() else {
            super::port::kill_dangling(&**step.store, "turn", "no_pending_round")
                .map_err(ActorError::Append)?;
            return Ok(());
        };
        // 输入是本幕的另一个交接件（准备那一环步放的）。没有它就没有「要发什么」——
        // 与没有宣布那一笔同一种处境：这一环步跑不了，把请求判死而不是发一条空请求。
        let Some(inputs) = step.inputs.take() else {
            crate::plugin_warn!(
                "session",
                "[Loop] 本幕没有备好的输入，`turn` 往返不派出（turn={}）",
                step.turn.turn_no
            );
            super::port::kill_dangling(&**step.store, "turn", "no_prepared_inputs")
                .map_err(ActorError::Append)?;
            return Ok(());
        };
        // 压缩的寄存位**在这一刻取走**（S2.7-b2）：`take` 而不是 `clone`——同一份
        // 快照只该落一次。位置也必须在环里：压缩是**准备段**跑的（`policy.compress`
        // 由轮首那台 host 选中），在环之前取会取到空值，压缩事实于是静默消失
        // （e2e t8 / t11 / t45 抓到的正是这一条）。
        let consolidation = step.context.consolidated.take();
        // 字段逐个借出去（三样 `&mut` 按字段重借，互不重叠）：`dispatch` 的闭包会把
        // 那几份借用**移进** future，之后再碰 `step` 就借不动了——只有 `outcome`
        // 需要活着，所以它单独借、单独写回。
        let parts = TurnParts {
            orchestrator: step.orchestrator,
            orchestrator_arc: step.orchestrator_arc,
            ctx: step.ctx,
            sink: step.sink,
            context: &mut *step.context,
            turn: &mut *step.turn,
            inputs: &inputs,
            user_text: step.user_text,
            consolidation: consolidation.as_deref(),
        };
        let outcome = super::port::dispatch(
            &**step.store,
            super::loop_rows::PORTS,
            super::port::Dispatch {
                call: &call,
                turn: parts.turn.turn_no,
                produced_by: parts.turn.user_seq,
                row: "dispatch-turn",
                now: parts.turn.now_ms,
                // 整幕等在当前 future 里；崩溃后恢复走新的一幕（新 id），重试旧 id
                // 只会留下一条没人接的 `request`。
                awaited: false,
            },
            || run_turn(parts),
            // `failed` 的 `why`：中止与失败是两种停法，闭集（`ClosureReason`）管的是
            // 轮子，这里管的是这一次调用为什么没成功。
            |e: &TurnExecExit| match e {
                TurnExecExit::Aborted(_) => "aborted".to_string(),
                TurnExecExit::Failed(err) => format!("failed: {err}"),
            },
        )
        .await;
        // 放回槽里：收尾两段（`settle_reasoning` / `finalize_turn_root`）要读
        // `root_id`，而它们在环之外（D-168 裁决 5：那是本环步的产出处理）。
        *step.inputs = Some(inputs);
        *step.outcome = Some(outcome);
        Ok(())
    })
}

/// 一次 `turn` 往返本体要借的**八样**（[`StageStep`] 去掉两个交接件之后的投影）。
///
/// 单独成形是为了让 [`run_dispatch_turn`] 能把它们整体移进派发的闭包里，而把
/// `outcome` 那个借位留在手上——`dispatch` 的 `FnOnce() -> Fut` 要求 future 的类型
/// 不借闭包自身，拆成值传递才成立。
struct TurnParts<'a> {
    orchestrator: &'a ChatOrchestrator,
    orchestrator_arc: &'a Arc<ChatOrchestrator>,
    ctx: &'a Arc<dyn PluginInvokeRequest>,
    sink: &'a crate::symbio_core::ExecEventSink,
    context: &'a mut SessionContext,
    turn: &'a mut TurnState,
    inputs: &'a TurnInputs,
    user_text: &'a str,
    consolidation: Option<&'a [crate::symbio_core::chat_node::ChatNode]>,
}

/// 一次 `turn` 往返的本体：把借用面摊成 [`super::turn_exec::ExecTurn`]。
///
/// 与搬到这一层之前逐字同源（`chat_loop` 的调用点），差别只有取值的来源：
/// 每一样都从借用面取，不再从外层函数的局部变量取。
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
                        dyn std::future::Future<
                                Output = Option<crate::symbio_core::DrainedSupplement>,
                            > + Send,
                    >,
                > = Box::pin(async move {
                    let supplements: Option<crate::symbio_core::DrainedSupplement> = session
                        .run_op(crate::symbio_core::run_ops::TAKE_SUPPLEMENTS, ())
                        .await
                        .unwrap_or_else(|e| {
                            crate::plugin_warn!(
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

/// 一次外环驱动的**入参包**。
///
/// 九样东西一起构成「这一幕现在长什么样」：编排器（模型 / 树）、请求句柄（能力口）、
/// 会话上下文（消息镜像 + 句柄）、本轮状态（幕锚与寄存位）、本幕已备好的输入。
/// 分开传时同一个 `u64`/`&str` 很容易写反，而写反的后果是**静默**的（派出去的
/// 那一幕不是日志上那一幕）。结构体字面量把「谁是谁」写在字段名上——与
/// [`super::port::Dispatch`] 同一条理由。
pub(crate) struct StageDrive<'a> {
    pub(crate) orchestrator: &'a ChatOrchestrator,
    /// 编排器的共享句柄：批次边界的能力口要它（见 [`StageStep::orchestrator_arc`]）。
    pub(crate) orchestrator_arc: &'a Arc<ChatOrchestrator>,
    pub(crate) ctx: &'a Arc<dyn PluginInvokeRequest>,
    pub(crate) sink: &'a crate::symbio_core::ExecEventSink,
    pub(crate) context: &'a mut SessionContext,
    pub(crate) turn: &'a mut TurnState,
    /// 请求级配置快照（准备段要它：显式系统提示词与 `auto_compress` 都在里面）。
    pub(crate) req: &'a super::chat_loop::state::TurnRequest,
    /// 本轮用户发言（`execute_turn` 的技能命中判定与轮末学习用它）。
    pub(crate) user_text: &'a str,
}

/// 外环跑完一幕之后交回编排层的那两样。
///
/// 两样都是**产物**，不是判断：`inputs` 是本幕备好的请求输入（收尾两段要它的
/// `root_id`），`result` 是那一幕的结局。`inputs = None` 只有一个来源——准备段
/// 发现本幕已经收束（定稿答话轮），那一幕没有输入，也没有往返。
///
/// 为什么交回而不是寄存进 `TurnState`：这两样的寿命就是这一幕。寄存进轮级容器，
/// 就等于让「一次只用一次的中间值」活到整轮结束（D-168 的诚实划界 2）。
pub(crate) struct StageOutcome {
    pub(crate) inputs: Option<TurnInputs>,
    pub(crate) result: Option<Result<ExecTurnResult, DispatchError<TurnExecExit>>>,
}

/// 外环派发点：每步向求解器要**本 host 行子集**里最小的一笔可还债，跑它的执行体，
/// 直到这一台不再欠账（准备 / 宣布 / 派出三笔都关掉，或本幕收束）。
///
/// ## 两条停机判据，都是「读事实」
///
/// 1. **本幕已收束** ⇒ 立刻停。这是 D-168 裁决 2 的幕间判据：`next_stage_owed` 问的是
///    「这一幕还有账吗」，而收束格落下之后答案恒为否——留着不停，`announce-turn`
///    会为一个**已经答完的轮**宣布一次往返（它在事实面上与「还没派出」同形：两者都
///    没有 `response(port=turn)`）。
/// 2. **选不到可还的债** ⇒ 停，与轮首那台同形：不拿假成功收场。
pub(crate) async fn drive_stage(drive: StageDrive<'_>) -> Result<StageOutcome, ActorError> {
    let StageDrive {
        orchestrator,
        orchestrator_arc,
        ctx,
        sink,
        context,
        turn,
        req,
        user_text,
    } = drive;
    let Some(store) = context.session.facts() else {
        return Ok(StageOutcome {
            inputs: None,
            result: None,
        });
    };
    let mut call: Option<Call> = None;
    let mut inputs: Option<TurnInputs> = None;
    let mut outcome = None;
    loop {
        let facts = store.range(Seq::new(0));
        if super::loop_rows::stage_closed(&facts, turn.turn_no, turn.now_ms) {
            break;
        }
        let Some(next) = super::loop_rows::next_stage(
            &facts,
            turn.turn_no,
            turn.now_ms,
            super::loop_rows::STAGE_ACTORS,
        ) else {
            break;
        };
        crate::plugin_debug!(
            "session",
            "[Loop] 外环派发 {}（执行体 {}；债 {}）",
            next.row,
            next.by,
            next.key
        );
        let head = store.head();
        {
            let mut step = StageStep {
                orchestrator,
                orchestrator_arc,
                ctx,
                store: &store,
                sink,
                context,
                turn,
                req,
                user_text,
                inputs: &mut inputs,
                call: &mut call,
                outcome: &mut outcome,
            };
            if let Err(e) = (next.action)(&mut step).await {
                crate::plugin_warn!(
                    "session",
                    "[Loop] 外环执行体 {} 失败（行 {}，turn={}）: {e:?}",
                    next.by,
                    next.row,
                    turn.turn_no
                );
                return Err(e);
            }
        }
        let ledger = store.range(head);
        if ledger.is_empty() {
            crate::plugin_warn!(
                "session",
                "[Loop] 外环执行体 {} 落了零格（行 {}，turn={}）——出口没进日志，本 host 停驱",
                next.by,
                next.row,
                turn.turn_no
            );
            break;
        }
        // 派发点重读切片：下一问问表读的是**刚落的那一格**。
        turn.facts = store.range(Seq::new(0));
    }
    Ok(StageOutcome {
        inputs,
        result: outcome,
    })
}

#[cfg(test)]
#[path = "stage_exec.test.rs"]
mod tests;
