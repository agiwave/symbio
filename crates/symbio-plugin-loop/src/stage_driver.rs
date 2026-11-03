//! 阶段驱动器（`stage_driver`）：生产入口、外层环与一次债驱动的全部机制（P2-6a）。
//!
//! 本模块做三件事：
//!
//! 1. [`run_turn`]：装配会话端口与编排器，交给 [`chat_loop::run_chat_loop`] 做前步骤
//!    （请求解析、状态准备）与收尾（落库映射）；它不再决定「下一步是什么」。
//! 2. [`drive`]：按冻结行表选一笔债 → 交给对应行/端口受理 → 读账本判本幕收束
//!    （停机格 [`crate::turn_exec::ends_the_drive`] 或下一幕 [`crate::loop_rows::next_stage_round`]）。
//! 3. 幕首、收幕与各行实现（旧的 `turn_exec` 阶段模块）：本环只做**选择**与**收束判定**，
//!    不解释任何行名 / 端口名 / 业务语义（`D-118` 决策 2）。
//!
//! ## 幕的被动机制
//!
//! 轮首四行、接缝、汇报与轮内各行都是[一张绑定表](crate::loop_rows::ACTORS)上的行；
//! 幕与幕在**同一次调用**里接续——`continue-turn` 那一行由本环选中，它落下的 `round`
//! 格既是上一笔的出口、也是下一幕的锚（`D-222` 裁决 2）。
//!
//! ### 三段，一个循环体
//!
//! | 段 | 做什么 | 在哪里 |
//! |---|---|---|
//! | 幕首 | 上下文 → 开一幕（用户格 / 恢复格 / 轮次格）→ 纯读 → 定根 → 装配 → 草稿 | [`open_stage`]，环顶 |
//! | 幕内 | 选最小可还债 → 跑**一个**动作 → 读账本 → 五道守卫 | 循环体（标号 `'step`） |
//! | 收幕 | 归类 → 派生事实 → 镜像并入 → 定格 → 落库 → 封根 | [`close_stage`]，收束之后 |
//!
//! ### 五道守卫，各一份
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
//! ### 幕间中止不落第二格收束
//!
//! 首请求前的中止由运行器落中止收束格（`f-{turn}`）；**幕间**的中止不能再落一次
//! ——同一轮的第二格收束会撞幂等键。所以整次驱动只在第一圈问中止
//! （[`TurnSession::abort_before_first_request`]），此后每一圈的开头只判
//! [`DriveEnd::Boundary`]：上一幕已收束、下一幕不开。
//!
//! 生产入口 [`drive`] 只收 [`Control`]：编排层交下本轮状态，本函数一幕一幕地开、
//! 跑到全部幕收工。测试的帷幕入口 [`drive_prepared`] 交一截**已经开好的幕**，共用
//! **同一个**循环体与同一组守卫，差的只是「幕从哪来」与「收束后交给谁」。

use std::sync::{Arc, Mutex};

use symbio_kernel::adapters::LatencyTier;
use symbio_kernel::capability::CapabilityMeta;
use symbio_kernel::event::{Entity, Event, Seq, Verb};
use symbio_kernel::exec::ExecEventSink;
use symbio_kernel::llm::TurnOutput;
use symbio_kernel::plugin::{PluginError, PluginInvokeRequest};
use symbio_kernel::store::{AppendError, Store};
use symbio_plugin_api::events::{
    EVENT_CHAT_ASSISTANT_FALLBACK, EVENT_CHAT_ASSISTANT_FINAL, EVENT_USER_MESSAGE,
};
use symbio_sdk::adapters::{FullModel, LlmAdapter, TokenIssuer};
use symbio_sdk::chat::chat_node::{
    ChatNode, MessageContent, NodeKind, NodeStatus, CHAT_STYLE_TAG_REASONING,
};
use symbio_sdk::turn::dispatch::{run_ops, ConsolidateRequest, SessionView};

use crate::chat_loop;
use crate::chat_loop::state::{ChatOrchestrator, SessionContext, TurnExit, TurnRequest, TurnState};
use crate::derived_facts;
use crate::loop_rows;
use crate::report_exec::{ReportLanding, ReportPort};
use crate::session_port::{self, SessionPort};
use crate::tool_dispatch::{DerivedFacts, PortOutput, SessionDispatchPort, ToolDispatch};
use crate::turn_exec::{
    ActorError, Ctx, ExecTurn, FrameQueue, HeadParts, StepFuture, SupplementFn, UiBridge, UiFrame,
    UI_FRAME_CAPACITY,
};
use crate::turn_runner::{
    artifact_verdict, begin_turn, DialogFacts, NodeFactsFn, RoundInjector, StreamNodeFacts,
    TurnInput, TurnOutcome, TurnSession, TurnSessionBegin,
};

/// 生产入口：装配一轮的会话句柄与编排器，跑到收尾。
pub(crate) async fn run_turn(
    host: Arc<dyn PluginInvokeRequest>,
    dispatch: Arc<symbio_sdk::turn::dispatch::TurnDispatch>,
) -> symbio_sdk::turn::dispatch::TurnReport {
    let port: Arc<dyn SessionPort> = Arc::new(crate::session_port::PortSession::new(host.clone()));
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
    chat_loop::run_chat_loop(
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
    Settled(TurnOutcome),
    /// 判决判成「就此答话」。
    Answered,
}

/// 控制面交下来的那一片（编排层与本轮状态）。
///
/// 借的是调用方的 `&mut`：本函数每一圈把它们再借给一个动作（[`crate::turn_exec::HeadCtx`]），
/// 收工之后调用方还要读它们（收口与最后一条回复）。
pub(crate) struct Control<'a> {
    pub(crate) orchestrator: &'a ChatOrchestrator,
    pub(crate) orchestrator_arc: &'a Arc<ChatOrchestrator>,
    pub(crate) ctx: &'a Arc<dyn PluginInvokeRequest>,
    pub(crate) sink: &'a ExecEventSink,
    pub(crate) context: &'a mut SessionContext,
    pub(crate) turn: &'a mut TurnState,
    pub(crate) req: &'a TurnRequest,
    /// 本请求带的那条用户消息（首幕追加进上下文；续幕为 `None`）。
    pub(crate) single_message: Option<ChatNode>,
    /// 本次请求的恢复动作（`None` = 新开一轮）。
    pub(crate) resume: Option<symbio_sdk::chat::chat_node::ResumeRequest>,
}

/// 一次驱动的结局（流向不在这里：`D-222` 裁决 3）。
///
/// `Bare` / `Running` 只由测试的帷幕入口走到（生产恒 `Completed` / `Answered` /
/// `Boundary`）；非测试构建里它们按构造不可达，字段因此允许无人读取。
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum DriveEnd {
    /// 判决判成「就此答话」：这一幕一次请求都没发，收束格由编排层落。
    Answered,
    /// 幕间中止：上一幕已收束，下一幕没开（不落第二格收束）。
    Boundary,
    /// 控制面跑完了全部幕（收束格与派生事实都已落格）。
    Completed,
    /// 夹具路径：一幕的结局按值交回（没有宿主面可归类）。
    Bare(TurnOutcome),
    /// 夹具路径：幕有宿主面，收幕交出归类要的交付束（归类与落库由夹具自己做）。
    Running(Box<Running>),
}

/// 一幕的运行时：草稿 + 驱动它要的端口 + 收束要的宿主面。
pub(crate) struct StageRuntime {
    pub(crate) draft: TurnSession,
    pub(crate) store: Arc<dyn Store<Event = Event>>,
    pub(crate) llm: Arc<dyn LlmAdapter>,
    pub(crate) tok: FullModel,
    pub(crate) sink: Arc<dyn symbio_sdk::adapters::DeltaSink>,
    pub(crate) tools: Arc<[CapabilityMeta]>,
    pub(crate) dispatch: Option<Arc<dyn ToolDispatch>>,
    /// 生产：收束要的宿主面。夹具为 `None`（收束时按值交回结局）。
    pub(crate) host: Option<StageHost>,
}

/// 幕的宿主面（装配束 + 发射任务 + 汇报落地面 + 分发方本体）。
///
/// 四样都在收幕时被拆开消费：[`StageHost::port`] 的两个出参要先取走（产出 / 派生
/// 事实），桥要随装配束一起丢（发射端退出的唯一信号），发射任务要收干。
pub(crate) struct StageHost {
    assembled: Assembled,
    emitter: tokio::task::JoinHandle<()>,
    report_landing: ReportLanding,
    port: Arc<SessionDispatchPort>,
}

impl StageRuntime {
    /// 轮首那一片的**owned 半边**：桥 / 发射口 / 根 id / 主体 / 汇报面全从宿主面克隆。
    ///
    /// 写在幕上而不是每个调用点各拼一份：这两组句柄只该有一个来源（装配束），
    /// 拼第二份就是拼第二个读数点。`context` / `turn` / `req` 仍是借的——它们每圈
    /// 要重新借（[`HeadParts::as_ctx`]），而且出借方是调用方。
    pub(crate) fn head_parts<'h>(
        &self,
        orchestrator: Arc<ChatOrchestrator>,
        request_ctx: Arc<dyn PluginInvokeRequest>,
        context: &'h mut SessionContext,
        turn: &'h mut TurnState,
        req: &'h TurnRequest,
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
            system_prompt: host.assembled.system_prompt.clone(),
            actor: host.assembled.actor.clone(),
            report_port: host.assembled.report.clone(),
            report_landing: host.report_landing.clone(),
        })
    }
}

/// 收幕的两条去向：夹具交回结局（或归类的输入），控制面继续接幕。
pub(crate) enum CloseEnd {
    Bare(TurnOutcome),
    Running(Box<Running>),
    Settled,
}

/// 环的驱动状态：生产只有 [`Control`]；测试的帷幕入口只带一截已开好的幕。
///
/// 生产构造永远把 `stage` / `head` / `request_ctx` 留空（幕由 [`open_stage`] 每幕
/// 现开）；[`drive_prepared`] 只填后三项、`control` 留空。两路共用同一个循环体，
/// 差的只是「幕从哪来」与「收束后交给谁」。
struct DriveState<'a> {
    control: Option<Control<'a>>,
    /// 已开好的幕（测试帷幕入口；生产为 `None`）。
    stage: Option<StageRuntime>,
    /// 轮首那一片（测试帷幕入口；生产每圈现借）。
    head: Option<HeadParts<'a>>,
    /// 裸幕夹具的请求句柄（`head = None` 时创建 / 授权行要它判风险档）。
    request_ctx: Option<Arc<dyn PluginInvokeRequest>>,
}

