//! 成本台账（S8 第 20 步，[roadmap/S09 §3](../../../../docs/plan/roadmap/S09-外部执行与熔断.md)
//! 的 `projection = budget` 参数；实现名 `cost_ledger`——`budget` 这个词让给
//! view 域的 [`crate::symbio_core::view::Budget`]（记账口径），这里只做累计视图）。
//!
//! 熔断闸门的判据来源：**已耗多少**从同一份事实源算出——事件自带的 `cost_ms`
//! 按主体累计。as-of 与其余投影同口径；纯函数 ⇒ 同一切片永远同一本账（N1）。

use super::super::event::Event;
use super::super::view::{Budget, View};
use super::Projection;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 一个主体的累计成本。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CostEntry {
    /// 主体（事件的 `actor`）。
    pub principal: String,
    /// 累计 `cost_ms`。
    pub spent_ms: u64,
    /// 计费事件数（记账透明度：花了多少钱 = 几笔账加出来的）。
    pub entries: u64,
}

/// 成本台账视图：按主体字典序（BTreeMap ⇒ 逐字节确定）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CostLedgerView {
    pub by_principal: BTreeMap<String, CostEntry>,
    /// 全局累计（`cost_ms` 之和）。
    pub total_ms: u64,
}

impl CostLedgerView {
    /// 某主体的累计（未记账主体 ⇒ 零账，不是错误）。
    pub fn of(&self, principal: &str) -> CostEntry {
        self.by_principal
            .get(principal)
            .cloned()
            .unwrap_or_default()
    }
}

/// 成本台账投影：从事件切片累计各主体的 `cost_ms`。
pub fn cost_ledger() -> Projection<CostLedgerView> {
    Projection::new(|events: &[Event], now, _budget: Budget| {
        let mut by_principal: BTreeMap<String, CostEntry> = BTreeMap::new();
        let mut total_ms = 0u64;
        for e in events {
            if e.ts > now || e.cost_ms == 0 {
                continue;
            }
            let entry = by_principal
                .entry(e.actor.clone())
                .or_insert_with(|| CostEntry {
                    principal: e.actor.clone(),
                    spent_ms: 0,
                    entries: 0,
                });
            entry.spent_ms += e.cost_ms;
            entry.entries += 1;
            total_ms += e.cost_ms;
        }
        View {
            value: CostLedgerView {
                by_principal,
                total_ms,
            },
            degraded: false,
            used: Budget::new(0, 0),
        }
    })
}
