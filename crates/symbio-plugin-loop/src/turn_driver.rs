//! 轮次驱动器（`turn_driver`）：生产入口与外层环（P2-6a）。
//!
//! `provider::run_turn` 只调 [`run_turn`]。本模块做两件事：
//!
//! 1. [`run_turn`]：装配会话端口与编排器，交给 [`chat_loop::run_chat_loop`] 做前步骤
//!    （请求解析、状态准备）与收尾（落库映射）；它不再决定「下一步是什么」。
//! 2. [`drive`]：按冻结行表选一笔债 → 交给对应行/端口受理 → 读账本判本幕收束
//!    （停机格 [`crate::turn_exec::ends_the_drive`] 或下一幕 [`crate::loop_rows::next_stage_round`]）。
//!
//! 幕首、收幕与各行实现仍是 `turn_exec` 的被动机制：本环只做**选择**与**收束判定**，
//! 不解释任何行名 / 端口名 / 业务语义（`D-118` 决策 2）。
//!
//! 旧的 [`crate::turn_exec::drive`] 保留为薄转发（测试与 `chat_loop` 的同名入口），
//! 不再自己持有循环。

use std::sync::Arc;

use symbio_kernel::event::Seq;
use symbio_kernel::plugin::PluginInvokeRequest;
use symbio_sdk::turn::dispatch::{TurnDispatch, TurnReport};

use crate::chat_loop::{run_chat_loop, ChatOrchestrator};
use crate::loop_rows;
use crate::session_port::{PortSession, SessionPort};
use crate::turn_exec::drive::{
    abandon, actor_exit, append_exit, close_stage, current_drive_seq, driver_aborted,
    driver_answered, exhausted, no_host, no_stage, open_stage, CloseEnd, DriveEnd, DriveStart,
    Driver, ReadyDriver, StageRuntime,
};
use crate::turn_exec::{Ctx, HeadParts, TurnExecExit};

/// 生产入口：装配一轮的会话句柄与编排器，跑到收尾。
pub(crate) async fn run_turn(
    host: Arc<dyn PluginInvokeRequest>,
    dispatch: Arc<TurnDispatch>,
) -> TurnReport {
    let port: Arc<dyn SessionPort> = Arc::new(PortSession::new(host.clone()));
    let view = dispatch.spec.view();
    let settings = view.settings.clone();
    let orchestrator = Arc::new(ChatOrchestrator {
        parent: dispatch.parent.clone(),
        context_limit: dispatch.context_limit,
        view,
        facts: dispatch.facts.clone(),
        session: port,
        writer: dispatch.writer.clone(),
        compose_enabled: settings.compose_enabled,
        opening_enabled: settings.opening_enabled,
        presentation_tree: settings.presentation_tree,
    });
    run_chat_loop(
        orchestrator,
        host,
        dispatch.sink.clone(),
        dispatch.abort.clone(),
    )
    .await
}

/// 一圈的结局。
enum StepEnd {
    /// 本圈落了账、还在跑。
    Continue,
    /// 本轮收束：结局已从草稿取走。
    Settled(crate::turn_runner::TurnOutcome),
    /// 判决判成「就此答话」。
    Answered,
}