impl DriveState<'_> {
    fn drive_seq(&self) -> Option<u64> {
        self.control.as_ref().map(|control| control.turn.drive_seq)
    }

    fn aborted(&self) -> bool {
        self.control
            .as_ref()
            .is_some_and(|control| control.turn.abort.is_aborted())
    }

    /// 判决已判成「就此答话」（读 `policy.classify` 落下的决定格）。
    fn answered(&self) -> bool {
        self.control.as_ref().is_some_and(|control| {
            crate::stage_events::answered(
                &control.context.facts_snapshot(),
                control.turn.turn_no,
                control.turn.drive_seq,
            )
        })
    }
}

pub(crate) async fn drive(
    control: Control<'_>,
    actors: &'static [loop_rows::ActorEntry],
) -> Result<DriveEnd, TurnExecExit> {
    run_loop(
        DriveState {
            control: Some(control),
            stage: None,
            head: None,
            request_ctx: None,
        },
        actors,
    )
    .await
}

/// 测试专用入口：驱动一幕**已经开好**的幕（`begin` + 基线 + 前缀都已填）。
///
/// 两个夹具各自能造出不同的幕：有宿主面时（`stage_runtime` 按生产的幕首装配），
/// 收幕把归类的输入按值交回（[`DriveEnd::Running`]）；没有宿主面时（夹具只造草稿
/// 与端口），收幕只交回结局（[`DriveEnd::Bare`]）。两条都**不做**归类、镜像并入与
/// 落库——那是控制面的收尾，夹具自己按要验的判据决定做多少。
#[cfg(test)]
pub(crate) async fn drive_prepared(
    stage: StageRuntime,
    head: Option<HeadParts<'_>>,
    request_ctx: Option<Arc<dyn PluginInvokeRequest>>,
    actors: &'static [loop_rows::ActorEntry],
) -> Result<DriveEnd, TurnExecExit> {
    run_loop(
        DriveState {
            control: None,
            stage: Some(stage),
            head,
            request_ctx,
        },
        actors,
    )
    .await
}

