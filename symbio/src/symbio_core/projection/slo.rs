//! `slo_report` 投影（SLO §1.2 时延列的统计口径，[plan/04 §1.2](../../../../docs/plan/04-工程落地.md)）。
//!
//! 时延样本 = `chat.assistant.final` 的 `cost_ms`（实测成功轮，[ADR-044](../../../../docs/decisions/core.md)：
//! 实测与判据同源——埋点在 adapter 边界，实测值随事件入账）。**兜底轮不计入
//! 时延样本**：兜底是失败路径，把它的耗时混进成功响应的分位数会同时污染两列
//! （时延被拉高、兜底率看不见）——失败轮的耗时记在 fallback 事件的 `cost_ms`
//! 里，由读方按需取用。
//!
//! 档位归属与 [`super::fallback::fallback_rate`] 同一判据（共享
//! [`declared_tier`] 单源）：final 按 turn 号归到本轮用户消息声明的档位；
//! 缺失/未知 ⇒ `unspecified` 桶（可观测，不静默归类）。
//!
//! 分位数是**读方计算**：视图只携带升序样本（构造即排序，逐字节确定，N1），
//! `percentile` 是纯查询——投影只报数，SLO 是承诺。

use super::super::event::{Entity, Event, Verb, EVENT_ASSISTANT_FINAL};
use super::super::view::{Budget, View};
use super::fallback::declared_tier;
use super::Projection;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 一个档位的时延样本（升序；`percentile` 的前提）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TierLatency {
    /// 档位名（四层之一，或 `unspecified`）。
    pub tier: String,
    /// 实测样本（毫秒，升序）。
    pub samples: Vec<u64>,
}

impl TierLatency {
    /// 分位数（`p` ∈ [0, 100]；无样本 ⇒ 0——不给空档位报警）。
    ///
    /// 最近邻取法（`ceil(p/100 × n) - 1`）：P50 = 中位、P100 = max，
    /// 与校准彩排同一取法。
    pub fn percentile(&self, p: u64) -> u64 {
        if self.samples.is_empty() || p == 0 {
            return 0;
        }
        let n = self.samples.len();
        let idx = ((p * n as u64).div_ceil(100) as usize).min(n) - 1;
        self.samples[idx]
    }

    /// P50。
    pub fn p50(&self) -> u64 {
        self.percentile(50)
    }

    /// P95。
    pub fn p95(&self) -> u64 {
        self.percentile(95)
    }

    /// P99。
    pub fn p99(&self) -> u64 {
        self.percentile(99)
    }

    /// 样本数。
    pub fn count(&self) -> usize {
        self.samples.len()
    }
}

/// 时延视图：按档位名字典序（BTreeMap ⇒ 逐字节确定，N1）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SloLatencyView {
    pub by_tier: BTreeMap<String, TierLatency>,
}

impl SloLatencyView {
    /// 某档位的样本（无数据 ⇒ 空账，不是错误）。
    pub fn of(&self, tier: &str) -> TierLatency {
        self.by_tier.get(tier).cloned().unwrap_or(TierLatency {
            tier: tier.to_string(),
            samples: Vec::new(),
        })
    }
}

/// `slo_report` 投影：从事件切片统计各档位的实测时延样本（final 的 cost_ms）。
pub fn slo_report() -> Projection<SloLatencyView> {
    Projection::new(|events: &[Event], now, _budget: Budget| {
        let mut by_tier: BTreeMap<String, Vec<u64>> = BTreeMap::new();
        for e in events {
            if e.ts > now {
                continue;
            }
            // 时延样本只收成功收束（final）；fallback 的耗时不是成功响应。
            if e.entity != Entity::Turn || e.verb != Verb::Closed || e.kind != EVENT_ASSISTANT_FINAL
            {
                continue;
            }
            let tier = declared_tier(events, e.turn, e.seq.map(|s| s.value()));
            by_tier.entry(tier).or_default().push(e.cost_ms);
        }
        let mut view = SloLatencyView::default();
        for (tier, mut samples) in by_tier {
            samples.sort_unstable();
            view.by_tier
                .insert(tier.clone(), TierLatency { tier, samples });
        }
        View {
            value: view,
            degraded: false,
            used: Budget::new(0, 0),
        }
    })
}

#[cfg(test)]
#[path = "slo.test.rs"]
mod tests;
