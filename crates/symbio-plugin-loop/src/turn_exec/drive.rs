//! 控制面的**唯一一个环**（`D-222` 裁决 5 / 判据 8）。
//!
//! 轮首四行、接缝、汇报与轮内各行都是[一张绑定表](super::super::loop_rows::ACTORS)
//! 上的行；幕与幕在**同一次调用**里接续——`continue-turn` 那一行由本环选中，它落下
//! 的 `round` 格既是上一笔的出口、也是下一幕的锚（`D-222` 裁决 2）。
//!
//! ## 三段，一个循环体
//!
//! | 段 | 做什么 | 在哪里 |
//! |---|---|---|
//! | 幕首 | 上下文 → 开一幕（用户格 / 恢复格 / 轮次格）→ 纯读 → 定根 → 装配 → 草稿 | [`open_stage`]，环顶 |
//! | 幕内 | 选最小可还债 → 跑**一个**动作 → 读账本 → 五道守卫 | 循环体（标号 `'step`） |
//! | 收幕 | 归类 → 派生事实 → 镜像并入 → 定格 → 落库 → 封根 | [`close_stage`]，收束之后 |
//!
//! 幕首不是交换点之外的东西：它排在同一圈里，因为「这一幕还开不开」本身由上一幕的
//! 收束格与 `turn_next_round` 回答（`D-222` 裁决 3：流向读事实，不读返回值）。
//!
//! ## 五道守卫，各一份
//!
//! | 守卫 | 判据 | 位置 |
//! |---|---|---|
//! | 中止 | 整次驱动的第一圈、选债之前（`D-052` 决策 1） | 循环体开头 |
//! | 判成答话 | 每圈选债之前（收束格由编排层落） | 同上 |
//! | 步数上界 | 选债之后、动作之前；有一笔在飞时先收回（`D-222` 订正 2） | 同上 |
//! | 写封闭 | 动作之后、账本尾段（`D-123` 决策 2） | 环尾 |
//! | 空账本 | 同上；「在等」不判死（`D-222` 订正 2） | 环尾 |
//!
//! 第五道「不重入」在类型上成立：草稿是 `&mut TurnSession`，一个循环体、按步借、
//! 按步还——没有任何动作能在窗口里再起第二个环。
//!
//! ## 幕间中止不落第二格收束
//!
//! 首请求前的中止由运行器落中止收束格（`f-{turn}`）；**幕间**的中止不能再落一次
//! ——同一轮的第二格收束会撞幂等键。所以整次驱动只在第一圈问中止
//! （[`TurnSession::abort_before_first_request`]），此后每一圈的开头只判
//! [`DriveEnd::Boundary`]：上一幕已收束、下一幕不开。
//!
//! ## 两条入口
//!
//! [`DriveStart::Control`] 是生产：编排层交下本轮状态，本函数一幕一幕地开、跑到
//! 全部幕收工。[`DriveStart::Ready`] 是夹具：幕已经开好（`begin` + 基线 + 前缀都已
//! 填），本函数只驱动它——两条入口共用**同一个**循环体与同一组守卫，差的只是「幕
//! 从哪来」与「收束后交给谁」。

use std::sync::Arc;

use symbio_core::chat_node::ChatNode;
use symbio_core::{
    CapabilityMeta, Event, ExecEventSink, FullModel, LlmAdapter, PluginError, Seq, Store,
};

use super::super::chat_loop::state::{
    ChatOrchestrator, SessionContext, TurnExit, TurnRequest, TurnState,
};
use super::super::loop_rows;
use super::super::report_exec::ReportLanding;
use super::super::tool_dispatch::{DerivedFacts, PortOutput, SessionDispatchPort, ToolDispatch};
use super::super::turn_runner::{TurnOutcome, TurnSession};
use super::{
    classify_turn, open_turn, persist_turn, ActorError, Assembled, Ctx, HeadParts, Running,
    StepFuture, TurnExecExit,
};

