//! 判决往返的派发（D-122 决策 2 / 4）——`classify` 那一次模型往返的入账形状。
//!
//! ## 它为什么住在端口层
//!
//! 这一段的**全部**内容就是「一次模型往返怎么入账」：规则短路时**零格**，模型路径
//! 时**三格带 `via`**。它住在端口层，用例也住在同一处——`cargo test -p symbio
//! --lib "loop::port"` 一条命令跑得着（验收口径 2）。
//!
//! ## 两条分支为什么必须在**同一个**函数里
//!
//! 「规则短路 ⇒ 零入账格」与「模型路径 ⇒ 三格」是同一个决策的**两侧**。拆成两处
//! （一处判零格、一处派发），判据就退化成「零格那条路没被调到」——那正是
//! D-122 决策 4 否掉的形态（「行成立但没写格」）。
//!
//! ## `model_calls` 为什么是**数**而不是「来源」
//!
//! `turn.stage{decide}` 的 `source` 记的是判决的**来源**（规则 / 模型 / 兜底），
//! 读不出「有没有打过模型」：模型调了而应答不可解析 ⇒ 判决落 `Escalate{unclassified}`
//! ⇒ `source=fallback`。判据侧要拿这次往返在事实层唯一的**数**去对账
//! 「观测 ⇔ 三格」，那个数就是它。

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use symbio_core::{Event, Store};
use symbio_kernel::dialog::{DecideOutcome, DecisionPlan, Verdict, REASON_UNCLASSIFIED};

use super::{round_trip, Round};

/// 一次判决的**结果**：判决 + 它打了几次模型。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Decision {
    /// 判决（闭集枚举）。
    pub(crate) verdict: Verdict,
    /// 这一轮判决发起的**模型往返次数**（规则短路 / 没有模型服务 = 0）。
    ///
    /// 它由**闭包**数出来，不由返回值推断：派发被拒时执行体压根没跑（0），
    /// 落格失败时它已经跑完（1）——两者都是 `Err`，靠返回值分不开。
    pub(crate) model_calls: u64,
}

/// 判决的派发点：规则短路 ⇒ **零入账格**；模型路径 ⇒ 走端口 `model`（三格带 `via`）。
///
/// ## 「没有模型服务」为什么也走这里
///
/// 编排层拿不到模型服务时把判决表达成 [`DecideOutcome::Decided`]（理由码
/// `unclassified`）：这一轮**不欠**模型往返 ⇒ 行不成立 ⇒ 零入账格（D-122
/// 决策 4）。先找服务再派发，顺序反了就会落一条「派发了却没打模型」的账——
///
/// ## `parse` 为什么是闭包
///
/// 「应答长什么样算哪个词」是 `classify` 的内部知识（词表只有它一处），编排层不
/// 知道那四个词。闭包把它接回来，端口层因此不必认识词表。它是**异步**的：解析要
/// 经容器路由回 `classify`，那是唯一拿得到那份词表的地方。
///
/// ## `run` 为什么收下**计划**
///
/// 计划是这次往返的**内容**（提示词 + 消息 + 模型）。端口层拿着它派发，才谈得上
/// 「这次往返长什么样」；调用方把它抄一份塞进闭包，计划就有第二个去处。
pub(crate) async fn classify_round_trip<S, F, Fut, P, PFut>(
    store: &S,
    round: Round<'_>,
    outcome: DecideOutcome,
    run: F,
    parse: P,
) -> Decision
where
    S: Store<Event = Event> + ?Sized,
    F: FnOnce(DecisionPlan) -> Fut,
    Fut: std::future::Future<Output = Result<String, String>>,
    P: FnOnce(&str) -> PFut,
    PFut: std::future::Future<Output = Verdict>,
{
    let plan = match outcome {
        // 规则短路（判据 2 的一半）：判决已定，**一条格都不落**。
        DecideOutcome::Decided { verdict } => {
            return Decision {
                verdict,
                model_calls: 0,
            };
        }
        DecideOutcome::NeedsModel { plan } => plan,
    };

    // 模型调用次数由闭包数：它是唯一知道「模型真的被打了」的一方。
    let calls = Arc::new(AtomicU64::new(0));
    let counter = calls.clone();
    let counted = move || {
        let counter = counter.clone();
        async move {
            counter.fetch_add(1, Ordering::SeqCst);
            run(plan).await
        }
    };
    match round_trip(store, round, counted).await {
        Ok(reply) => Decision {
            verdict: parse(&reply).await,
            model_calls: calls.load(Ordering::SeqCst),
        },
        Err(e) => {
            // 派发没成（模型没答成 / 行不欠 / 落格失败）：判决退回 `Escalate`——
            // **沉默是最坏的失败形态**（用户什么都收不到，且没有错误信号）。
            symbio_core::plugin_warn!("session", "[Classify] 判决往返未成，按 Escalate 处理：{e}");
            Decision {
                verdict: escalate_unclassified(),
                model_calls: calls.load(Ordering::SeqCst),
            }
        }
    }
}

/// 「判不出来」的判决：`Escalate` + 理由码 `unclassified`。
pub(crate) fn escalate_unclassified() -> Verdict {
    Verdict::Escalate {
        reason: REASON_UNCLASSIFIED.to_string(),
    }
}

#[cfg(test)]
#[path = "classify.test.rs"]
mod tests;
