//! **轮首 host** 的执行体与派发点（DE-015 的 D-159 落地）。
//!
//! ## 这一台 host 管的是哪几步
//!
//! 一轮的固定七步里，「判决 / 压缩 / 召回」三步的账在行表上、动作也在这台的绑定表里：
//! 派发点向求解器要本 host 行子集里最小的一笔可还债，行按 `by` 落进哪张绑定表就住哪台
//! host（D-159）。**账在表上、动作在表外**是 DE-090 §3 的调度旁路，本模块的形状就是要它
//! 不出现。选债仍然只有 `ring::drive` 一个求解器。
//!
//! ## host 是借用面，不是新的机制词
//!
//! [`super::turn_exec::TurnStep`] 借走本轮草稿 + 七个端口；[`HeadStep`] 借走本轮状态 +
//! 会话上下文 + 请求句柄。动作形状逐字相同（`fn(&mut Step) -> StepFuture`）、失败通道
//! 同一个 [`ActorError`]、对账名单同一个 `Next::writes_closed`——两台 host 的差别只在
//! **借什么**，不在**怎么选**。
//!
//! ## 出口那一格由执行体自己落
//!
//! 执行体不交回产出（D-123 决策 5 在轮首照用）：召回的三份产物（记忆视图 / 调度段 /
//! 技能命中）寄存进本轮状态，由编排层取。但它**做完了**这件事只写在日志上——
//! `turn_stages` 视图按幕锚折叠已落的 `turn.stage` 格，不落格的执行体让本幕的第二次
//! 选债照样读到「没做过」，环在这一台上一步都走不动。落格能力就在编排层
//! （`chat_loop::drive::begin_drive` 落用户格 / 配置 / 轮次格走的是同一条路、同一个句柄）。

use std::sync::Arc;

use crate::symbio_core::{Event, PluginInvokeRequest, Seq, Store};

use super::chat_loop::state::{ChatOrchestrator, SessionContext, TurnState};
use super::turn_exec::{ActorError, StepFuture};

/// 一次轮首执行体动作需要的**全部**环境：编排器 + 请求句柄 + 事实源 + 会话上下文 +
/// 本轮状态。
///
/// 与 [`super::turn_exec::TurnStep`] 同样是派发器与执行体之间唯一的界面：执行体拿得到
/// 它们，拿不到「下一笔债是什么」的选择权（D-118 决策 4 在轮首照用）。
///
/// 会话上下文是 `&mut`：压缩改写 `context.messages`（就地折叠历史）。轮首这三行
/// （判决 / 压缩 / 召回）里只有压缩要写它，但借用面按**这台 host 借走什么**声明，
/// 不按「这一批恰好谁用到」声明——否则接下一行时又要改形状。
pub(crate) struct HeadStep<'a> {
    pub(crate) orchestrator: &'a ChatOrchestrator,
    pub(crate) ctx: &'a Arc<dyn PluginInvokeRequest>,
    /// 本轮的**事实源**：执行体落自己那一行出口的地方，派发器读账本的地方。
    ///
    /// 交 `Arc` 而不是 `&dyn Store`：压缩那一步的入账面（[`super::port::Compress`]）
    /// 持有一份句柄随摘要往返走，派发器与执行体拿同一个句柄才是同一个事实源。
    pub(crate) store: &'a Arc<dyn Store<Event = Event>>,
    pub(crate) context: &'a mut SessionContext,
    pub(crate) turn: &'a mut TurnState,
}

/// 轮首绑定表里的动作类型（与 `turn_exec::Action` 同形状，借的面不同）。
pub(crate) type HeadAction = for<'a> fn(&'a mut HeadStep<'a>) -> StepFuture<'a>;

/// `classify` 执行体：`policy.classify` 行选中的动作（D-159）。
///
/// 本体同样留在它的领域层（[`super::chat_loop::run_classify`]——判决能力 `classify`
/// 与它的效应执行都住在那里），这里只把它指到名字上并交出出口那一格。
pub(crate) fn run_classify<'a>(step: &'a mut HeadStep<'a>) -> StepFuture<'a> {
    Box::pin(async move {
        let HeadStep {
            orchestrator,
            ctx,
            store,
            context,
            turn,
        } = step;
        super::chat_loop::run_classify(orchestrator, ctx, store, context, turn).await
    })
}