/// 一次驱动的入口。
///
/// 两支都装箱：它们各自按值带着一整幕的东西（`Control` 里有用户消息，`ReadyStage`
/// 里有整份装配束），而这个枚举只在一次调用的栈上活一瞬——按最大支定尺寸没有理由。
pub(crate) enum DriveStart<'a> {
    /// 生产：从控制面开第一幕，幕后接续。
    Control(Box<Control<'a>>),
    /// 夹具：幕已经开好，只驱动这一幕。
    #[cfg_attr(not(test), allow(dead_code))]
    Ready(Box<ReadyStage<'a>>),
}

/// 控制面交下来的那一片（编排层与本轮状态）。
///
/// 借的是调用方的 `&mut`：本函数每一圈把它们再借给一个动作（[`HeadCtx`]），
/// 收工之后调用方还要读它们（收口与最后一条回复）。
pub(crate) struct Control<'a> {
    pub(crate) orchestrator: &'a ChatOrchestrator,
    pub(crate) orchestrator_arc: &'a Arc<ChatOrchestrator>,
    pub(crate) ctx: &'a Arc<dyn symbio_core::PluginInvokeRequest>,
    pub(crate) sink: &'a ExecEventSink,
    pub(crate) context: &'a mut SessionContext,
    pub(crate) turn: &'a mut TurnState,
    pub(crate) req: &'a TurnRequest,
    /// 本请求带的那条用户消息（首幕追加进上下文；续幕为 `None`）。
    pub(crate) single_message: Option<ChatNode>,
    /// 唤醒动作名（`None` = 新开一轮）。
    pub(crate) resume_kind: Option<&'a str>,
}

/// 夹具交下来的**已经开好的幕**（`begin` 之后、驱动之前的那一刻）。
///
/// 幕按两种夹具各自能造出来的东西分两档：`StageRuntime` 自带宿主面时（夹具照生产的
/// 幕首装配），收幕把归类的输入按值交回（[`DriveEnd::Running`]）；没有宿主面时
/// （夹具只造了草稿与端口），收幕只交回结局（[`DriveEnd::Bare`]）。两条都**不做**
/// 归类、镜像并入与落库——那是控制面的收尾，夹具自己按要验的判据决定做多少。
pub(crate) struct ReadyStage<'a> {
    pub(crate) stage: StageRuntime,
    /// 轮首那一片（`None` = 夹具不跑轮首行）。
    pub(crate) head: Option<HeadParts<'a>>,
}

/// 一次驱动的结局（流向不在这里：`D-222` 裁决 3）。
pub(crate) enum DriveEnd {
    /// 判决判成「就此答话」：这一幕一次请求都没发，收束格由编排层落。
    Answered,
    /// 幕间中止：上一幕已收束，下一幕没开（不落第二格收束）。
    Boundary,
    /// 控制面跑完了全部幕（收束格与派生事实都已落格）。
    Completed,
    /// 夹具路径：一幕的结局按值交回（没有宿主面可归类）。
    #[cfg_attr(not(test), allow(dead_code))]
    Bare(TurnOutcome),
    /// 夹具路径：幕有宿主面，收幕交出归类要的交付束（归类与落库由夹具自己做）。
    #[cfg_attr(not(test), allow(dead_code))]
    Running(Box<Running>),
}

/// 谁在驱动这个环。
///
/// 两支同样装箱：`Driver` 活在整次驱动里，它的尺寸会被每一圈扛着。
enum Driver<'a> {
    Control(Box<Control<'a>>),
    /// 夹具：幕在第一次循环时从 `stage` 取走，头那一片留在 `head`（每圈再借）。
    Ready(Box<ReadyDriver<'a>>),
}

/// 夹具那一路的驱动状态（[`Driver::Ready`] 的载荷）。
struct ReadyDriver<'a> {
    stage: Option<StageRuntime>,
    head: Option<HeadParts<'a>>,
}

/// 一幕的运行时：草稿 + 驱动它要的端口 + 收束要的宿主面。
pub(crate) struct StageRuntime {
    draft: TurnSession,
    store: Arc<dyn Store<Event = Event>>,
    llm: Arc<dyn LlmAdapter>,
    tok: FullModel,
    sink: Arc<dyn symbio_core::DeltaSink>,
    tools: Arc<[CapabilityMeta]>,
    dispatch: Option<Arc<dyn ToolDispatch>>,
    /// 生产：收束要的宿主面。夹具为 `None`（收束时按值交回结局）。
    host: Option<StageHost>,
}

