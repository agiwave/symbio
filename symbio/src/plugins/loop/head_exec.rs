//! **轮首 host** 的执行体与派发点（DE-015 的 D-159 落地第一批）。
//!
//! ## 它接回的是什么
//!
//! 一轮的固定七步里，「判决 / 准备」那几步此前由外层代码按 `owes()` 点问自己执行：
//! 行在表上声明了这笔账，动作却住在派发面之外——`DE-090 §3` 叫它**调度旁路**。
//! D-159 把它接回环上：一台 host 一张绑定表，行按 `by` 落进哪张表就住哪台 host，
//! 选债仍然只有 `ring::drive` 一个求解器。
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
//! 召回的三份产物（记忆视图 / 调度段 / 技能命中）寄存在本轮状态里由编排层取，执行体
//! 不交回产出（D-123 决策 5 在轮首照用）。但它**做完了**这件事只写在日志上：
//! `turn_stages` 视图按幕锚折叠已落的 `turn.stage` 格，草稿要等到步骤 4 才进日志，
//! 于是寄存草稿的形态下本幕的第二次选债照样读到「没做过」——环在这一台上一步都走不动。
//! 落格能力此刻就在编排层（`chat_loop::drive::begin_drive` 落用户格 / 配置 / 轮次格
//! 走的是同一条路），所以接回这一行顺带把「草稿交运行器落」那条寄存去掉。

use std::sync::Arc;

use crate::symbio_core::{Event, PluginError, PluginInvokeRequest, Seq, Store};

use super::chat_loop::state::{SessionContext, TurnState};
use super::turn_exec::{ActorError, StepFuture};

/// 一次轮首执行体动作需要的**全部**环境：本轮状态 + 会话上下文 + 请求句柄 + 事实源。
///
/// 与 [`super::turn_exec::TurnStep`] 同样是派发器与执行体之间唯一的界面：执行体拿得到
/// 它们，拿不到「下一笔债是什么」的选择权（D-118 决策 4 在轮首照用）。
pub(crate) struct HeadStep<'a> {
    pub(crate) ctx: &'a Arc<dyn PluginInvokeRequest>,
    /// 本轮的**事实源**：执行体落自己那一行出口的地方，派发器读账本的地方。
    pub(crate) store: &'a dyn Store<Event = Event>,
    pub(crate) context: &'a SessionContext,
    pub(crate) turn: &'a mut TurnState,
}

/// 轮首绑定表里的动作类型（与 `turn_exec::Action` 同形状，借的面不同）。
pub(crate) type HeadAction = for<'a> fn(&'a mut HeadStep<'a>) -> StepFuture<'a>;

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
        } = step;
        super::chat_loop::run_recall(ctx, *store, context, turn).await
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
    ctx: &Arc<dyn PluginInvokeRequest>,
    context: &SessionContext,
    turn: &mut TurnState,
) -> Result<(), ActorError> {
    loop {
        let store = context.session.facts().ok_or_else(|| {
            ActorError::Read(PluginError::InternalError(
                "轮首派发需要持久会话（出口格无处可落）".into(),
            ))
        })?;
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
                ctx,
                store: store.as_ref(),
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