/// `recall` 执行体：`policy.recall` 行选中的动作。
///
/// 动作本体留在它的领域层（[`super::chat_loop::run_recall`]——记忆 / 技能 / 调度段
/// 三段读视图都住在那里），这里只把它指到名字上并交出出口那一格。
pub(crate) fn run_recall<'a>(step: &'a mut HeadStep<'a>) -> StepFuture<'a> {
    Box::pin(async move {
        let HeadStep {
            ctx,
            store,
            context,
            turn,
            ..
        } = step;
        super::chat_loop::run_recall(ctx, &***store, context, turn).await
    })
}

/// `compress` 执行体：`policy.compress` 行选中的动作。
///
/// 本体同样留在它的领域层（[`super::chat_loop::run_compress`]，压缩流水线的唯一入口
/// 在 `context::pipeline`）。水位要的那份请求级开销读 [`HeadStep::turn`] 的
/// `request_overhead`——它由准备段的 ①② 写进本轮状态。
pub(crate) fn run_compress<'a>(step: &'a mut HeadStep<'a>) -> StepFuture<'a> {
    Box::pin(async move {
        let HeadStep {
            orchestrator,
            ctx,
            store,
            context,
            turn,
        } = step;
        super::chat_loop::run_compress(orchestrator, ctx, store, context, turn).await
    })
}

/// 轮首派发点：每步向求解器要**本 host 行子集**里最小的一笔可还债，跑它的执行体，
/// 直到这一台不再欠账。
///
/// 与 `turn_exec::drive_steps` 共享的全部纪律：选债只有一个求解器（[`super::loop_rows::next_head`]）、
/// 动作前后各读一次日志（尾段 = 这一次动作的**实际写入**，不由执行体自报）、名单外
/// 领域格与空账本都出声。
///
/// 两处**按 host 不同**的处置，都不是放宽：
///
/// - 停机格的落法：轮内派发器拿得到本轮草稿，违规时落一条 `exhausted{…}` 收束格；
///   轮首派发点没有草稿可寄存，收束格的名额在 `close` 执行体名下（两个执行体写同一批格
///   ⇒ 装载期依赖成环）。故这里判红的方式是**出声 + 停驱**，账仍开着、下一幕照常读到它。
/// - 空账本在轮内是 `exhausted{no_progress}`，在轮首是**必须停**：出口没落格 ⇒ 同一笔
///   债再次被选中 ⇒ 空转。停驱加告警，就是「这一行的执行体写了零格」的显形。
pub(crate) async fn drive_head(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn PluginInvokeRequest>,
    context: &mut SessionContext,
    turn: &mut TurnState,
) -> Result<(), ActorError> {
    loop {
        // **临时会话整台跳过**：轮首这几行的出口格只能落在事实源上，没有事实源就没有
        // 账可对——派发点读到「没做过」却永远关不了账。跳过而不是判红：这一支在 v2 路径
        // 上到得了的地方只有 `begin_drive` 之前的段（开轮块已把无事实源的幕判成失败），
        // 而这里不许把一个优化项的缺账升级成整轮失败。
        let Some(store) = context.session.facts() else {
            return Ok(());
        };
        let facts = store.range(Seq::new(0));
        let Some(next) = super::loop_rows::next_head(
            &facts,
            turn.turn_no,
            turn.now_ms,
            super::loop_rows::HEAD_ACTORS,
        ) else {
            return Ok(());
        };
        crate::plugin_debug!(
            "session",
            "[Loop] 轮首派发 {}（执行体 {}；债 {}）",
            next.row,
            next.by,
            next.key
        );
        let head = store.head();
        {
            let mut step = HeadStep {
                orchestrator,
                ctx,
                store: &store,
                context,
                turn,
            };
            (next.action)(&mut step).await?;
        }
        let ledger = store.range(head);
        if let Some(offender) = ledger
            .iter()
            .map(|e| e.kind.as_str())
            .find(|k| !next.writes_closed(k))
        {
            crate::plugin_warn!(
                "session",
                "[Loop] 轮首执行体 {} 写了未登记的事实 `{}`（行 {}，turn={}）——本 host 停驱，账仍开着",
                next.by,
                offender,
                next.row,
                turn.turn_no
            );
            return Ok(());
        }
        if ledger.is_empty() {
            crate::plugin_warn!(
                "session",
                "[Loop] 轮首执行体 {} 落了零格（行 {}，turn={}）——出口没进日志，本 host 停驱",
                next.by,
                next.row,
                turn.turn_no
            );
            return Ok(());
        }
        // 派发点重读切片：下一问问表读的是**刚落的那一格**（视图按幕锚折叠，
        // 不刷新就会把同一笔债再还一遍）。
        turn.facts = store.range(Seq::new(0));
    }
}
