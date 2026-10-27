//! `fallback_rate` 投影（SLO §1.2 兜底率列的统计口径，[plan/04 §1.2](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）。
//!
//! 兜底率 = 每档位上「走了兜底」的 turn 占比。SLO 初值：反射 < 1% / 快速 < 3% /
//! 深度 < 5% / 自主 < 5%（**初值，阶段三出真实数字**）——本投影只报数，
//! 达标判定交给读方（SLO 是承诺，投影是事实）。
//!
//! 口径：
//! - **turn** = `user.message`（`turn × opened`）——用户消息开启一轮；
//! - **档位** = `turn.dispatch` 载荷 `tier`（[`crate::adapters::LatencyTier`]
//!   的名字）——**调度决定是数据**：档位要到执行装配期才定，故另立一格；
//!   历史事件没有这一格时回退读 `user.message` 的 `tier`；缺失/未知 ⇒ `unspecified`
//!   桶（可观测，不静默归类）；
//! - **兜底** = `chat.assistant.fallback`（`turn × closed`）——生成失败也必须有
//!   输出（I3），fallback 落在哪档就是哪档的兜底。

use super::super::event::{Entity, Event, Verb};
use super::Projection;
use crate::ChatVocab;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 一个档位的兜底统计。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TierStats {
    /// 档位名（四层之一，或 `unspecified`）。
    pub tier: String,
    /// turn 总数（final + fallback 都是被服务的 turn）。
    pub turns: u64,
    /// 其中走兜底的轮数。
    pub fallbacks: u64,
}

impl TierStats {
    /// 兜底率（无 turn = 无数据，按 0.0——不给空档位报警）。
    pub fn rate(&self) -> f64 {
        if self.turns == 0 {
            0.0
        } else {
            self.fallbacks as f64 / self.turns as f64
        }
    }

    /// 零账（`merge` 的累加起点；与 [`Self::of`] 查不到时给的是同一形态）。
    pub fn empty(tier: &str) -> Self {
        Self {
            tier: tier.to_string(),
            turns: 0,
            fallbacks: 0,
        }
    }
}

/// 兜底率视图：按档位名字典序（BTreeMap ⇒ 逐字节确定，N1）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FallbackRateView {
    pub by_tier: BTreeMap<String, TierStats>,
}

impl FallbackRateView {
    /// 某档位的统计（无数据 ⇒ 零账，不是错误）。
    pub fn of(&self, tier: &str) -> TierStats {
        self.by_tier.get(tier).cloned().unwrap_or(TierStats {
            tier: tier.to_string(),
            turns: 0,
            fallbacks: 0,
        })
    }

    /// 合并多份视图（**跨会话聚合**）。与
    /// `SloLatencyView::merge`
    /// 同族同理由：
    /// 分子分母都是**计数**，逐档位相加即可——率不在这时候算（`TierStats::rate`
    /// 是纯查询，合并后再算与分开算逐字相同，因为率是除法而不是累加）。
    ///
    /// 空输入 ⇒ 空视图。同档位在两份输入里都有时**相加**，不取较新的一份：
    /// 取新会让「上一次计算的 persist」悄悄覆盖历史，而 WAL 是 append-only 的，
    /// 读数没有「哪一份更新」可谈。
    pub fn merge(parts: &[FallbackRateView]) -> FallbackRateView {
        let mut by_tier: BTreeMap<String, TierStats> = BTreeMap::new();
        for part in parts {
            for (tier, st) in &part.by_tier {
                let acc = by_tier
                    .entry(tier.clone())
                    .or_insert_with(|| TierStats::empty(tier));
                acc.turns += st.turns;
                acc.fallbacks += st.fallbacks;
            }
        }
        FallbackRateView { by_tier }
    }
}

/// 本轮声明的档位：优先读本轮 `turn.dispatch` 载荷的 `tier`，历史切片没有
/// 这一格时回退读 `user.message` 的 `tier`；缺失/未知 ⇒ `unspecified` 桶
/// （可观测，不静默归类）。
///
/// `before_seq`：
/// - `Some(b)` ⇒ 只认 seq `< b` 的声明（兜底事件要归到**它之前**那一幕的档位）；
/// - `None` ⇒ 不限 seq，取**最早**一条派发格——开轮事件的分母按「这一轮以哪一档
///   开局」归位（续写幕可以换档，那是另一幕的事实）。
///
/// 兜底率与时延报告（[`super::slo`]）共用的**唯一判据**——档位归属只在这
/// 一处定义，两列统计不各自为政。
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
        .and_then(crate::adapters::LatencyTier::from_name)
        .map(|t| t.name().to_string())
        .unwrap_or_else(|| "unspecified".to_string())
}

/// `fallback_rate` 投影：从事件切片统计各档位的兜底率。
pub fn fallback_rate(v: ChatVocab) -> Projection<FallbackRateView> {
    Projection::new(move |events: &[Event], now| {
        let mut by_tier: BTreeMap<String, TierStats> = BTreeMap::new();
        for e in events {
            if e.ts > now {
                continue;
            }
            if e.entity != Entity::Turn {
                continue;
            }
            match e.verb {
                Verb::Opened => {
                    if e.kind != v.user_message {
                        continue;
                    }
                    // 档位 = 本轮派发格（历史事件回退开轮格）；缺失/未知进
                    // unspecified（可观测）。分母按整条史取**最早**一幕的派发档。
                    let tier = declared_tier(events, e.turn, None, &v);
                    let st = by_tier.entry(tier.clone()).or_insert_with(|| TierStats {
                        tier,
                        turns: 0,
                        fallbacks: 0,
                    });
                    st.turns += 1;
                }
                Verb::Closed => {
                    if e.kind != v.fallback {
                        continue;
                    }
                    // 兜底落在哪档 = 本轮用户消息声明的档位（turn 号即归属）。
                    let tier = declared_tier(events, e.turn, e.seq.map(|s| s.value()), &v);
                    let st = by_tier.entry(tier.clone()).or_insert_with(|| TierStats {
                        tier,
                        turns: 0,
                        fallbacks: 0,
                    });
                    st.fallbacks += 1;
                }
                _ => {}
            }
        }
        FallbackRateView { by_tier }
    })
}