pub(crate) async fn drive(
    start: DriveStart<'_>,
    actors: &'static [loop_rows::ActorEntry],
) -> Result<DriveEnd, TurnExecExit> {
    let mut driver = match start {
        DriveStart::Control(control) => Driver::Control(control),
        DriveStart::Ready(ready) => Driver::Ready(Box::new(ReadyDriver {
            stage: Some(ready.stage),
            head: ready.head,
            request_ctx: ready.request_ctx,
        })),
    };
    let mut stage: Option<StageRuntime> = None;
    let mut first_stage = true;
    let mut first_step = true;
    // 已收束那一幕的幕锚：控制面把轮次推到下一幕之后，旧幕才让位。
    //
    // 让位不能排在收束当场：`continue-turn` 那一行还要用这一幕的草稿（`Ctx.session`
    // 不是 `Option`），而它正是由本环在收束之后选中的。
    let mut retiring: Option<u64> = None;

    loop {
        // ── 幕间中止：上一幕已收束、下一幕不开（`D-052` 决策 1）───────────
        //
        // `retiring` 是「上一幕已收束、`continue-turn` 还没跑」的唯一标记——只有
        // 这一圈才是**幕界**。幕内（含首幕）的中止不在这里判：那会把「本轮没有
        // 收束格」当成正常出口，而中止是**状态**、须由该幕自己的收束格记下。
        //
        // 排在让位与选债**之前**：被中止的幕连 `continue-turn` 都不跑——那一行会在
        // 事实面上留下一格「还有下一轮」的轮次格，而这一轮其实已经收工。
        if retiring.is_some() && driver_aborted(&driver) {
            return Ok(DriveEnd::Boundary);
        }
        // ── 幕间：旧幕让位 ───────────────────────────────────────────────
        //
        // 判据是幕锚变了：`continue-turn` 落地之后 `turn.drive_seq` 指向新落的
        // `round` 格，与已收束那一幕的锚不同 ⇒ 旧幕（只剩一个空壳草稿）可以丢了。
        if let Some(seq) = retiring {
            if current_drive_seq(&driver) != Some(seq) {
                stage = None;
                retiring = None;
            }
        }
        // ── 幕首：没有幕就开一幕（`D-222` 订正 3：草稿建在幕首）───────────
        if stage.is_none() {
            stage = Some(match &mut driver {
                Driver::Control(control) => open_stage(control, first_stage).await?,
                Driver::Ready(ready) => ready.stage.take().ok_or_else(no_stage)?,
            });
            first_stage = false;
        }

        // ── 幕内：一圈 ───────────────────────────────────────────────────
        let step: StepEnd = 'step: {
            let st = stage.as_mut().ok_or_else(no_stage)?;

            // 守卫一：闸门与首请求之间的中止（`D-052` 决策 1）。只在**整次驱动**的
            // 第一圈：此后每一笔请求的结局都由端口面记成事实，不再由这里判；幕间那
            // 一种已由上面那道 `Boundary` 接住（同一轮不落第二格收束）。
            if first_step {
                first_step = false;
                if let Some(outcome) = st
                    .draft
                    .abort_before_first_request(st.store.as_ref(), st.llm.as_ref())
                    .await
                    .map_err(append_exit)?
                {
                    break 'step StepEnd::Settled(outcome);
                }
            }
            // 守卫二：判成「就此答话」的一轮不进驱动循环（`D-162` 裁决 3）。
            if driver_answered(&driver) {
                break 'step StepEnd::Answered;
            }

            // 选债：读切片 → 最小可还债 → 按 `by` 查绑定表。不按行名 / 端口名 /
            // 业务语义分支（`D-118` 决策 2）。
            let facts = st.store.range(Seq::new(0));
            let now = st.draft.now_ms();
            let turn_no = st.draft.turn();
            let next = match loop_rows::next(&facts, turn_no, now, actors) {
                Some(next) => next,
                // 没有可还的债：要么已经收束，要么表装载失败 / 只剩 blocked 行。
                // 「只剩 blocked 行」有两种处境（`D-126`）：有一笔已派出未收清的往返
                // 时这一轮不是「没事」，是「在等」——混成一个值会让「为什么停」在
                // 事实面上不可判，而恢复路径恰恰要认这一种。
                None => {
                    let detail = if loop_rows::waiting(&facts, turn_no, now) {
                        "waiting"
                    } else {
                        "no_progress"
                    };
                    break 'step StepEnd::Settled(exhausted(st, detail).await?);
                }
            };
            symbio_plugin_api::plugin_debug!(
                "session",
                "[Loop] 派发 {}（执行体 {}；债 {}）",
                next.row,
                next.by,
                next.key.trace()
            );
            // 守卫三：步数上界是**收工的判据**，不是「半路撂下」的许可（`D-222`
            // 订正 2）。有一笔在飞时先把那一笔收回来，否则日志上留一条 `sent` 无
            // 终局，重启后 `reap` 要判死一笔其实已经答了的往返。
            //
            // 已收束那一幕不再数步：本圈要跑的是 `continue-turn`（它不属于谁的
            // 工作），而上界约束的是**一幕之内**的往返。
            if retiring.is_none()
                && st.draft.over_step_limit()
                && !loop_rows::waiting(&facts, turn_no, now)
            {
                symbio_plugin_api::plugin_warn!(
                    "actors",
                    "[v2] 轮内步数超过上限（turn={}），收束为 exhausted（step_limit）",
                    turn_no
                );
                break 'step StepEnd::Settled(exhausted(st, "step_limit").await?);
            }

            let head_before = st.store.head();
            {
                // 请求句柄：控制面在轮首片里，裸幕夹具由入口直给（`head = None`）。
                let request_ctx = match &mut driver {
                    Driver::Control(control) => Some(control.ctx.clone()),
                    Driver::Ready(ready) => ready
                        .request_ctx
                        .clone()
                        .or_else(|| ready.head.as_ref().map(|h| h.request_ctx.clone())),
                };
                // 草稿与端口逐圈再借：动作结束即还（`D-222` 裁决 5 的「按步获取
                // 按步释放」）。`head` 那一片同样每圈重借一次。
                let mut control_parts;
                let head = match &mut driver {
                    Driver::Control(control) => {
                        // 控制面的幕一定有宿主面（`open_stage` 建的）；没有就是幕与
                        // 驱动面对不上，出声而不是让轮首行读到一片假的空。
                        control_parts = Some(
                            st.head_parts(
                                control.orchestrator_arc.clone(),
                                control.ctx.clone(),
                                &mut *control.context,
                                &mut *control.turn,
                                control.req,
                            )
                            .ok_or_else(no_host)?,
                        );
                        control_parts.as_mut().map(HeadParts::as_ctx)
                    }
                    Driver::Ready(ready) => ready.head.as_mut().map(HeadParts::as_ctx),
                };
                let mut ctx = Ctx {
                    session: &mut st.draft,
                    store: &st.store,
                    llm: st.llm.clone(),
                    tok: st.tok,
                    sink: st.sink.clone(),
                    tools: st.tools.clone(),
                    dispatch: st.dispatch.clone(),
                    // 债实例随环境进动作：动作只收 `&mut Ctx`，「这是哪一笔债」
                    // 只能由派发器写在环境上（见 `Ctx::row`）。
                    row: next.row,
                    key: next.key.clone().0,
                    head,
                    request_ctx,
                };
                // 动作只回答「这一步做完了 / 失败了」：本轮收没收由下面的账本判
                // （`D-123` 决策 5）。
                (next.action)(&mut ctx).await.map_err(actor_exit)?;
            }

            // **账本**（`D-123` 决策 1）：这一步的**实际写入** = 日志尾段，不是执行
            // 体自报的一份陈述。写不经过返回值（`DE-015` §2.3「单写者是 store」），
            // 所以自报的 `Vec<Fact>` 漏一条就对不上一条。
            let ledger = st.store.range(head_before);
            // 守卫四：运行期写入封闭（`DE-015` §3.4 第 5 条，`D-123` 决策 2）。
            // 领域格必须在这个执行体登记过的名单里，协议五格豁免（引擎写）。
            // **全部行同一份判据**：没有任何一行在窗口里显式放弃这一条。越界不静默、
            // 不 panic：落一条带理由的停机格，让它在事实面显形。
            if let Some(offender) = ledger
                .iter()
                .map(|e| e.kind.as_str())
                .find(|k| !next.writes_closed(k))
            {
                symbio_plugin_api::plugin_warn!(
                    "actors",
                    "[v2] 执行体 {} 写了未登记的事实 `{}`（行 {}，turn={}）——名单 ⊆ 声明的写入封闭在运行期破了",
                    next.by,
                    offender,
                    next.row,
                    turn_no
                );
                break 'step StepEnd::Settled(exhausted(st, "undeclared-write").await?);
            }
            // 收束判断读账本（`D-123` 决策 5）：产出是收束那一步寄存进草稿的。
            if let Some(closing) = ledger.iter().find(|e| crate::turn_exec::ends_the_drive(e, turn_no)) {
                let Some(outcome) = st.draft.take_settled() else {
                    // 账本说收了工、草稿里没有产出 = 有一条收束路径写了停机格却没寄存
                    // 它。不猜一个产出（那等于把「没记账」读成「记了个空账」）。
                    symbio_plugin_api::plugin_warn!(
                        "actors",
                        "[v2] 执行体 {} 落了停机格 `{}`（行 {}，turn={}）却没寄存本轮产出——收束路径漏记",
                        next.by,
                        closing.kind,
                        next.row,
                        turn_no
                    );
                    break 'step StepEnd::Settled(exhausted(st, "unrecorded-outcome").await?);
                };
                break 'step StepEnd::Settled(outcome);
            }
            // 守卫五：空账本（`D-118` 决策 7 + `D-123` 决策 1）。「没写」有两种
            // （`D-222` 订正 2）：一次动作把在飞的 future poll 了一次、它还没好——
            // 那不是没账可还，是环**合法地不前进**（等 I/O）。判据靠形状。
            if ledger.is_empty() && !loop_rows::waiting(&facts, turn_no, now) {
                break 'step StepEnd::Settled(exhausted(st, "no_progress").await?);
            }
            StepEnd::Continue
        };

        match step {
            StepEnd::Continue => continue,
            StepEnd::Answered => {
                let st = stage.take().ok_or_else(no_stage)?;
                abandon(st).await;
                return Ok(DriveEnd::Answered);
            }
            StepEnd::Settled(outcome) => {
                // 幕锚只有控制面有（夹具的幕不接续），让位判据因此也只对控制面成立。
                let seq = current_drive_seq(&driver);
                let mut st = stage.take().ok_or_else(no_stage)?;
                match close_stage(&mut driver, &mut st, outcome).await? {
                    // 夹具没有宿主面：结局按值交回，归类与落库由夹具自己做。
                    CloseEnd::Bare(outcome) => return Ok(DriveEnd::Bare(outcome)),
                    // 夹具自带宿主面：交回归类的输入（`Running`），后续同样归夹具。
                    CloseEnd::Running(running) => return Ok(DriveEnd::Running(running)),
                    CloseEnd::Settled => {
                        // 控制面：本幕收束了。还开不开下一幕读事实，不读返回值
                        // （`D-222` 裁决 2 / 裁决 3）。
                        let Driver::Control(control) = &mut driver else {
                            return Ok(DriveEnd::Completed);
                        };
                        let seq = seq.ok_or_else(no_stage)?;
                        if loop_rows::next_stage_round(
                            &control.turn.facts,
                            control.turn.turn_no,
                            control.turn.now_ms,
                        )
                        .is_none()
                        {
                            return Ok(DriveEnd::Completed);
                        }
                        // 还有下一幕：草稿留着给 `continue-turn` 用，宿主面已交出去。
                        stage = Some(st);
                        retiring = Some(seq);
                    }
                }
            }
        }
    }
}
