//! 档位归属的**唯一判据**——兜底率（[`super::fallback`]）与时延报告（[`super::slo`]）
//! 两列统计共用，不各自为政。
//!
//! 本模块**不产 `Projection`**：它是折叠目录里的共享读侧，与 core 的 `mutations` /
//! `rebuild` / `turns` 同一形态。两个产投影的兄弟都可以导它——`PP-001` 的
//! 「投影 A 不读投影 B」（L2）因此不需要豁免，也不必把这条判据搬回架构模块。

use symbio_core::{ChatVocab, Entity, Event, Verb};

/// 本轮声明的档位：优先读本轮 `turn.dispatch` 载荷的 `tier`，历史切片没有
/// 这一格时回退读 `user.message` 的 `tier`；缺失/未知 ⇒ `unspecified` 桶
/// （可观测，不静默归类）。
///
/// `before_seq`：
/// - `Some(b)` ⇒ 只认 seq `< b` 的声明（兜底事件要归到**它之前**那一幕的档位）；
/// - `None` ⇒ 不限 seq，取**最早**一条派发格——开轮事件的分母按「这一轮以哪一档
///   开局」归位（续写幕可以换档，那是另一幕的事实）。
pub(super) fn declared_tier(
    events: &[Event],
    turn: u64,
    before_seq: Option<u64>,
    v: &ChatVocab,
) -> String {
    let before = |e: &Event| before_seq.is_none_or(|b| e.seq.is_some_and(|s| s.value() < b));
    let is_dispatch = |e: &Event| {
        e.entity == Entity::Turn
            && e.verb == Verb::Asserted
            && e.kind == v.dispatch
            && e.turn == turn
    };
    let dispatch = match before_seq {
        None => events.iter().find(|e| is_dispatch(e) && before(e)),
        Some(_) => events.iter().rev().find(|e| is_dispatch(e) && before(e)),
    };
    dispatch
        .and_then(|e| e.payload.get("tier").and_then(|v| v.as_str()))
        // 接线前的历史事件：档位随开轮格写入（`user.message.tier`）。
        .or_else(|| {
            events
                .iter()
                .rev()
                .find(|u| {
                    u.entity == Entity::Turn
                        && u.verb == Verb::Opened
                        && u.turn == turn
                        && u.kind == v.user_message
                        && before(u)
                })
                .and_then(|u| u.payload.get("tier").and_then(|v| v.as_str()))
        })
        .and_then(symbio_core::LatencyTier::from_name)
        .map(|t| t.name().to_string())
        .unwrap_or_else(|| "unspecified".to_string())
}
