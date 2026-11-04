//! 义务环（`ring`）：一张债表上的一层薄循环。
//!
//! 本模块只允许五类判断（白名单）：
//!
//! 1. **选债**：读事实切片 → [`crate::loop_rows::next`] 取最小可还的一笔；
//! 2. **`run_port`**：把这一笔交给绑定表里的动作，由动作按端口名受理；
//! 3. **落事实与关账**：读动作前后的账本尾段，判停机格（[`crate::turn_exec::ends_the_drive`]）；
//! 4. **预算 / 租约守卫**：`over_step_limit`（按已 `sent` 的笔数）与 `waiting`（未收清）；
//! 5. **停机判据**：无债可还时「在等 / 不前进」两档。
//!
//! 环**不**知道幕、阶段号或轮次：开一幕、收一幕、幕间接续都在 [`crate::entry`]。
//! 环`不`解释行名 / 端口名 / 业务语义（`D-118` 决策 2）；它不按任何序号分支。

use crate::entry::{DriveState, FrameEnd, StageRuntime, TurnExecExit as Exit};
use crate::loop_rows;
use crate::turn_exec::{actor_exit, ends_the_drive, exhausted, no_host, Ctx, HeadParts};
use symbio_kernel::event::Seq;

/// 驱动一幕直到它收束（或判成答话）。
///
/// 每圈：判成答话 → 选最小可还债 → 预算守卫 → 跑一个动作 → 读账本（写封闭 /
/// 停机格 / 空账本）→ 继续。幕的开关与接续不在本函数里。
pub(crate) async fn drive_frame(
    state: &mut DriveState<'_>,
    st: &mut StageRuntime,
    actors: &'static [loop_rows::ActorEntry],
) -> Result<FrameEnd, Exit> {
    drive_frame_inner(state, st, actors, 0).await
}

/// 在收束之后的幕上只跑**一笔**债（幕间接续：`continue-turn`）。
///
/// 返回是否选中并跑了债。它不做收束判定——那一幕已经收束，调用方只借这一步
/// 让 `continue-turn` 把新幕的锚落进事实。与 [`drive_frame`] 共用**同一个**
/// 循环体与同一个选债口（`sp-033 TC-1a/TC-1b`：选债调用只有一处）。
pub(crate) async fn run_one(
    state: &mut DriveState<'_>,
    st: &mut StageRuntime,
    actors: &'static [loop_rows::ActorEntry],
) -> Result<bool, Exit> {
    Ok(matches!(
        drive_frame_inner(state, st, actors, 1).await?,
        FrameEnd::Stepped
    ))
}

async fn drive_frame_inner(
    state: &mut DriveState<'_>,
    st: &mut StageRuntime,
    actors: &'static [loop_rows::ActorEntry],
    max_steps: usize,
) -> Result<FrameEnd, Exit> {
    let mut stepped = 0usize;
    loop {
        // 判成答话：判决行已经落下「这一轮不再发请求」的事实（状态合法性）。
        if state.answered() {
            return Ok(FrameEnd::Answered);
        }

        let facts = st.store.range(Seq::new(0));
        let now = st.draft.now_ms();
        let epoch = st.epoch();
        let next = match loop_rows::next(&facts, epoch, now, actors) {
            Some(next) => next,
            // 没有可还的债：要么已经收束，要么表装载失败 / 只剩 blocked 行。
            // 「只剩 blocked 行」有两种处境（`D-126`）：有一笔已派出未收清的往返
            // 时这一轮不是「没事」，是「在等」——混成一个值会让「为什么停」在
            // 事实面上不可判，而恢复路径恰恰要认这一种。
            None => {
                let detail = if loop_rows::waiting(&facts, epoch, now) {
                    "waiting"
                } else {
                    "no_progress"
                };
                return Ok(FrameEnd::Settled(exhausted(st, detail).await?));
            }
        };
        symbio_plugin_api::plugin_debug!(
            "session",
            "[Loop] 派发 {}（执行体 {}；债 {}）",
            next.row,
            next.by,
            next.key.trace()
        );

        // 预算守卫：上界是**收工的判据**，不是「半路撂下」的许可（`D-222` 订正 2）。
        // 有一笔在飞时先把那一笔收回来，否则日志上留一条 `sent` 无终局，重启后
        // `reap` 要判死一笔其实已经答了的往返。步数就是已 `sent` 的笔数。
        if st.draft.over_step_limit(&facts) && !loop_rows::waiting(&facts, epoch, now) {
            symbio_plugin_api::plugin_warn!(
                "actors",
                "[v2] 轮内步数超过上限（epoch={}），收束为 exhausted（step_limit）",
                epoch
            );
            return Ok(FrameEnd::Settled(exhausted(st, "step_limit").await?));
        }

        if let Some(end) = execute(state, st, next, &facts, now, epoch).await? {
            return Ok(end);
        }
        stepped += 1;
        if max_steps != 0 && stepped >= max_steps {
            return Ok(FrameEnd::Stepped);
        }
    }
}

