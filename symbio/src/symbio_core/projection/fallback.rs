//! `fallback_rate` 投影（SLO §1.2 兜底率列的统计口径，[plan/04 §1.2](../../../../docs/plan/04-工程落地.md)）。
//!
//! 兜底率 = 每档位上「走了兜底」的 turn 占比。SLO 初值：反射 < 1% / 快速 < 3% /
//! 深度 < 5% / 自主 < 5%（**初值，阶段三出真实数字**）——本投影只报数，
//! 达标判定交给读方（SLO 是承诺，投影是事实）。
//!
//! 口径：
//! - **turn** = `user.message`（`turn × opened`）——用户消息开启一轮；
//! - **档位** = 用户消息载荷 `tier`（[`crate::symbio_core::adapters::LatencyTier`]
//!   的名字）——**调度决定是数据**：谁装配 turn 进哪一档，谁写进载荷；
//!   缺失/未知 ⇒ `unspecified` 桶（可观测，不静默归类）；
//! - **兜底** = `chat.assistant.fallback`（`turn × closed`）——生成失败也必须有
//!   输出（I3），fallback 落在哪档就是哪档的兜底。

use super::super::event::{Entity, Event, Verb, EVENT_ASSISTANT_FALLBACK, EVENT_USER_MESSAGE};
use super::super::view::{Budget, View};
use super::Projection;

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
}

/// 本轮声明的档位：找 `turn` 号在本事件**之前**开启的那条 `user.message`，
/// 读载荷 `tier`；缺失/未知 ⇒ `unspecified` 桶（可观测，不静默归类）。
///
/// 兜底率与时延报告（[`super::slo`]）共用的**唯一判据**——档位归属只在这
/// 一处定义，两列统计不各自为政。
pub(super) fn declared_tier(events: &[Event], turn: u64, before_seq: Option<u64>) -> String {
    events
        .iter()
        .rev()
        .find(|u| {
            u.entity == Entity::Turn
                && u.verb == Verb::Opened
                && u.turn == turn
                && u.kind == EVENT_USER_MESSAGE
                && u.seq.map(|s| s.value()) < before_seq
        })
        .and_then(|u| u.payload.get("tier").and_then(|v| v.as_str()))
        .and_then(crate::symbio_core::adapters::LatencyTier::from_name)
        .map(|t| t.name().to_string())
        .unwrap_or_else(|| "unspecified".to_string())
}

/// `fallback_rate` 投影：从事件切片统计各档位的兜底率。
pub fn fallback_rate() -> Projection<FallbackRateView> {
    Projection::new(|events: &[Event], now, _budget: Budget| {
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
                    if e.kind != EVENT_USER_MESSAGE {
                        continue;
                    }
                    // 档位 = 用户消息载荷 tier；缺失/未知进 unspecified（可观测）。
                    let tier = e
                        .payload
                        .get("tier")
                        .and_then(|v| v.as_str())
                        .and_then(crate::symbio_core::adapters::LatencyTier::from_name)
                        .map(|t| t.name().to_string())
                        .unwrap_or_else(|| "unspecified".to_string());
                    let st = by_tier.entry(tier.clone()).or_insert_with(|| TierStats {
                        tier,
                        turns: 0,
                        fallbacks: 0,
                    });
                    st.turns += 1;
                }
                Verb::Closed => {
                    if e.kind != EVENT_ASSISTANT_FALLBACK {
                        continue;
                    }
                    // 兜底落在哪档 = 本轮用户消息声明的档位（turn 号即归属）。
                    let tier = declared_tier(events, e.turn, e.seq.map(|s| s.value()));
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
        View {
            value: FallbackRateView { by_tier },
            degraded: false,
            used: Budget::new(0, 0),
        }
    })
}