impl StageRuntime {
    /// 夹具的裸幕：草稿与端口齐了，但没有宿主面（没有装配束、发射任务与汇报落地面）。
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn bare(
        draft: TurnSession,
        store: Arc<dyn Store<Event = Event>>,
        llm: Arc<dyn LlmAdapter>,
        tok: FullModel,
        sink: Arc<dyn symbio_core::DeltaSink>,
        tools: Arc<[CapabilityMeta]>,
        dispatch: Option<Arc<dyn ToolDispatch>>,
    ) -> Self {
        StageRuntime {
            draft,
            store,
            llm,
            tok,
            sink,
            tools,
            dispatch,
            host: None,
        }
    }

    /// 夹具用：把快路正文寄进草稿。生产里由宿主在 `RowStep::Answer` 之后同一处调用
    /// （`turn_exec::run_plugin_row`），夹具只是把那一半补上。
    #[cfg(test)]
    pub(crate) fn set_reflex_for_test(&mut self, text: Option<String>) {
        self.draft
            .set_reflex(text.map(|text| super::super::turn_runner::ReflexAnswer { text }));
    }

    /// 轮首那一片的**owned 半边**：桥 / 发射口 / 根 id / 主体 / 汇报面全从宿主面克隆。
    ///
    /// 写在幕上而不是每个调用点各拼一份：这两组句柄只该有一个来源（装配束），
    /// 拼第二份就是拼第二个读数点。`context` / `turn` / `req` 仍是借的——它们每圈
    /// 要重新借（[`HeadParts::as_ctx`]），而且出借方是调用方。
    pub(crate) fn head_parts<'h>(
        &self,
        orchestrator: Arc<super::super::chat_loop::state::ChatOrchestrator>,
        request_ctx: Arc<dyn symbio_core::PluginInvokeRequest>,
        context: &'h mut super::super::chat_loop::state::SessionContext,
        turn: &'h mut super::super::chat_loop::state::TurnState,
        req: &'h super::super::chat_loop::state::TurnRequest,
    ) -> Option<HeadParts<'h>> {
        let host = self.host.as_ref()?;
        Some(HeadParts {
            orchestrator,
            request_ctx,
            loop_session: host.assembled.session.clone(),
            context,
            turn,
            req,
            bridge: host.assembled.bridge.clone(),
            event_sink: host.assembled.sink.clone(),
            root_id: host.assembled.root_id.clone(),
            fresh_root: host.assembled.fresh_root,
            actor: host.assembled.actor.clone(),
            report_port: host.assembled.report.clone(),
            report_landing: host.report_landing.clone(),
        })
    }
}

/// 幕的宿主面（装配束 + 发射任务 + 汇报落地面 + 分发方本体）。
///
/// 四样都在收幕时被拆开消费：[`StageHost::port`] 的两个出参要先取走（产出 / 派生
/// 事实），桥要随装配束一起丢（发射端退出的唯一信号），发射任务要收干。
struct StageHost {
    assembled: Assembled,
    emitter: tokio::task::JoinHandle<()>,
    report_landing: ReportLanding,
    port: Arc<SessionDispatchPort>,
}

/// 收幕的两条去向：夹具交回结局（或归类的输入），控制面继续接幕。
enum CloseEnd {
    Bare(TurnOutcome),
    Running(Box<Running>),
    Settled,
}

/// 一圈的结局。
enum StepEnd {
    /// 本圈落了账、还在跑。
    Continue,
    /// 本轮收束：结局已从草稿取走。
    Settled(TurnOutcome),
    /// 判决判成「就此答话」。
    Answered,
}