async fn run_loop<'a>(
    mut state: DriveState<'a>,
    actors: &'static [loop_rows::ActorEntry],
) -> Result<DriveEnd, TurnExecExit> {
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
        if retiring.is_some() && state.aborted() {
            return Ok(DriveEnd::Boundary);
        }
        // ── 幕间：旧幕让位 ───────────────────────────────────────────────
        //
        // 判据是幕锚变了：`continue-turn` 落地之后 `turn.drive_seq` 指向新落的
        // `round` 格，与已收束那一幕的锚不同 ⇒ 旧幕（只剩一个空壳草稿）可以丢了。
        if let Some(seq) = retiring {
            if state.drive_seq() != Some(seq) {
                stage = None;
                retiring = None;
            }
        }
        // ── 幕首：没有幕就开一幕（`D-222` 订正 3：草稿建在幕首）───────────
        if stage.is_none() {
            stage = Some(match state.stage.take() {
                Some(ready) => ready,
                None => {
                    let control = state.control.as_mut().ok_or_else(no_stage)?;
                    open_stage(control, first_stage).await?
                }
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
            if state.answered() {
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
                && st.draft.over_step_limit(&facts)
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
            if let Some(closing) = ledger
                .iter()
                .find(|e| crate::turn_exec::ends_the_drive(e, turn_no))
            {
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
                let seq = state.drive_seq();
                let mut st = stage.take().ok_or_else(no_stage)?;
                match close_stage(state.control.as_mut(), &mut st, outcome).await? {
                    // 夹具没有宿主面：结局按值交回，归类与落库由夹具自己做。
                    CloseEnd::Bare(outcome) => return Ok(DriveEnd::Bare(outcome)),
                    // 夹具自带宿主面：交回归类的输入（`Running`），后续同样归夹具。
                    CloseEnd::Running(running) => return Ok(DriveEnd::Running(running)),
                    CloseEnd::Settled => {
                        // 控制面：本幕收束了。还开不开下一幕读事实，不读返回值
                        // （`D-222` 裁决 2 / 裁决 3）。
                        let Some(control) = state.control.as_mut() else {
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
pub(crate) async fn open_stage(
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
        resume,
    } = control;
    // ── 幕首 ①：本轮上下文（窗口生效；心跳可关历史）──────────────────────
    //
    // 每幕重新读：上一幕的原生产出已在收幕时并入落库镜像，这一幕的 prompt 要从
    // 那份镜像出（事实源是同一份，视图每幕刷新）。
    context.messages = if req.load_history {
        session_port::call::<Vec<ChatNode>>(
            context.session.as_ref(),
            symbio_sdk::turn::dispatch::run_ops::CONTEXT_MESSAGES,
            (),
        )
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
    // 幕锚交给 `open-turn` / `resume-turn` 两行（D-313 P2-1）：驱动只选行、收
    // [`Opened`]，不按分支写事实。续幕不在这里落格——它那一条已由 `continue-turn`
    // 的执行体落过。
    let mut opened = chat_loop::open_turn::Opened::default();
    if first_stage {
        let rows = chat_loop::open_turn::select(resume.as_ref(), true);
        for row in rows {
            let mut octx = chat_loop::open_turn::OpeningCtx {
                context,
                turn,
                req,
                session: &context.session,
                request_ctx: ctx,
                resume: resume.as_ref(),
                out: &mut opened,
            };
            (row.action)(&mut octx).await.map_err(to_exit)?;
            symbio_plugin_api::plugin_info!(
                "session",
                "[Turn] 幕锚行 {} 落格 [{}] (turn={})",
                row.name,
                row.writes.join(","),
                opened.root_id.as_deref().unwrap_or("-")
            );
        }
    }
    let user_text = if first_stage {
        opened.user_text
    } else {
        // 本幕的锚与轮次都在 `turn` 上（`continue-turn` 的执行体刚写完）；用户发言
        // 从同一份镜像取（与首幕同一个判据：最近一条用户消息）。
        chat_loop::open_turn::current_user_text(context)
    };
    symbio_plugin_api::plugin_info!(
        "session",
        "[Turn] 第 {} 次请求开始 (msgs={})",
        turn.round() + 1,
        context.messages.len()
    );

    // ── 幕首 ③：环前纯读（提示词 / 工具 / 开销）────────────────────────────
    let reads = chat_loop::inputs::collect_reads(ctx, req).await;

    // ── 幕首 ④：定根（一幕一个根）────────────────────────────────────────
    //
    // 续写幕复用被续写轮的根（骨架已经发过，再发会把已完成的卡打回运行态）；
    // 新幕现分配。「定 id」在幕首（`begin` 的输入），「把骨架发给流式桥」仍在
    // 接缝行（`prepare-turn`，压缩之后：压不动时不该留半截 Turn 根）。
    let (root_id, fresh_root) = match opened.root_id.take() {
        Some(root) => (root, opened.fresh_root),
        None => (symbio_sdk::llm::turn::llm_short_id(), true),
    };
    // 轮内注入：把本会话的收件箱抽干口交给工具循环，让「用户中途插话」
    // **折进本轮**。抽干住在 session（队列与配置都在那边），驱动器经端口取。
    let supplements: SupplementFn = {
        let session = context.session.clone();
        Arc::new(move || {
            let session = session.clone();
            let fut: std::pin::Pin<
                Box<
                    dyn std::future::Future<
                            Output = Option<symbio_sdk::turn::facts::DrainedSupplement>,
                        > + Send,
                >,
            > = Box::pin(async move {
                let supplements: Option<symbio_sdk::turn::facts::DrainedSupplement> =
                    session_port::call(
                        session.as_ref(),
                        symbio_sdk::turn::dispatch::run_ops::TAKE_SUPPLEMENTS,
                        (),
                    )
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
        }) as SupplementFn
    };
    // 轮中汇报的能力口：**只产措辞**——判定与落格都不在这里（D-163）。
    // `None` = compose 产线关着：那一行到期时执行体交不回话，只落一格
    // `result:"silent"` 的尝试，这一段静默因此不再重复到期。
    let report = orchestrator.compose_enabled.then(|| {
        chat_loop::report_port(
            orchestrator_arc.clone(),
            ctx.clone(),
            context.messages.clone(),
        )
    });

    let (assembled, emitter) = assemble_turn(ExecTurn {
        session: context.session.clone(),
        view: context.view.clone(),
        facts: context.facts.clone(),
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
        window_turns: orchestrator.view.context_window,
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
/// `collect_reads` / 定根 / `open-turn` 那几行），但「分发方与汇报落地面怎么从
/// 装配束里造」只该有一处写法——照抄一遍就是第二个读数点。
pub(crate) fn stage_runtime(
    assembled: Assembled,
    emitter: tokio::task::JoinHandle<()>,
    open: OpenRun,
) -> StageRuntime {
    // 工具分发：契约与实现都在插件侧——它持插件宿主、请求上下文、转写出口与
    // 工具存档目录（core 认识这些即违 E-009）。
    // 产物闸在轮首判一次：只认 actor，结论（放行 + 拒绝理由）随分发方带给执行体。
    let verdict = artifact_verdict(&assembled.actor);
    let port = Arc::new(SessionDispatchPort::new(
        assembled.parent.clone(),
        assembled.ctx.clone(),
        assembled.sink.clone(),
        assembled.view.tool_archive_dir.clone(),
        assembled.store.clone(),
        assembled.abort.clone(),
        assembled.root_id.clone(),
        assembled.turn_no,
        assembled.user_seq,
        assembled.drive_seq,
        assembled.actor.clone(),
        verdict.allowed,
        verdict.denied_reason,
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
pub(crate) async fn close_stage(
    control: Option<&mut Control<'_>>,
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
    let Some(control) = control else {
        return Ok(CloseEnd::Running(Box::new(running)));
    };
    // 归类（中止 / 授权拒 / 正常）与派生事实落格。召回读数从 `policy.recall`
    // 落下的那一格重建（DE-048 D-306），不再寄存于本轮状态。
    let facts = control.context.facts_snapshot();
    let recall = crate::stage_events::recall_of(
        &facts,
        control.turn.turn_no,
        control.turn.drive_seq,
    )
    .map(|record| symbio_plugin_api::retrieval::Retrieval {
        degraded: record.degraded,
        entries: record
            .entries
            .into_iter()
            .map(
                |(seq, tag, content)| symbio_plugin_api::retrieval::Retrieved { seq, content, tag },
            )
            .collect(),
    });
    let res = classify_and_persist(running, recall.as_ref()).await?;
    // 收口：镜像并入（v2 原生产生的消息）→ 中止复查 → 定格 → 落库 → 封根。
    //
    // 次序与 `stage` 拆出去之前逐字一致：`finalize_turn_root` 必须晚于定格（Turn 是
    // 组合节点，终态跟随子树），而落库是收束的写投影。
    let session = control.context.session.clone();
    chat_loop::merge_runner_messages(&session, control.context, control.turn, res.messages)
        .await
        .map_err(TurnExecExit::Failed)?;
    if control.turn.abort.is_aborted() {
        // 请求结束后才置位的 abort 同样向上冒泡。本轮已完整产出（镜像已并入、随
        // 落库写入），没有半截要补 ⇒ `partial` 空。
        return Err(TurnExecExit::Aborted(Vec::new()));
    }
    let finish = chat_loop::settle_reasoning(
        control.orchestrator,
        control.context,
        control.sink,
        &root_id,
        res.output,
    )
    .await;
    chat_loop::persist_turn(control.sink, control.context, control.turn, finish).await;
    chat_loop::finalize_turn_root(control.sink, control.context, &root_id).await;
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
    recalled: Option<&symbio_plugin_api::retrieval::Retrieval>,
) -> Result<ExecTurnResult, TurnExecExit> {
    match classify_turn(running, recalled).await? {
        Classified::Finished(v) => v,
        Classified::Settled(settled) => persist_turn(settled),
    }
}

/// 判成答话那一幕的**拆场**：不进收束，但桥要关、发射任务要收干。
///
/// 全部字段逐一拆开再 `drop`：`sink` / `dispatch` 也各持一份 `Arc<UiBridge>`，
/// 留着它们（`..` 的落点）会让「最后一个强引用」晚于 `emitter.await` 出现——
/// 发射端等一个永远不会关闭的队列，这一幕就卡在拆场里。
pub(crate) async fn abandon(st: StageRuntime) {
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
        view,
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
        view: Arc::new(view),
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

/// 停机收束：落兜底格且 `reason = exhausted`。
pub(crate) async fn exhausted(
    st: &mut StageRuntime,
    detail: &str,
) -> Result<TurnOutcome, TurnExecExit> {
    st.draft
        .exhausted(st.store.as_ref(), st.llm.as_ref(), detail)
        .await
        .map_err(append_exit)
}

/// 落格失败的统一出口（与 [`open_turn`] 的同一句话）。
pub(crate) fn append_exit(e: AppendError) -> TurnExecExit {
    TurnExecExit::Failed(PluginError::InternalError(format!("运行器落格失败：{e:?}")))
}

pub(crate) fn actor_exit(e: ActorError) -> TurnExecExit {
    match e {
        // 读失败：轮首那一片的会话端口读不出来，本轮没有可关的账。
        ActorError::Read(e) => TurnExecExit::Failed(e),
        ActorError::Append(e) => {
            TurnExecExit::Failed(PluginError::InternalError(format!("本幕落格失败：{e:?}")))
        }
    }
}

/// 开一幕失败（`open-turn` / `resume-turn` 两行的出口）：失败原样，其余按内部错误。
fn to_exit(err: TurnExit) -> TurnExecExit {
    match err {
        TurnExit::Failed(e) => TurnExecExit::Failed(e),
        other => TurnExecExit::Failed(PluginError::InternalError(format!(
            "开一幕交出了非失败结局：{other:?}"
        ))),
    }
}

pub(crate) fn no_stage() -> TurnExecExit {
    TurnExecExit::Failed(PluginError::InternalError(
        "环在幕外推进（按构造不可达）".into(),
    ))
}

pub(crate) fn no_host() -> TurnExecExit {
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
// ── 装配 ────────────────────────────────────────────────────────────────

/// 一幕的**静态面**：装配段读出/构造、且整幕还要用的一切按值带走。
///
/// 与 [`ExecTurn`] 同族（捆包而非长参数表）。刻意把「①用过但幕内每一步都要用」的
/// 字段一并带上——回头重取就是第二个读数点，正是本拆分要避免的形状。
///
/// **全部 owned**（`D-222` 裁决 5 的后果）：控制面只有**一个**环，幕与幕在同一次
/// 调用里接续，而每一幕的装配输入（读到的提示词 / 现分配的根 id / 用户发言）都是
/// 那一幕的局部变量。借来的话，它们活不到环把它交给下一个动作的那一刻。
pub(crate) struct Assembled {
    // —— 交付包直通（幕内每一步消费）——
    pub(crate) session: Arc<dyn SessionPort>,
    /// 会话配置八格（`session-config` 回体的同一个投影）。
    pub(crate) view: SessionView,
    pub(crate) parent: Option<Arc<dyn symbio_kernel::plugin::Plugin>>,
    pub(crate) ctx: Arc<dyn PluginInvokeRequest>,
    pub(crate) system_prompt: String,
    pub(crate) abort: symbio_kernel::exec::ExecAbortSignal,
    pub(crate) root_id: String,
    /// 一幕一个根：`true` = 本幕现分配的根（发骨架），`false` = 复用被续写轮的根。
    pub(crate) fresh_root: bool,
    pub(crate) sink: symbio_kernel::exec::ExecEventSink,
    pub(crate) user_text: String,
    pub(crate) window_turns: u64,
    pub(crate) tools: Arc<[CapabilityMeta]>,
    pub(crate) supplements: Option<SupplementFn>,
    pub(crate) report: Option<ReportPort>,
    pub(crate) supplemental_no: Arc<std::sync::atomic::AtomicU64>,
    pub(crate) presentation_tree: symbio_sdk::turn::node_build::PresentationTree,
    // —— 装配的读数与构造 ——
    pub(crate) store: Arc<dyn Store<Event = Event>>,
    pub(crate) turn_no: u64,
    pub(crate) user_seq: u64,
    pub(crate) drive_seq: u64,
    pub(crate) collected: Arc<std::sync::Mutex<Vec<ChatNode>>>,
    pub(crate) bridge: Arc<UiBridge>,
    pub(crate) actor: symbio_kernel::actors::ActorSpec,
    pub(crate) principal: String,
    pub(crate) to: String,
}

/// **压缩入格**：轮首行 `policy.compress` 寄存的合并结果落成 `session.consolidated`。
///
/// 单列一步而不是并进 [`land_seam`]：它的前置只有「压缩跑完了」，而接缝的其余部分
/// 还要读判决 / 召回。环把它们排成相邻两步（压缩行的出口 ⇒ 接缝行的入口），各是
/// 一行的事实，不再由这一侧的记忆替环排序。
///
/// 落格失败只出声不升级：「压缩已生效」——这一步关系到的是事实面是否记下它，
/// 记不下也不该让整轮失败。
pub(crate) async fn land_consolidation(
    session: &dyn SessionPort,
    consolidation: Option<&[ChatNode]>,
) {
    // 必须在派发格**之前**：本轮的用户格已由编排层先落，先归档才能让「本轮之前的
    // 一切」被完整取代（否则快照与它后面的新事件之间会夹着被取代的旧事件，现行窗口
    // 不再自洽）。
    //
    // 归档移走前缀**不影响**续写锚：轮的「已开未收束」判据读完整史（D-085），
    // 等待轮被压进归档段后仍然成立——而且续写锚此刻已经由编排层按 seq 直给。
    let Some(msgs) = consolidation else {
        return;
    };
    if let Err(e) = session_port::call_unit(
        session,
        run_ops::CONSOLIDATE,
        ConsolidateRequest {
            messages: msgs.to_vec(),
        },
    )
    .await
    {
        symbio_plugin_api::plugin_warn!("session", "[Turn] 压缩事实落格失败（压缩已生效）: {e}");
    }
}

/// **接缝**：轮首四行跑完、任何请求之前（`D-223` 订正 1 / 订正 2）。
///
/// 三件事都是「轮首那几行跑完才知道」的，因此都不在幕首段（[`assemble_turn`]）里：
///
/// | 动作 | 为什么要等那几行 | 落成什么 |
/// |---|---|---|
/// | 定档 | 折自宿主代落的 `stage:"reflex"` 格（`turn_head.reflex_answered`） | `turn.dispatch`（`tier` / `budget_ms` / `principal`） |
/// | 首响上线 | 判决行 `Escalate` 的那句话 | `chat.assistant.opening` + 一帧实时面 |
/// | （压缩入格） | 同为本幕的前置动作，见 [`land_consolidation`] | `session.consolidated` + 归档 |
///
/// 次序是硬的：压缩早于派发（`consolidation_is_forwarded_before_the_turn_runs`
/// 钉着），首响早于基线 prompt 的渲染（渲染在同一动作的下一步）。
///
/// ## 为什么不在这里渲染 prompt
///
/// 渲染要往草稿里写（`TurnSession::render_base`），而草稿在本函数调用点的
/// [`Ctx::session`] 上——本函数只收一个会话动作面（它只需要 `port=session` 的行）。
/// 渲染因此留在执行体里、紧随本函数；「压缩先落、prompt 后渲染」由同一个动作的
/// 行序保证，不需要把草稿传进来。
pub(crate) async fn land_seam(
    store: &dyn Store<Event = Event>,
    bridge: &UiBridge,
    turn: &TurnState,
    actor: &symbio_kernel::actors::ActorSpec,
    opening_text: Option<&str>,
    opening_dialog: Option<&DialogFacts>,
) -> Result<(), ActorError> {
    // ── 定档：折自快路那一行的出口格（`D-223` 订正 1）─────────────────────
    //
    // 读**视图**而不是执行侧的副本（`reflex.is_some()`）：视图与那一行的判定同源，
    // 而「命中集为空、行自己关账」那一支（`result = miss`）只有视图分得出来。
    let facts_now = store.range(Seq::new(0));
    let now = loop_rows::wake_now_ms(&facts_now, turn.drive_seq);
    let tier = loop_rows::turn_tier(&facts_now, turn.turn_no, now);
    chat_loop::land_dispatch(
        store,
        turn.turn_no,
        turn.user_seq,
        turn.drive_seq,
        tier,
        actor,
    )
    .map_err(ActorError::Append)?;
    // ── 首响落格 + 上线 ──────────────────────────────────────────────────
    chat_loop::land_opening(
        store,
        bridge,
        turn.turn_no,
        turn.user_seq,
        &actor.principal,
        opening_text,
        opening_dialog,
    )
    .map_err(ActorError::Append)?;
    Ok(())
}

/// 一幕的幕首：装配，产 [`Assembled`] 与流式桥的发射任务。
///
/// 发射任务的句柄**按值交回**而不是塞进 [`Assembled`]：它在幕末要被
/// `classify_turn` 收干（见那一处），而 [`Assembled`] 要活到整幕收完——两样东西的
/// 寿命不同，混在一个结构里就只能靠 `Option` 表达「已经取走了」。
pub(crate) async fn assemble_turn(
    req: ExecTurn<'_>,
) -> Result<(Assembled, tokio::task::JoinHandle<()>), TurnExecExit> {
    let ExecTurn {
        session,
        view,
        facts,
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
        fresh_root,
    } = req;
    // 事实源：本会话的**唯一句柄**（由交付包按值带来、按引用分发）。
    // 与运行器落格、收件箱补落、轮外压缩进的是同一份实例——进程内不存在
    // 第二个各自 replay 的写者（D-084 §2）。
    let store = facts
        .ok_or_else(|| PluginError::InternalError("执行需要持久会话（临时会话无事实源）".into()))?;
    // **补落寄存的变更**：跨进程另一写者持令牌时，本进程的编辑 / 截断
    // 会得到 `NotTheWriter` ⇒ 该变更被寄存在会话侧。本函数是持有交付包的时刻，
    // 经端口动作让 session 在**它自己的令牌窗口**里补落。
    //
    // 位置在运行器落格之前：补落改的就是运行器即将读的那份切片（prompt 窗口 /
    // 产物编号），不先落完，本轮会站在一份**缺了那次编辑**的状态上做判断。
    if let Err(e) = session_port::call_unit(session.as_ref(), run_ops::DRAIN_PARKED, ()).await {
        symbio_plugin_api::plugin_warn!("session", "[Turn] 补落寄存变更失败（本轮继续）: {e}");
    }

    // 一次性阶段**不在这里落**：环上的每一行（判决 / 召回 / 压缩）由它自己的执行体
    // 当场入日志（D-159 裁决 4）——装配段不是任何一行的出口。
    let principal = symbio_sdk::authz::request_principal(ctx.as_ref());
    // 节点身份的同一份主体：流式帧与落库副本必须说同一个人（D-147）。
    let session_principal = symbio_sdk::chat::principal::Principal::from_legacy(&principal);

    // 轮号与溯源锚由编排层在判决之前定下：用户格先落，这里只收三个
    // seq，不再从切片反推。
    // - 新开轮：`turn_no` = 第几条用户格，`user_seq` = 该格的 seq；
    // - 唤醒幕：`turn_no` = **已开未收束**的那一轮（审批 / 问答恢复），`user_seq`
    //   是它的原用户格 seq——唤醒**不新开用户格**，收束仍记在该轮上（C4 按 turn
    //   配对，另开新轮会把原轮变成永久假缺口）。

    // 折进的补充消息的收集器：注入闭包（`'static`）往它 push，
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
                    symbio_sdk::turn::node_build::detach_if_flat(
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
                    symbio_sdk::turn::node_build::detach_if_flat(
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
    // 技能行作答后宿主代落的 `stage:"reflex"` 格（`D-260`），开场白是判决行
    // `Escalate` 的首响——两者都要等轮首那几行跑完。落点是接缝（[`land_seam`]），由 [`drive`] 在 `begin` 之后调。
    //
    // 本段（幕首）只做「不需要等那几行」的事：取事实源、补落寄存变更、建收集器与
    // 流式桥、定主体与来源主体。
    //
    // 身份进事件：本轮以**会话选定的那个 agent** 为主体，规格从
    // **授权表行**取（`request_actor_spec`）——能力集与模式不再是 `trivial` 的空表
    // 平凡值，它们与写侧闸判的是同一张表（判的对象 = 写的对象 = 表授予的对象）。
    // 未选智能体 ⇒ `agent:main`（DE-038 §4 平凡值，与接线前逐字一致）。
    //
    // 主体**不吃档位**（`D-223` 订正 2）：它要在幕首就建（`D-222` 订正 3），而档位要
    // 等轮首四行跑完——两者互相派生就成环。规格里的 `budget_ms` 是主体的**声明档**，
    // 本幕的预算归接缝上那一格 `turn.dispatch`。
    //
    // `principal` 已在落阶段事实时取过（同一取值点 `request_principal`）：
    // `actor` 随 `input` 被移动进运行器，轮末的记忆与学习写方（`record_learning`）
    // 仍要以它为主体——先取字符串，不依赖 `actor` 的存活。
    // 作用域：`agent_run` 派生的子会话挂父主体，顶层会话恒 `Root`。
    // 与身份同批取——两者都经 `ctx` / 元数据解析，分开取会让「主体是 A、作用域按 B 算」。
    let scope = view.scope.clone();
    let actor = chat_loop::request_actor_spec(ctx.as_ref(), scope);
    // 来源主体（D-067）：`commitment.*` 的 `to`。在此刻取而不是轮末——`ctx` 会被
    // 移进工具分发口（`SessionDispatchPort`），轮末已经拿不到它。
    let to = chat_loop::request_source_principal(ctx.as_ref());

    Ok((
        Assembled {
            session: session.clone(),
            view: view.clone(),
            parent,
            ctx,
            system_prompt: system_prompt.to_string(),
            abort,
            root_id: root_id.to_string(),
            fresh_root,
            sink: sink.clone(),
            user_text: user_text.to_string(),
            window_turns,
            tools: Arc::from(tools),
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
            actor,
            principal,
            to,
        },
        emitter,
    ))
}

/// **已开未收束**的轮次（`turn × opened` 无同号收束）：返回 `(turn, 用户格 seq)`。
///
/// ## 输入必须是完整史
///
/// 调用方传 `range(0)`（含归档前缀），**不是**现行窗口（[D-085](../../symbio-plugin-session/README.md#d-085-压缩取代的是消息列表不是轮次生命周期恢复锚读完整史归档刀口不动 "crates/symbio-plugin-session/src/README")）：
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
// ── 开轮（幕首建草稿）────────────────────────────────────────────────────

/// 收幕 ⇒ 归类 / 落库的交付束：调用结果与桥、发射任务、落库还要用的一切。
///
/// **全部 owned**（`D-222` 裁决 5）：它是由幕的运行态（[`StageRuntime`]）拆出来的，
/// 拆完那一幕就没了——借来的引用没有出借方可以活到归类。
///
/// `bridge` 与 `emitter` 按值走：收幕之后 `bridge.close()` 是发射端退出的**唯一**
/// 信号，必须正好剩最后一个强引用——所以幕内所有 `bridge.clone()` 都随注入闭包与
/// 分发方在收幕时死去，最后一个原样搬进这里、由归类消费。
pub(crate) struct Running {
    /// 会话配置八格（`session-config` 回体的同一个投影）：落库只需要身份一格。
    pub(crate) view: Arc<SessionView>,
    pub(crate) store: Arc<dyn Store<Event = Event>>,
    pub(crate) bridge: Arc<UiBridge>,
    pub(crate) emitter: tokio::task::JoinHandle<()>,
    pub(crate) outcome: TurnOutcome,
    pub(crate) produced: Vec<ChatNode>,
    pub(crate) derived: DerivedFacts,
    pub(crate) collected: Arc<std::sync::Mutex<Vec<ChatNode>>>,
    pub(crate) turn_no: u64,
    pub(crate) user_seq: u64,
    pub(crate) principal: String,
    pub(crate) to: String,
    pub(crate) user_text: String,
    pub(crate) root_id: String,
    pub(crate) presentation_tree: symbio_sdk::turn::node_build::PresentationTree,
}

/// [`open_turn`] 的出口：草稿与驱动它要的端口。
///
/// `elimination_revoked` 在开轮前取一次（与接线前同一时刻）：淘汰闸的部署级判据由
/// session 折叠成**一个值**（`D-086 §4`：全量事件过墙是「值注入」的反面）。
pub(crate) struct OpenRun {
    pub(crate) session: TurnSession,
    pub(crate) llm: Arc<dyn LlmAdapter>,
    pub(crate) tok: FullModel,
    pub(crate) elimination_revoked: bool,
}

/// 幕首开轮（[`TurnRunner::begin`]）：只建草稿，不落任何「要等轮首那几行」的格。
///
/// 首请求前的中止由派发器的第一圈守卫接住（[`run_loop`] 的
/// `abort_before_first_request`，落中止收束格、交回 [`DriveEnd`]）——它排在任何一行
/// 轮首行**之前**，所以被中止的幕不再跑判决 / 压缩 / 召回（`D-222` 订正 3 认下的行为变化）。
///
/// 开轮即收束那一支（[`TurnSessionBegin::Done`]）在本路**按构造不可达**：
/// 它只由定稿答话轮（`final_reply`）触发，而本函数恒传 `None`。所以这里不给它
/// 一条安静的出口——真到了那里说明有人改坏了 `TurnInput`，按内部错误报出来。
pub(crate) async fn open_turn(a: &Assembled) -> Result<OpenRun, TurnExecExit> {
    // 深度档：完整推理（含工具内环）。快路的**判定**已经不在这一层：它由环上的
    // `skill.recall` 行（能力包）选中，出口格（`stage:"reflex"`）由宿主代落、正文由
    // 宿主经 `set_reflex` 寄进草稿；本侧只把正文交给收束动作，不自己落 `final`
    // （D-260 / D-262）。
    //
    // 模型面只按端口名取（D-298）：解析与适配器构造在提供者一侧。端口缺席在生产
    // 不可达（session 在进环之前已保证生效模型存在，限流 / 上下文上限那一排闸
    // 都读它）——真缺席就是装配错了，按内部错误响亮报出。
    let llm: Arc<dyn LlmAdapter> = match crate::port::model_port(&a.ctx)
        .await
        .and_then(|provider| provider.llm_round_trip(None, &a.system_prompt, a.abort.clone()))
    {
        Some(llm) => llm,
        None => {
            return Err(TurnExecExit::Failed(PluginError::InternalError(
                "`port.model` 缺席：没有可用的模型提供者（session 应在进环前保证生效模型）".into(),
            )))
        }
    };
    let tok = TokenIssuer::issue_deep();
    let input = TurnInput {
        turn: a.turn_no,
        user_seq: a.user_seq,
        drive_seq: a.drive_seq,
        window_turns: Some(a.window_turns),
        actor: a.actor.clone(),
        // 轮内注入：用户中途说的话折进**本轮**。
        //
        // core 只知道「加消息」——抽干、合并、**落格**全在这里（插件侧）。
        // core 不该知道「收件箱补充」这个概念（队列、`merge_supplements`、
        // `InboxItem` 都在 `crates/symbio-plugin-session`），而落格也不需要 core 参与：
        // 本函数自己就开着 `store`。
        //
        // 轮中汇报**不在这里**：批次边界那件事成了 `policy.report` 行，
        // 由同一个环按 `(phase, anchor)` 选中、由它自己的执行体落格上线（D-163）。闭包只留
        // 「抽干」这一件事——它仍然是 core 唯一的轮边界挂点。
        inject: injector(a.supplements.clone()).map(|drain| {
            let store = a.store.clone();
            let actor = a.actor.clone();
            let turn_no = a.turn_no;
            let supplemental_no = a.supplemental_no.clone();
            // 折进的消息往收集器 push（末尾随 `ExecTurnResult` 回灌进落库镜像）。
            let sink_msgs = a.collected.clone();
            Arc::new(
                move |anchor: Option<u64>| -> std::pin::Pin<
                    Box<
                        dyn std::future::Future<Output = Vec<symbio_kernel::llm::PromptMessage>>
                            + Send,
                    >,
                > {
                    // `async move` 块按值捕获，所以这里先各 clone 一份——
                    // 否则第一次调用就把捕获物搬走，闭包退化成 `FnOnce`。
                    let drain = drain.clone();
                    let store = store.clone();
                    let actor = actor.clone();
                    let sink_msgs = sink_msgs.clone();
                    let supplemental_no = supplemental_no.clone();
                    Box::pin(async move {
                        // 抽干口本身就是异步的（会话动作面），直接 await——
                        // 搬家前那套 `blocking_read` + `block_in_place` 的桥接
                        // 随队列一起留在了 session，这里不再需要判 runtime flavor。
                        let batch = drain().await;
                        supplement_into_request(
                            batch,
                            anchor,
                            store.as_ref(),
                            &actor,
                            turn_no,
                            &sink_msgs,
                            &supplemental_no,
                        )
                    })
                },
            ) as RoundInjector
        }),
        // 本路**恒**走生成：定稿答话轮不经这里——它一个模型请求都不发，
        // 由 `execute_final_reply_turn` 单独入口落格（见该函数）。
        final_reply: None,
        final_cost_ms: 0,
        // 本路恒走生成 ⇒ 没有对话面节点（定稿答话轮不走这里）。
        dialog: None,
        // 落格身份：本轮 Turn 根 id（载荷 `turn_id`）+ 流式节点取数口。用户格
        // 已由编排层先落，运行器只收它的 seq。取数口是**快照闭包**：
        // 运行器每轮收束 / 进展 / 兜底时问一次「此刻桥上攒着哪两个节点」，
        // 中途正文与思考因此逐轮入格（半截中止时它们是唯一幸存者）。
        turn_root_id: a.root_id.clone(),
        nodes: Some(Arc::new({
            let bridge = a.bridge.clone();
            move || {
                let (text_id, text) = bridge
                    .current_node()
                    .map(|(id, acc)| (Some(id), Some(acc)))
                    .unwrap_or((None, None));
                let (reasoning_id, reasoning) = bridge
                    .current_reasoning()
                    .map(|(id, acc)| (Some(id), Some(acc)))
                    .unwrap_or((None, None));
                StreamNodeFacts {
                    text_id,
                    text,
                    reasoning_id,
                    reasoning,
                }
            }
        }) as NodeFactsFn),
        // 中途中止信号（D-052 决策 1）：闸门与首请求之间的空隙由运行器
        // 接管——落完用户格、发第一个请求前检查，已中止则走中止收束格。
        abort: Some(a.abort.clone()),
    };
    // 淘汰闸的部署级判据：取失败按「未收回」继续并出声——与「读不出的 WAL 跳过」的
    // 既有口径一致。取值点与接线前一致：开轮之前（轮首那几行不参加这个判断）。
    let elimination_revoked =
        session_port::call::<bool>(a.session.as_ref(), run_ops::ELIMINATION_REVOKED, ())
            .await
            .unwrap_or_else(|e| {
                symbio_plugin_api::plugin_warn!(
                    "session",
                    "[Turn] 淘汰闸判据读取失败（按未收回处理，本轮继续）: {e}"
                );
                false
            });
    match begin_turn(a.store.as_ref(), input).await {
        // 落格失败与动作失败走**同一个出口**（`ActorError` 的语义，这里折成
        // `TurnExecExit::Failed`）：调用方只认这一条通道，不为「开轮」单开一条。
        Err(e) => Err(TurnExecExit::Failed(PluginError::InternalError(format!(
            "运行器落格失败：{e:?}"
        )))),
        Ok(TurnSessionBegin::Done(_)) => Err(TurnExecExit::Failed(PluginError::InternalError(
            "生成路径的开轮被判成「已收束」（定稿答话轮才走那一条）".into(),
        ))),
        Ok(TurnSessionBegin::Run(session)) => Ok(OpenRun {
            session,
            llm,
            tok,
            elimination_revoked,
        }),
    }
}

/// 轮边界注入的唯一挂点：抽干口（`None` = 本会话没装收件箱）。
///
/// 汇报不挂在这里：它是 `policy.report` 行（D-163），判定归行表、落格与上线归行
/// 自己的执行体。这里只剩「用户有没有中途插话」这一件 core 认得的事。
fn injector(supplements: Option<SupplementFn>) -> Option<SupplementFn> {
    supplements
}

/// 抽干结果 → **落格 + 构造注入消息**（轮内注入的全部逻辑，闭包之外）。
///
/// 为什么抽出来：这段逻辑内联在补充注入的闭包里已经 90 行，且一半是「抽干 / 落格 / 造节点」的直线代码。
/// 轮中汇报要往同一个闭包里再加一段逻辑，不抽出来的话那个闭包会奔到 200 行——
/// 而它的可读性靠的正是「一眼看到这一轮边界上依次发生了什么」。
///
/// 返回**空 vec** = 本轮无可注入（没有补充 / 正文为空 / 拿不到溯源锚点）。三种情形
/// 都**不消耗任何配额**、不留半截状态，与 `apply_verdict` 拿不到措辞时同一条方向。
#[allow(clippy::too_many_arguments)] // 抽出来之后参数就是它的全部依赖，多一个都该重新想
pub(crate) fn supplement_into_request(
    batch: Option<symbio_sdk::turn::facts::DrainedSupplement>,
    anchor: Option<u64>,
    store: &dyn Store<Event = Event>,
    actor: &symbio_kernel::actors::ActorSpec,
    turn_no: u64,
    sink_msgs: &Arc<std::sync::Mutex<Vec<ChatNode>>>,
    no_counter: &std::sync::atomic::AtomicU64,
) -> Vec<symbio_kernel::llm::PromptMessage> {
    let Some(drained) = batch else {
        return Vec::new();
    };
    let (text, count, ids) = (drained.text, drained.count, drained.ids);
    if text.trim().is_empty() {
        return Vec::new();
    }
    // **落格**：成为一格事实（`turn × asserted`）。不落格的话它只活在这一次请求里，
    // 下一轮又消失了——而「用户说过的话没有变成事实」正是轮内注入要消掉的失效本身。
    //
    // 事件 id 带序号（`s-{turn}-{no}`）：一次工具循环里可能折进多条，不带序号会撞
    // 幂等键（`Duplicate`）。
    //
    // 拿不到溯源锚点 ⇒ **不落也不注入**：I2 要求断言类事件带溯源，而无溯源的断言
    // 事件比不落更难查。
    let Some(anchor) = anchor else {
        symbio_plugin_api::plugin_warn!(
            "session",
            "[Turn] 补充无溯源锚点，本轮丢弃（见本函数「拿不到溯源锚点」处）"
        );
        return Vec::new();
    };
    let no = no_counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    if let Err(e) = store.append(
        symbio_kernel::event::Event::pending(
            format!("s-{turn_no}-{no}"),
            symbio_plugin_api::events::EVENT_TURN_SUPPLEMENTED,
            symbio_kernel::event::Entity::Turn,
            symbio_kernel::event::Verb::Asserted,
            turn_no,
            &actor.principal,
        )
        .with_produced_by(anchor)
        // 载荷带上**节点身份**（原则：节点身份是事实的一部分）：重建按 `id`
        // 复刻这条用户消息的身份、按 `ids` 复刻 `meta.supplement_ids`。缺了它们，
        // 重建只能合成 id（每次重开都换一批），且合并标记里的原始条目 id 无处可寻
        // ——而「合并后原始条目只此一处留痕」（见 `transcript::supplements` 的模块头）。
        .with_payload(serde_json::json!({
            "text": text,
            "count": count,
            "id": drained.id.clone(),
            "ids": ids.clone(),
        })),
    ) {
        // 落格失败**不静默**：消息仍会进这一次请求，但事实网格里没有它 ⇒ 下一轮起
        // 不可见。所以出声。
        //
        // `{:?}` 不是 `{e}`：`AppendError` 只 derive 了 `Debug`。
        symbio_plugin_api::plugin_warn!(
            "session",
            "[Turn] 补充落格失败（仍注入本轮，下一轮起不可见）: {e:?}"
        );
    }
    let msg = ChatNode {
        // 沿用第一条补充的 id（见 `DrainedSupplement::id` 的说明）
        id: drained.id.clone(),
        principal: Some(symbio_sdk::chat::principal::Principal::from_legacy(
            symbio_sdk::authz::PRINCIPAL_USER,
        )),
        content: Some(MessageContent::Text(text.clone())),
        meta: Some(serde_json::json!({
            symbio_sdk::turn::facts::META_SUPPLEMENT: true,
            symbio_sdk::turn::facts::META_SUPPLEMENT_COUNT: count,
            symbio_sdk::turn::facts::META_SUPPLEMENT_IDS: ids,
        })),
        ..Default::default()
    };
    // 锁拿不到就**跳过回灌**，不 panic：这条补充**已经落格、已经进了本轮请求**，
    // 用户看得见——丢的只是落库镜像里的一条（重启后少它）。为这个 panic 不值。
    if let Ok(mut sink) = sink_msgs.lock() {
        sink.push(msg);
    }
    vec![symbio_kernel::llm::PromptMessage {
        role: "user".into(),
        text,
        tool_call_id: None,
        tool: None,
        tool_calls: None,
    }]
}

// ── 结局归类 ────────────────────────────────────────────────────────────

/// 一轮 v2 原生产物的**全部出口**。
pub(crate) struct ExecTurnResult {
    /// 收口用产物（正文 / 节点 id）——`TurnOutput` 同形，收口步骤 5–7 只认这一份结构。
    pub output: TurnOutput,
    /// 本轮 v2 原生**额外产生**的消息（工具轮：助手中途正文 + ToolCall + 工具结果），
    /// 按产生顺序。chat_loop 把它们并入 `context.messages`——落库权威是那份
    /// 内存镜像（`persist_messages` 只写它的尾部增量），不并入就只看得见不落库。
    pub messages: Vec<ChatNode>,
}

/// 一幕执行的退出类型（[`run_loop`] 与 [`persist_turn`] 共用的那一份）。
///
/// 为什么不是裸 `PluginError`：中止出口要携带**半截产出**（`Vec<ChatNode>`），
/// 而那批节点是 chat 域词汇。`PluginError` 是跨插件共享的错误词汇（`model`
/// 插件也构造 `Aborted`）——它不搬这个箱子：`to_frame` 只写错误码与文案，
/// 模型侧的中止在 `adapters` 边界转成 `AdapterError::Aborted`（载荷在那里
/// 被丢掉），loop 侧由 [`classify_turn`] 见到 `outcome.aborted` 后**重新构造**
/// 一份。半截产出的唯一生产方与唯一消费方都在 loop 内、是一次普通的进程内
/// Rust 调用，型在两侧编译期可见；塞进共享词汇再拆箱（无论用不用 `Any`）
/// 都是给一条不跨墙的数据付跨墙的税（[D-095]）。
///
/// 决策 D-095 见 [D-095](../../../docs/003-design/005-协议规范.md#d-095-keys-域环拆解键与值同住aborted-载荷不跨域vdfs_change_of-归-event_bus "D-095")
#[derive(Debug)]
pub(crate) enum TurnExecExit {
    /// 用户中止且半截产出已在手（正文 / 思考 / 工具节点）——它们是事实，
    /// 不因没说完就不记（消费循环按 id 去重、整条覆盖、孤儿丢弃后入格）。
    Aborted(Vec<ChatNode>),
    /// 其余一切（模型失败 / 编排失败 / 内部错误）。
    Failed(PluginError),
}

impl From<PluginError> for TurnExecExit {
    fn from(e: PluginError) -> Self {
        // `Aborted` 一律走中止出口，不压成失败（D-044 同源纪律）：走到这里的
        // 中止没有半截产出可带（模型侧中止在 adapters 边界已转
        // `AdapterError::Aborted`，由 `outcome.aborted` 那条路重新构造）。
        match e {
            PluginError::Aborted => TurnExecExit::Aborted(Vec::new()),
            other => TurnExecExit::Failed(other),
        }
    }
}

/// 归类的出口：本轮已提前了结（[`Classified::Finished`]），或正常收束待落格
/// （[`Classified::Settled`]）。刻意收全三个提前出口于类型——落库段拿不到被跳过的
/// 结局，就不会出现「中止轮还去写派生事实」的顺序漂移。
pub(crate) enum Classified<'b> {
    Finished(Result<ExecTurnResult, TurnExecExit>),
    Settled(Settled<'b>),
}

/// 归类 → 落库的交付束：收束结局 + 桥上取下的节点身份 + 落库还要用的一切。
pub(crate) struct Settled<'b> {
    /// 会话配置八格（`session-config` 回体的同一个投影）：落库只需要身份一格。
    pub(crate) view: Arc<SessionView>,
    pub(crate) store: Arc<dyn Store<Event = Event>>,
    pub(crate) outcome: TurnOutcome,
    pub(crate) produced: Vec<ChatNode>,
    pub(crate) derived: DerivedFacts,
    pub(crate) collected: Arc<std::sync::Mutex<Vec<ChatNode>>>,
    pub(crate) turn_no: u64,
    pub(crate) user_seq: u64,
    pub(crate) principal: String,
    pub(crate) to: String,
    pub(crate) user_text: String,
    /// 本轮的召回视图（轮末 `record_learning` 的入参）。
    ///
    /// 借自调用方的本轮状态（[`crate::chat_loop::state::TurnState`]），
    /// 与 [`Running`] 的寿命无关：环收工之后调用方才交得出来（`D-222` 裁决 5 的
    /// 「按步借、按步还」——环里那份借用已经还回去了）。
    pub(crate) recalled: Option<&'b symbio_plugin_api::retrieval::Retrieval>,
    pub(crate) child_id: String,
    pub(crate) reasoning_child_id: String,
    pub(crate) reasoning_text: String,
}

/// 一轮的执行之三：结局归类——消费 [`Running`]，产 [`Classified`]。
pub(crate) async fn classify_turn<'b>(
    r: Running,
    recalled: Option<&'b symbio_plugin_api::retrieval::Retrieval>,
) -> Result<Classified<'b>, TurnExecExit> {
    let Running {
        view,
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
    } = r;
    // 本轮**未定格**的正文与推理节点必须在桥 drop 之前取走：它们的收束帧由
    // chat_loop 的 `finalize_assistant_turn` 发出（用 `TurnOutput` 里的这两个 id），
    // 而**落库**则由 `TurnOutput::into_messages` 按同一批 id 建节点。
    //
    // 工具轮的那几轮在 `tool_dispatch::dispatch` 里已经定格并取走了 ⇒ 这里拿到的是
    // **最后一轮**（无工具调用那一轮）的节点；没有推理就是 `None`（`on_reasoning`
    // 一次都没被调过），不是空节点。
    let (child_id, child_text) = bridge.current_node().unwrap_or_default();
    let (reasoning_child_id, reasoning_text) = bridge.current_reasoning().unwrap_or_default();
    // 通道关闭 ⇒ 发射端排空后退出。
    //
    // **显式关**而不是等 `drop(bridge)`：草稿里的 `nodes` 闭包捕获着同一份
    // `Arc<UiBridge>`，而草稿未必在本函数之前已释放（收幕那一侧才 drop 它）。
    // 只靠引用计数归零，这里就会等一个永不关闭的通道。
    bridge.close();
    drop(bridge);
    emitter
        .await
        .map_err(|e| PluginError::InternalError(format!("流式发射任务失败：{e}")))?;

    // 中止：收束格已由运行器落格（`f-{turn}`，载荷 `status: "aborted"` + `terminated`），
    // 出口走 `TurnExecExit::Aborted`——由 chat_loop/消费循环落库为 `NodeStatus::Aborted`
    // + 会话结局 `aborted`（**不是** `failed`）。
    // 记忆与学习也**不写**——中止的轮次没有定稿产出，没有可固化的东西。
    //
    // **半截产出随出口带走**：末轮正文 / 思考此刻还只攒在桥上，丢了它们
    // 用户中断前看到的那半句话就消失。消费循环把它收尾入格
    // （按 id 去重、整条覆盖、孤儿丢弃，见 `persist_failure`）；工具轮已定格的节点
    // （`produced`）同走这条通道——中止出口不再回灌 `ExecTurnResult`，不带上就没了。
    if outcome.aborted {
        let session_principal = symbio_sdk::chat::principal::Principal::from_legacy(&principal);
        let mut partial = produced;
        if !child_text.is_empty() {
            let mut n = symbio_sdk::turn::node_build::llm_build_text_node(
                &root_id,
                &child_text,
                None,
                child_id,
                &session_principal,
            );
            symbio_sdk::turn::node_build::detach_if_flat(&mut n, &root_id, presentation_tree);
            partial.push(n);
        }
        if !reasoning_text.trim().is_empty() {
            let mut n = ChatNode {
                id: reasoning_child_id,
                parent_id: Some(root_id.to_string()),
                principal: Some(session_principal.clone()),
                msg_type: Some(NodeKind::Notice),
                content: Some(MessageContent::Text(reasoning_text.clone())),
                status: Some(NodeStatus::Completed),
                timestamp: Some(symbio_sdk::clock::clock_now_ms()),
                ..Default::default()
            }
            .with_style_tag(CHAT_STYLE_TAG_REASONING);
            symbio_sdk::turn::node_build::detach_if_flat(&mut n, &root_id, presentation_tree);
            partial.push(n);
        }
        return Ok(Classified::Finished(Err(TurnExecExit::Aborted(partial))));
    }

    // ── 收束派生事实（两半）：承诺 / 任务表 + 记忆 ───────────────────────────
    //
    // **本步的存在理由**：轮次事实由运行器原生入格，这两半都**不是轮次事实**，而是
    // 本轮的**派生副作用**——运行器一处都不写，写方就是这里的 `record_derived` /
    // `record_learning`（溯源锚 = 运行器原生写的 `u-{turn}` 格）。不在这里补，
    // 代际立约 / 任务表 / 长期记忆
    // 就整体失效。熔断事实由执行体随债自己落（DE-039 §6），不在这两半里。
    // 技能编译与回退观测不住这里：它们是技能能力包的收束贡献
    // （`Contributor`，D-250 / D-262），由 `settle` 统一落格。
    //
    // 兜底收束的 `response` 为 `None`：兜底说明这条路没走通，固化它等于把失败写成
    // 套路。写侧闸拒（`closure_denied`，见运行器的 `closure_gate`）：收束格
    // **未入格** ⇒ 本轮的派生事实**也不写**（承诺 / 任务 / 记忆全不写）。
    // 用户的答案照旧经 `produced` 回给调用方——收束格是**记录**，不是呈现。
    if outcome.closure_denied {
        symbio_plugin_api::plugin_warn!(
            "session",
            "[turn-exec] 收束被授权拒绝，本轮派生事实不入格（turn={turn_no}）"
        );
        // 被拒的兜底仍是**失败**（I3）：呈现走 Failed 出口，与未拒时同形。
        if outcome.fell_back {
            return Ok(Classified::Finished(Err(TurnExecExit::Failed(
                PluginError::InternalError(outcome.text),
            ))));
        }
        return Ok(Classified::Finished(Ok(ExecTurnResult {
            output: stage_output(outcome, reasoning_text, child_id, reasoning_child_id),
            messages: produced,
        })));
    }
    Ok(Classified::Settled(Settled {
        view,
        store,
        outcome,
        produced,
        derived,
        collected,
        turn_no,
        user_seq,
        principal,
        to,
        user_text,
        recalled,
        child_id,
        reasoning_child_id,
        reasoning_text,
    }))
}

/// 归类 / 落库两处共用的 `TurnOutput` 构造。
///
/// 实测用量与末轮原因都是**直通**格：这一格若丢，校准比冻结在初值 1.0；换成
/// `..Default::default` 会把 finish 静默填成 Stop，长度截断与自动续写因此不可达。
/// 反射档 / 兜底 / 中止没有模型轮 ⇒ Stop 就是诚实值。
fn stage_output(
    outcome: TurnOutcome,
    reasoning: String,
    response_text_child_id: String,
    reasoning_child_id: String,
) -> TurnOutput {
    TurnOutput {
        text: outcome.text,
        reasoning,
        response_text_child_id,
        reasoning_child_id,
        // 实测用量直通：校准反馈的唯一数据源。
        usage: outcome.usage,
        // 末轮原因直通：见本函数说明。
        finish: outcome.finish,
        ..Default::default()
    }
}
// ── 落格 + 定稿答话轮 ────────────────────────────────────────────────────

/// 收束贡献（DE-025 D-250 / D-252）：逐个询问已链接的能力，按 `(prio, owner)` 落格。
///
/// 每问一个之前**重取一次史**：后一个贡献者要看得见前一个刚落格的草稿——编码先于
/// 巩固、任务表先于调度，这两条依赖都由这一行兑现，而不是由贡献者自己持状态。
///
/// 信封在这里盖：轮号、时刻与归属人之外的一层（`turn` / `ts`）都由写口填，能力只交
/// 词干与载荷（D-252）——「这笔账记在哪一轮」不是能力的判断。
///
/// 失败只记日志、不打断下一个：轮次已收束，附加事实失败不该把成功的一轮说成失败
/// （与效果 / 记忆三段同一条口径）。
///
/// 形参宽是这一族的固有形状：八个量都是本轮的**事实端口**（与
/// [`execute_final_reply_turn`] 同一条口径），收成结构体只会把同一次调用拆成两处。
#[allow(clippy::too_many_arguments)]
fn land_contributions(
    store: &dyn Store<Event = Event>,
    face: symbio_plugin_api::SettleFace,
    turn: u64,
    user_seq: u64,
    principal: &str,
    user_text: &str,
    response: Option<&str>,
    recalled: Option<&symbio_plugin_api::retrieval::Retrieval>,
) {
    let anchor = format!("t{turn}");
    // 本轮的检索读数（DE-026 D-264 第 2 条）：与宿主读侧排版、`record_learning`
    // 同一个值形——摘除之后的条目在这里交给贡献者，而不是让能力自己反查。
    let recall_lines: Vec<symbio_plugin_api::RecallLine<'_>> = recalled
        .map(|view| {
            view.entries
                .iter()
                .map(|e| symbio_plugin_api::RecallLine {
                    seq: e.seq,
                    content: &e.content,
                    tag: &e.tag,
                })
                .collect()
        })
        .unwrap_or_default();
    let recall = recalled.map(|view| symbio_plugin_api::RecallFacts {
        degraded: view.degraded,
        lines: &recall_lines,
    });
    // 同一轮各贡献者共用同一个时刻：收束这一刻就是它们的 `ts`。
    let now_ms = symbio_sdk::clock::clock_now_ms();
    for contributor in symbio_plugin_api::settle::contributors() {
        // 完整史（`range(0)`）：累计口径的贡献者（巩固 / 校准）要看得见归档段。
        let history = store.range(Seq::new(0));
        let at = symbio_plugin_api::Settling {
            face,
            now_ms,
            principal,
            user_seq,
            anchor: &anchor,
            user_text,
            response,
            recall: recall.as_ref(),
            history: &history,
        };
        for contribution in contributor.contribute(&at) {
            let event_id = contribution.fact.key.clone();
            let actor = contribution.actor.unwrap_or(principal);
            let mut event = contribution.fact.into_event(actor);
            event.turn = turn;
            event.ts = now_ms;
            if let Err(e) = store.append(event) {
                symbio_plugin_api::plugin_warn!(
                    "session",
                    "[turn] 收束贡献入格失败（owner {}，事件 {event_id}，轮 {turn}）：{e:?}",
                    contributor.owner()
                );
            }
        }
    }
}

/// 一轮的执行之末：落格——消费 [`Settled`]，产本轮的最终结果。
pub(crate) fn persist_turn(s: Settled<'_>) -> Result<ExecTurnResult, TurnExecExit> {
    let Settled {
        view,
        store,
        outcome,
        produced,
        derived,
        collected,
        turn_no,
        user_seq,
        principal,
        to,
        user_text,
        recalled,
        child_id,
        reasoning_child_id,
        reasoning_text,
    } = s;
    let response = (!outcome.fell_back).then_some(outcome.text.as_str());
    // 本轮开口格（`u-{turn}`）的 seq —— 记忆 / 任务 / 承诺的溯源锚。
    //
    // 读**完整史**（`range(0)`）：这两个写方（`record_derived` 的
    // 承诺与任务表 / `record_learning` 的记忆两段）都是**累计口径**——它们的幂等
    // doc-link-allow D-004: 「曾经落过什么」问的是事实网格里有无该条记录，属数据语义
    // 判据与状态推导要看**曾经**落过什么。被快照取代而移入归档段的那些事实同样在
    // `range(0)` 里 ⇒ 压缩前开过的任务不会在就绪集里消失，幂等判据也不会失效
    // （看不见 ⇒ 重复落格）。
    //
    // 用户格 seq **不再从这里反查**：它在判决之前就落了，由编排层沿
    // `ExecTurn::user_seq` 直给——本函数只把完整史带给累计口径的写方。
    let all = store.range(Seq::new(0));
    // 承诺 / 任务表：写方是 `record_derived`（熔断事实由执行体随债自己落）。
    // 每族的失败在函数内各自记日志、互不打断——轮次已收束，派生事实失败不该
    // 把成功的一轮说成失败（与记忆同一条口径）。本调用点因此不判返回值。
    //
    // `to` = 来源主体（D-067）：本主体向「这条消息的出处」立约。缺省落
    // 本机用户——本机路径与接线前一致（取值点见 `request_source_principal`，
    // 在 `ctx` 被移进分发口之前取好）。
    derived_facts::record_derived(
        view.session_id.as_str(),
        store.as_ref(),
        &all,
        turn_no,
        user_seq,
        &principal,
        &to,
        &derived.effects,
    );
    // 收束贡献（DE-025 D-250）：能力在这一刻交出本轮的派生事实（记忆 / 技能 / 调度…）。
    // 排在效果之后：贡献者要看得见本轮刚落的工具效果（调度的输入就是本轮刚开的任务，
    // 宿主按 `prio` 逐个询问并在每次之前重取史）；排在淘汰回路之前——那一族读的是
    // 承诺与声誉，与这里的贡献没有数据依赖。
    land_contributions(
        store.as_ref(),
        symbio_plugin_api::SettleFace::Generated,
        turn_no,
        user_seq,
        &principal,
        &user_text,
        response,
        recalled,
    );
    // 淘汰回路（DE-043 §3）：**必须排在承诺落格之后**——它读的声誉投影
    // 要看得见本轮刚写下的那条违约（委托失败 ⇒ 声誉 −1 ⇒ 收回 assign.work）。
    // 顺序反了会让判据滞后一个轮次：那一轮里主体已经又派出去一轮。
    derived_facts::record_elimination(store.as_ref(), turn_no, user_seq);
    // 记忆：只剩**巩固**（`record_learning`）。编码、检索事实与技能编译、回退观测
    // 都不在这里——它们是能力包的收束贡献（`land_contributions`，D-250 / D-262 /
    // D-263 / D-264），上面那一步已把编码与 `memory.recalled` 落格，本函数读的是
    // **含新格**的史。
    derived_facts::record_learning(
        view.session_id.as_str(),
        // `store` 是 `Arc<dyn Store>`（轮内注入闭包要 `'static`），而本函数
        // 收 `&dyn Store` ⇒ `as_ref()` 一次取到 trait object。
        store.as_ref(),
        turn_no,
        symbio_sdk::clock::clock_now_ms(),
    );

    // 兜底已入格（I3），失败呈现走 Failed 出口 + 重试。
    if outcome.fell_back {
        return Err(TurnExecExit::Failed(PluginError::InternalError(
            outcome.text,
        )));
    }
    // 等待用户（`outcome.awaits_user`）：本轮尚未了结（运行器只落了进展格
    // `status: "pending"`，收束格留给恢复幕），但**按正常收束返回**——待用户动作由
    // 分发方广播的 `user_prompt` 节点承载（`WaitingUserAction`），呈现是卡片 +
    // 恢复入口；恢复时经 [`TurnResume`] 续写同一轮（不重开用户格）。
    // 此处的 `text` 只是模型在工具轮说过的话，不是终答。

    // 折进的补充也要进落库镜像：排在 `produced` **之前** ——
    // 用户先插话、工具结果后到，这是时间序（`persist_messages` 只写尾部增量，
    // 顺序错了落库就乱了）。
    // 同上：锁拿不到就用空的（落库镜像少这批补充，其余不受影响）。
    let mut all_messages: Vec<ChatNode> = collected
        .lock()
        .map(|mut v| std::mem::take(&mut *v))
        .unwrap_or_default();
    all_messages.extend(produced);

    Ok(ExecTurnResult {
        output: stage_output(outcome, reasoning_text, child_id, reasoning_child_id),
        messages: all_messages,
    })
}

/// **定稿答话轮**：`classify` 判 `Answered` ⇒ 本轮**一个模型请求都不发**，
/// 但收束格仍须由 v2 运行器落（用户格已由编排层在判决之前落）。
///
/// ## 它解决的是什么
///
/// `Answered` 判决在 `chat_loop` 步骤 2 就短路收尾，**根本不进 [`drive`]**；
/// 若就此返回，那一轮在事实网格里只有用户格、没有 `chat.assistant.final`。于是下一轮
/// prompt 里这次问答**整个消失**，`session/stats` 的轮次计数也缺收束——而
/// `messages.json` 里有（`context.messages` 落库）⇒ **两条真源**。完全静默：不报错、
/// 不告警、既有断言全绿。判据 = e2e `t22`。
///
/// ## 为什么走运行器，而不是本函数自己写收束格
///
/// 「一个 turn = 用户格 opened + 至多一条收束 closed」这条不变量由运行器守着。插件自己
/// 补写就成了**第二个轮次事实写方**——同轮两份记账的假象随之回来。所以答话交给运行器
/// （`TurnInput::final_reply`），收束格仍由它落。
///
/// ## 与生成路径的差异
///
/// 1. **承诺 / 任务表不写**：那两族的事实来源是工具交换（派活、就绪集、
///    失败），`Answered` 轮一次工具都不调，没有可记的东西。
///    但**记忆不在此列**：编码的是「这一轮用户说了什么、智能体答了什么」，与
///    有没有工具交换无关。跳过它就是答话轮记忆静默丢失（实测：默认配置下
///    「记住我的幸运数字」由 `classify` 判成答话 ⇒ WAL 里没有 `memory.encoded`，
///    下一个会话当然想不起来）。因此这里与 [`persist_turn`] 一样先走
///    `land_contributions`（编码是情景记忆能力包的收束贡献，D-263）、再调
///    `record_learning`（检索锚 + 巩固）——见 DE-036「记忆编码是轮级派生，不依附驱动循环」。
/// 2. **不上线 UI 帧**：答话节点已由 `apply_verdict` 推进 `context.messages`
///    （`dialog_node`，`exclude_from_context = false`），由 `finish_turn` 落库 ⇒ 这里用
///    [`symbio_sdk::adapters::SilentDeltas`]；再发一次就是同一条答话重复上屏。
///
/// ## 模型面不是本路的依赖
///
/// 短路发生在**用到它之前**（[`run_loop`] 的答话守卫）——这一轮一次模型
/// 往返都不发，因此不取 `port.model`。
///
/// 形参宽（`consolidation` 与三个 seq 都是这一轮的**事实端口**）：与
/// [`open_turn`] 同一条口径——「一轮的全部输入」本就是
/// 一个宽签名，收成结构体只会把同一次调用拆成两处。
#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute_final_reply_turn(
    context: &SessionContext,
    ctx: &Arc<dyn PluginInvokeRequest>,
    abort: symbio_kernel::exec::ExecAbortSignal,
    // 轮号与溯源锚由编排层在判决之前定下：答话轮同样先落用户格，
    // 这里只收三个 seq（`drive_seq` 供派发格幂等键）。
    turn_no: u64,
    user_seq: u64,
    drive_seq: u64,
    // 本轮用户发言：记忆编码的**内容**（`record_learning`）。答话轮同样
    // 是「用户说了什么」的一轮，不传它就只能记下判断、丢掉经验。
    user_text: &str,
    reply: &str,
    // 生成 `reply` 的那次措辞往返的实测耗时（`WordingReply.cost_ms`）。它是本
    // 轮唯一的模型往返代价，落进收束格的 `cost_ms`；模板 / 无往返产线给 0。
    reply_cost_ms: u64,
    // 答话**对话面节点**的身份与标记（`id` / `reason` / `exclude_from_context`）：
    // 收束格据它复刻那条根级节点（`Answered` 答话 `exclude = false`）；必须随
    // 事实入格（WAL 为唯一事实源，见 `TurnInput::dialog`）。
    dialog: DialogFacts,
    // 生成这次答话的**思考事实**（`(reasoning_id, reasoning)`）：措辞产线的模型
    // 往回是这条轮次里唯一一次真实推理，不入格则 `Reason` 节点对普通答话轮从未
    // 存在（AMD MiMo 实测 2026-10-13）。模板产线 / 生成失败退模板 ⇒ `None`。
    reasoning: Option<(String, String)>,
    // 本轮之前发生过压缩 ⇒ 压缩后的整份消息列表（与工具轮同源：
    // `run_seam` 从 `context.consolidated` 取走的那一份）。
    // 答话轮在步骤 2 就短路收尾，**不进** [`drive`]，所以它也得自己落这一格
    // ——否则「上一轮末尾压过一次、这一轮是答话轮」时那次压缩永远不入格。
    consolidation: Option<&[ChatNode]>,
) -> Result<(), PluginError> {
    // 事实源：与 [`assemble_turn`] 同一条路径——交付包构造的唯一句柄（D-084 §2）。
    let store = context
        .facts()
        .ok_or_else(|| PluginError::InternalError("执行需要持久会话（临时会话无事实源）".into()))?;
    // **补落寄存的变更**：答话轮同样是会话侧的落格窗口，经端口动作发起。
    // 与 [`assemble_turn`] 同序（在运行器落格之前——本轮的用户格已由编排层先落）。
    if let Err(e) =
        session_port::call_unit(context.session.as_ref(), run_ops::DRAIN_PARKED, ()).await
    {
        symbio_plugin_api::plugin_warn!("session", "[Turn] 补落寄存变更失败（本轮继续）: {e}");
    }
    // 身份与档位取值点同 [`assemble_turn`]：主体 = 会话选定的那个 agent；档位 = 深度
    // （`Answered` 轮没有可跳过的模型调用，不属于反射档）。规格从授权表行取
    // （`request_actor_spec`，能力集 / 模式不再是平凡值）。
    //
    // 主体**不吃档位**（`D-223` 订正 2）：规格里那一格是主体的声明档，本幕的档与预算
    // 归 `turn.dispatch`（下面那次 `land_dispatch` 直接折 `tier.budget_ms()`）。
    // 作用域与身份同批取：子会话挂父主体，顶层会话恒 `Root`。
    let scope = context.view.scope.clone();
    let actor = chat_loop::request_actor_spec(ctx.as_ref(), scope);
    // 压缩入格 + 归档：与接缝同序（`D-223` 订正 1：压缩必须早于派发格）。归档之后才
    // 落派发格，本幕的 `turn.dispatch` 才不会随「本轮之前的一切」一起被移进归档段。
    land_consolidation(context.session.as_ref(), consolidation).await;
    // 派发档（`D-223` 订正 1）：答话轮不走快路（`Answered` 是判决的结论，不是技能的命中），
    // 档位恒为深度档。它与工具轮那一格同一条纪律——落在这里而不是草稿的构造里。
    chat_loop::land_dispatch(
        store.as_ref(),
        turn_no,
        user_seq,
        drive_seq,
        LatencyTier::Deep,
        &actor,
    )
    .map_err(|e| PluginError::InternalError(format!("派发档落格失败：{e:?}")))?;
    // 本入口不开环 ⇒ 不需要模型端口与令牌（那两者只由 [`drive`] 的动作读）。
    let input = TurnInput {
        turn: turn_no,
        user_seq,
        drive_seq,
        // 不生成 ⇒ 没有 prompt，也就没有窗口可裁。
        window_turns: None,
        actor,
        inject: None,
        final_reply: Some(reply.to_string()),
        final_cost_ms: reply_cost_ms,
        // 答话**对话面节点**的身份与标记：收束格据它复刻那条根级节点。答话轮没有
        // 流式过程 ⇒ 无半截可记：载荷 `turn_id` 记空串（重建侧按 `dialog.id` 与
        // 事件 id 复刻），取数口按有无思考事实决定挂不挂（见下方 `nodes`）。
        dialog: Some(dialog),
        turn_root_id: String::new(),
        // 取数口：**恒定快照**——答话轮没有流式过程（措辞插件的模型往返是静默的，
        // 此刻已结束），思考事实是调用方搬运来的定值。挂上它，定稿答话收束格的
        // `produced` 载荷才带 `reasoning` / `reasoning_id`（重建据此复刻 Reason 节点）。
        nodes: reasoning.map(|(reasoning_id, reasoning)| {
            let facts = StreamNodeFacts {
                text_id: None,
                text: None,
                reasoning_id: Some(reasoning_id),
                reasoning: Some(reasoning),
            };
            Arc::new(move || facts.clone()) as NodeFactsFn
        }),
        // 定稿答话轮不经过工具循环的闸门空隙——它一次请求都不发（`final_reply` 直接
        // 收束）。但信号照传：若用户在答话落格前按下停止，首请求前中止检查
        // 会在落完用户格后接管，与其它轮同一条路径（终止是状态，不是特例）。
        abort: Some(abort),
    };
    // `Run` 这一支在本入口**按构造不可达**（`final_reply` 非空 ⇒ `prepare_turn` 的
    // 定稿答话分支就地收束、返回 `Done`）。本入口**不开环**（`D-222` 裁决 5：环只住在
    // [`drive`]，而它要的那一片编排层状态这一轮根本没有）——真到了这里说明
    // 收束路径与交付包对不上，出声。
    let outcome = match begin_turn(store.as_ref(), input)
        .await
        .map_err(|e| PluginError::InternalError(format!("运行器落格失败：{e:?}")))?
    {
        TurnSessionBegin::Done(outcome) => outcome,
        TurnSessionBegin::Run(_) => {
            return Err(PluginError::InternalError(
                "定稿答话轮开出了驱动环（按构造不可达）".into(),
            ))
        }
    };
    // ── 记忆（与 [`persist_turn`] 同一条口径）──────────────────────────────
    //
    // 三个跳过条件与 `classify_turn` 逐条对齐：中止 / 收束被授权拒绝 ⇒ 本轮
    // 没有定稿产出，派生事实全不写；兜底 ⇒ 只写用户经验（收束贡献拿到的
    // `response` 是 `None`，技能编译因此不写）。**没有工具交换不构成跳过条件**
    // ——记忆是轮级派生，不是工具副产物。
    if !outcome.aborted && !outcome.closure_denied {
        let principal = symbio_sdk::authz::request_principal(ctx.as_ref());
        let response = (!outcome.fell_back).then_some(reply);
        // 收束贡献（DE-025 D-250）：答话轮同样是一条「用户说了什么、智能体答了什么」
        // 的收束——记忆与学习在这里不该缺席。贡献者按 `SettleFace::Answered` 自行判断
        // 哪一部分不适用于零工具交换的那一刻（调度就不在这个面上开账）。
        land_contributions(
            store.as_ref(),
            symbio_plugin_api::SettleFace::Answered,
            turn_no,
            user_seq,
            &principal,
            user_text,
            response,
            // 答话轮不建召回视图（分类器判 `direct` 的前提就是「现有对话里答得出」）
            // ⇒ 语义记忆的贡献者拿到 `None`，这一轮不落 `memory.recalled`。
            None,
        );
        derived_facts::record_learning(
            context.view.session_id.as_str(),
            store.as_ref(),
            turn_no,
            symbio_sdk::clock::clock_now_ms(),
        );
    }
    Ok(())
}