/// 跑一笔债并读它的账本。返回 `Some(停机)` 表示这一幕到此为止。
async fn execute(
    state: &mut DriveState<'_>,
    st: &mut StageRuntime,
    next: loop_rows::Next,
    facts: &[symbio_kernel::event::Event],
    now: i64,
    epoch: u64,
) -> Result<Option<FrameEnd>, Exit> {
    let head_before = st.store.head();
    {
        // 请求句柄：控制面在轮首片里，裸幕夹具由入口直给（`head = None`）。
        let request_ctx = match &state.control {
            Some(control) => Some(control.ctx.clone()),
            None => state
                .request_ctx
                .clone()
                .or_else(|| state.head.as_ref().map(|h| h.request_ctx.clone())),
        };
        // 草稿与端口逐圈再借：动作结束即还（`D-222` 裁决 5 的「按步获取
        // 按步释放」）。`head` 那一片同样每圈重借一次。
        let mut control_parts;
        let head = match state.control.as_mut() {
            Some(control) => {
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
            None => state.head.as_mut().map(HeadParts::as_ctx),
        };
        let client = st.client();
        let capacities = st.capacities();
        let mut ctx = Ctx {
            session: &mut st.draft,
            store: &st.store,
            llm: client,
            tok: st.tok,
            sink: st.sink.clone(),
            tools: capacities,
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

    // **账本**（`D-123` 决策 1）：这一步的**实际写入** = 日志尾段，不是执行体
    // 自报的一份陈述。写不经过返回值（`DE-015` §2.3「单写者是 store」），
    // 所以自报的 `Vec<Fact>` 漏一条就对不上一条。
    let ledger = st.store.range(head_before);
    // 写封闭：运行期写入封闭（`DE-015` §3.4 第 5 条，`D-123` 决策 2）。
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
            "[v2] 执行体 {} 写了未登记的事实 `{}`（行 {}，epoch={}）——名单 ⊆ 声明的写入封闭在运行期破了",
            next.by,
            offender,
            next.row,
            epoch
        );
        return Ok(Some(FrameEnd::Settled(
            exhausted(st, "undeclared-write").await?,
        )));
    }
    // 收束判断读账本（`D-123` 决策 5）：产出是收束那一步寄存进草稿的。
    if let Some(closing) = ledger.iter().find(|e| ends_the_drive(e, epoch)) {
        let Some(outcome) = st.draft.take_settled() else {
            // 账本说收了工、草稿里没有产出 = 有一条收束路径写了停机格却没寄存
            // 它。不猜一个产出（那等于把「没记账」读成「记了个空账」）。
            symbio_plugin_api::plugin_warn!(
                "actors",
                "[v2] 执行体 {} 落了停机格 `{}`（行 {}，epoch={}）却没寄存本轮产出——收束路径漏记",
                next.by,
                closing.kind,
                next.row,
                epoch
            );
            return Ok(Some(FrameEnd::Settled(
                exhausted(st, "unrecorded-outcome").await?,
            )));
        };
        return Ok(Some(FrameEnd::Settled(outcome)));
    }
    // 空账本（`D-118` 决策 7 + `D-123` 决策 1）。「没写」有两种
    // （`D-222` 订正 2）：一次动作把在飞的 future poll 了一次、它还没好——
    // 那不是没账可还，是环**合法地不前进**（等 I/O）。判据靠形状。
    if ledger.is_empty() && !loop_rows::waiting(facts, epoch, now) {
        return Ok(Some(FrameEnd::Settled(exhausted(st, "no_progress").await?)));
    }
    Ok(None)
}