/// 驱动控制面：开一幕、跑一幕、收一幕，直到全部幕收工（`D-222` 裁决 5）。
pub(crate) async fn drive(
    start: DriveStart<'_>,
    actors: &'static [loop_rows::ActorEntry],
) -> Result<DriveEnd, TurnExecExit> {
    let mut driver = match start {
        DriveStart::Control(control) => Driver::Control(control),
        DriveStart::Ready(ready) => Driver::Ready(Box::new(ReadyDriver {
            stage: Some(ready.stage),
            head: ready.head,
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
            if let Some(closing) = ledger.iter().find(|e| super::ends_the_drive(e, turn_no)) {
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

// ── 幕首 ────────────────────────────────────────────────────────────────

/// 开一幕：上下文 → 开一幕的格 → 纯读 → 定根 → 装配 → 草稿 → 分发方。
///
/// `first_stage` 区分「本请求的第一幕」与「同一轮再接的一幕」：前者要落唤醒格
/// （用户格 / 恢复格），后者的 `round` 格已经由 `continue-turn` 那一行落下——
/// 再落一次就是第二个写方。
async fn open_stage(
    control: &mut Control<'_>,
    first_stage: bool,
) -> Result<StageRuntime, TurnExecExit> {
    let Control {
        orchestrator,
        orchestrator_arc,
        ctx,
        sink,
        context,
        turn,
        req,
        single_message,
        resume_kind,
    } = control;
    // ── 幕首 ①：本轮上下文（窗口生效；心跳可关历史）──────────────────────
    //
    // 每幕重新读：上一幕的原生产出已在收幕时并入落库镜像，这一幕的 prompt 要从
    // 那份镜像出（事实源是同一份，视图每幕刷新）。
    context.messages = if req.load_history {
        context
            .session
            .run_op::<Vec<ChatNode>>(symbio_core::run_ops::CONTEXT_MESSAGES, ())
            .await
            .unwrap_or_default()
    } else {
        // 心跳等无上下文场景：从空上下文开始，仅携带本轮新消息。
        Vec::new()
    };
    // 首幕追加本请求那条用户消息（去重：存储里可能已持久化）。
    if first_stage {
        if let Some(msg) = single_message.take() {
            if !context.messages.iter().any(|m| m.id == msg.id) {
                context.messages.push(msg);
            }
        }
    }
    turn.last_saved = context.messages.len();

    // ── 幕首 ②：开一幕的格（用户格 / 恢复格 / 轮次格），定下三个身份 ──────
    //
    // 格先于任何判决入格：判决只读事实，配置、轮号、幕锚都在这一处定下，后续阶段
    // 不再各自数一遍。续幕不在这里落格——它那一条已由 `continue-turn` 落过。
    let user_text = if first_stage {
        let opened = super::super::chat_loop::begin_drive(context, turn, req, resume_kind.take());
        to_exit(opened)?.user_text
    } else {
        // 本幕的锚与轮次都在 `turn` 上（`continue-turn` 的执行体刚写完）；用户发言
        // 从同一份镜像取（与首幕同一个判据：最近一条用户消息）。
        super::super::chat_loop::drive::current_user_text(context)
    };
    symbio_plugin_api::plugin_info!(
        "session",
        "[Turn] 第 {} 次请求开始 (msgs={})",
        turn.request_round + 1,
        context.messages.len()
    );

    // ── 幕首 ③：环前纯读（提示词 / 工具 / 开销）────────────────────────────
    let reads = super::super::chat_loop::inputs::collect_reads(ctx, req, turn).await;

    // ── 幕首 ④：定根（一幕一个根）────────────────────────────────────────
    //
    // 续写幕复用被续写轮的根（骨架已经发过，再发会把已完成的卡打回运行态）；
    // 新幕现分配。「定 id」在幕首（`begin` 的输入），「把骨架发给流式桥」仍在
    // 接缝行（`prepare-turn`，压缩之后：压不动时不该留半截 Turn 根）。
    let (root_id, fresh_root) = match turn.resume_root.take() {
        Some(root) => (root, false),
        None => (symbio_core::llm_short_id(), true),
    };
    // 轮内注入：把本会话的收件箱抽干口交给工具循环，让「用户中途插话」
    // **折进本轮**。抽干住在 session（队列与配置都在那边），驱动器经端口取。
    let supplements: super::SupplementFn = {
        let session = context.session.clone();
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
                        symbio_plugin_api::plugin_warn!(
                            "session",
                            "[Loop] 取出用户补充失败（本轮继续）: {e}"
                        );
                        None
                    });
                supplements
            });
            fut
        }) as super::SupplementFn
    };
    // 轮中汇报的能力口：**只产措辞**——判定与落格都不在这里（D-163）。
    // `None` = compose 产线关着：那一行到期时执行体交不回话，只落一格
    // `result:"silent"` 的尝试，这一段静默因此不再重复到期。
    let report = orchestrator.compose_enabled.then(|| {
        super::super::chat_loop::report_port(
            orchestrator_arc.clone(),
            ctx.clone(),
            context.messages.clone(),
        )
    });

    let (assembled, emitter) = super::assemble_turn(super::ExecTurn {
        session: &context.session,
        provider: orchestrator.provider.clone(),
        parent: orchestrator.parent.clone(),
        ctx: ctx.clone(),
        system_prompt: &reads.system_prompt,
        abort: turn.abort.clone(),
        root_id: &root_id,
        fresh_root,
        sink,
        user_text: &user_text,
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
    .await?;
    let open = open_turn(&assembled).await?;
    Ok(stage_runtime(assembled, emitter, open))
}

/// 幕首之后的那一半：装配束 + 发射任务 + 已开的草稿 → 一幕的运行时。
///
/// 单列出来是给夹具一条**同形**的入口：夹具的幕首可以自己备（它不需要
/// `collect_reads` / 定根 / `begin_drive` 那几行），但「分发方与汇报落地面怎么从
/// 装配束里造」只该有一处写法——照抄一遍就是第二个读数点。
pub(crate) fn stage_runtime(
    assembled: Assembled,
    emitter: tokio::task::JoinHandle<()>,
    open: super::OpenRun,
) -> StageRuntime {
    // 工具分发：契约与实现都在插件侧——它持插件宿主、请求上下文、转写出口与
    // 工具存档目录（core 认识这些即违 E-009）。
    let port = Arc::new(SessionDispatchPort::new(
        assembled.parent.clone(),
        assembled.ctx.clone(),
        assembled.sink.clone(),
        assembled.session.tool_archive_dir().to_path_buf(),
        assembled.store.clone(),
        assembled.abort.clone(),
        assembled.root_id.clone(),
        assembled.turn_no,
        assembled.user_seq,
        assembled.drive_seq,
        assembled.bridge.clone(),
        assembled.presentation_tree,
        open.elimination_revoked,
    ));
    // 汇报的落格面（事实源 / 实时桥 / 落库镜像 / 序号游标）：`policy.report` 的
    // 执行体读它。四样都是本幕已建好的句柄，环里没有第二处能造出同一组。
    let report_landing = ReportLanding {
        collected: assembled.collected.clone(),
        live: assembled.bridge.clone(),
        no: assembled.supplemental_no.clone(),
    };
    StageRuntime {
        draft: open.session,
        store: assembled.store.clone(),
        llm: open.llm,
        tok: open.tok,
        sink: assembled.bridge.clone(),
        tools: assembled.tools.clone(),
        dispatch: Some(port.clone() as Arc<dyn ToolDispatch>),
        host: Some(StageHost {
            assembled,
            emitter,
            report_landing,
            port,
        }),
    }
}

// ── 收幕 ────────────────────────────────────────────────────────────────

/// 收一幕：归类 → 派生事实 → 镜像并入 → 定格 → 落库 → 封根。
///
/// 宿主面按值消费（它活不过这一幕），草稿留在 [`StageRuntime`] 上——`continue-turn`
/// 那一行还要它（`Ctx.session` 不是 `Option`）。
async fn close_stage(
    driver: &mut Driver<'_>,
    st: &mut StageRuntime,
    outcome: TurnOutcome,
) -> Result<CloseEnd, TurnExecExit> {
    let Some(host) = st.host.take() else {
        return Ok(CloseEnd::Bare(outcome));
    };
    // 分发方也随幕去：它的产出先取走（工具轮的消息 / 派生事实），再放掉它——它是
    // 桥的一份强引用，留着会让发射端等到一个不会到来的关闭。
    st.dispatch = None;
    let StageHost {
        assembled,
        emitter,
        report_landing,
        port,
    } = host;
    let PortOutput {
        nodes: produced,
        derived,
    } = port.take_output();
    drop(port);
    drop(report_landing);
    let root_id = assembled.root_id.clone();
    let running = into_running(assembled, emitter, outcome, produced, derived);
    let Driver::Control(control) = driver else {
        return Ok(CloseEnd::Running(Box::new(running)));
    };
    // 归类（中止 / 授权拒 / 正常）与派生事实落格。借的是本轮状态里的
    // 召回视图，它在幕内被填好、环收工之后才交得出来。
    let res = classify_and_persist(running, control.turn.recall_view.as_ref()).await?;
    // 收口：镜像并入（v2 原生产生的消息）→ 中止复查 → 定格 → 落库 → 封根。
    //
    // 次序与 `stage` 拆出去之前逐字一致：`finalize_turn_root` 必须晚于定格（Turn 是
    // 组合节点，终态跟随子树），而落库是收束的写投影。
    let session = control.context.session.clone();
    super::super::chat_loop::merge_runner_messages(
        &session,
        control.context,
        control.turn,
        res.messages,
    )
    .await
    .map_err(TurnExecExit::Failed)?;
    if control.turn.abort.is_aborted() {
        // 请求结束后才置位的 abort 同样向上冒泡。本轮已完整产出（镜像已并入、随
        // 落库写入），没有半截要补 ⇒ `partial` 空。
        return Err(TurnExecExit::Aborted(Vec::new()));
    }
    let finish = super::super::chat_loop::settle_reasoning(
        control.orchestrator,
        control.context,
        control.sink,
        &root_id,
        res.output,
    )
    .await;
    super::super::chat_loop::persist_turn(control.sink, control.context, control.turn, finish)
        .await;
    super::super::chat_loop::finalize_turn_root(control.sink, control.context, &root_id).await;
    // 幕间判据与这一轮的收尾都读最新切片：本幕落的格必须在 `turn.facts` 里看得见。
    control.turn.facts = st.store.range(Seq::new(0));
    Ok(CloseEnd::Settled)
}

/// 归类与落格：提前了结（中止 / 授权拒）原样交回，正常收束落派生事实。
///
/// 单列一个函数只为一件事：`classify_turn` 借的是本轮状态里的召回视图，而它返回的
/// `Settled` 还带着那个借用——收在一处，借用在返回前结束。
async fn classify_and_persist(
    running: Running,
    recalled: Option<&super::super::recall::RecallView>,
) -> Result<super::ExecTurnResult, TurnExecExit> {
    match classify_turn(running, recalled).await? {
        super::Classified::Finished(v) => v,
        super::Classified::Settled(settled) => persist_turn(settled),
    }
}

/// 判成答话那一幕的**拆场**：不进收束，但桥要关、发射任务要收干。
///
/// 全部字段逐一拆开再 `drop`：`sink` / `dispatch` 也各持一份 `Arc<UiBridge>`，
/// 留着它们（`..` 的落点）会让「最后一个强引用」晚于 `emitter.await` 出现——
/// 发射端等一个永远不会关闭的队列，这一幕就卡在拆场里。
async fn abandon(st: StageRuntime) {
    let StageRuntime {
        draft,
        store,
        llm,
        // 用量记录是 `Copy` 值，不持任何句柄——拆出来即可，没有可释放的引用。
        tok: _,
        sink,
        tools,
        dispatch,
        host,
    } = st;
    drop(draft);
    drop(store);
    drop(llm);
    drop(sink);
    drop(tools);
    drop(dispatch);
    let Some(host) = host else {
        return;
    };
    let StageHost {
        assembled,
        emitter,
        report_landing,
        port,
    } = host;
    drop(port);
    drop(report_landing);
    // 最后一份桥随装配束一起走：`UiBridge::drop` 关队列，发射端排空后退出。
    drop(assembled);
    if let Err(e) = emitter.await {
        symbio_plugin_api::plugin_warn!(
            "session",
            "[Turn] 流式发射任务未正常退出（本幕已收束，继续）: {e}"
        );
    }
}

/// 宿主面 → 归类要的交付束（`D-222` 裁决 5：拆出来的东西全部 owned）。
fn into_running(
    assembled: Assembled,
    emitter: tokio::task::JoinHandle<()>,
    outcome: TurnOutcome,
    produced: Vec<ChatNode>,
    derived: DerivedFacts,
) -> Running {
    let Assembled {
        session,
        store,
        bridge,
        collected,
        turn_no,
        user_seq,
        principal,
        to,
        user_text,
        root_id,
        presentation_tree,
        ..
    } = assembled;
    Running {
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
        root_id,
        presentation_tree,
    }
}

// ── 小件 ────────────────────────────────────────────────────────────────

/// 本幕的幕锚（夹具没有「幕」这一层身份）。
fn current_drive_seq(driver: &Driver<'_>) -> Option<u64> {
    match driver {
        Driver::Control(control) => Some(control.turn.drive_seq),
        Driver::Ready(_) => None,
    }
}

fn driver_aborted(driver: &Driver<'_>) -> bool {
    match driver {
        Driver::Control(control) => control.turn.abort.is_aborted(),
        Driver::Ready(_) => false,
    }
}

/// 判决已判成「就此答话」（`head_answer` 由 `policy.classify` 的执行体寄存）。
fn driver_answered(driver: &Driver<'_>) -> bool {
    match driver {
        Driver::Control(control) => control.turn.head_answer.is_some(),
        Driver::Ready(_) => false,
    }
}

/// 停机收束：落兜底格且 `reason = exhausted`。
async fn exhausted(st: &mut StageRuntime, detail: &str) -> Result<TurnOutcome, TurnExecExit> {
    st.draft
        .exhausted(st.store.as_ref(), st.llm.as_ref(), detail)
        .await
        .map_err(append_exit)
}

/// 落格失败的统一出口（与 [`super::open_turn`] 的同一句话）。
fn append_exit(e: symbio_core::AppendError) -> TurnExecExit {
    TurnExecExit::Failed(PluginError::InternalError(format!("运行器落格失败：{e:?}")))
}

fn actor_exit(e: ActorError) -> TurnExecExit {
    match e {
        // 读失败：轮首那一片的会话端口读不出来，本轮没有可关的账。
        ActorError::Read(e) => TurnExecExit::Failed(e),
        ActorError::Append(e) => {
            TurnExecExit::Failed(PluginError::InternalError(format!("本幕落格失败：{e:?}")))
        }
    }
}

/// 开一幕失败（`begin_drive` / `continue_drive` 的出口）：失败原样，其余按内部错误。
fn to_exit(
    opened: Result<super::super::chat_loop::drive::Drive, TurnExit>,
) -> Result<super::super::chat_loop::drive::Drive, TurnExecExit> {
    opened.map_err(|exit| match exit {
        TurnExit::Failed(e) => TurnExecExit::Failed(e),
        other => TurnExecExit::Failed(PluginError::InternalError(format!(
            "开一幕交出了非失败结局：{other:?}"
        ))),
    })
}

fn no_stage() -> TurnExecExit {
    TurnExecExit::Failed(PluginError::InternalError(
        "环在幕外推进（按构造不可达）".into(),
    ))
}

fn no_host() -> TurnExecExit {
    TurnExecExit::Failed(PluginError::InternalError(
        "控制面的幕没有宿主面（按构造不可达）".into(),
    ))
}

/// 动作的类型在这里被点一次名：`StepFuture` 是绑定表与派发器之间唯一的返回通道，
/// 本文件不另立形状（`D-222` 裁决 5）。
const _: fn() = || {
    fn assert_type(_: Option<StepFuture<'static>>) {}
    let _ = assert_type;
};
