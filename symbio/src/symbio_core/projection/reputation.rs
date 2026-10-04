//! `reputation` 投影（S6 第 15 步，[roadmap/S08 §2/§3](../../../../docs/plan/roadmap/S08-多主体与对等承诺.md)）。
//!
//! 声誉 = **一个投影**，不是一套机制：违约可被观测 ⇔ 承诺事件进 Log ⇔
//! 任何主体都可以从同一份事实源算出同一张声誉表（N1 双跑一致的天然受益者）。
//!
//! ## 边界（S08 §7 诚实划界）
//!
//! - 打分函数是**参数**不是架构：[`ReputationScore`] 给出平凡值（守约 − 违约），
//!   换算法不改本投影的形状；
//! - `commitment` 只记录「立了约、是否违约」，不保证「守约」；
//! - as-of 语义与 `recall` 同口径：形参 `now` 即锚，未来的收束不影响当前声誉。

use super::super::event::{
    Entity, Event, EVENT_COMMITMENT_BROKEN, EVENT_COMMITMENT_OFFERED, EVENT_COMMITMENT_RELEASED,
};
use super::super::view::{Budget, View};
use super::Projection;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 一个主体的声誉条目。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReputationEntry {
    /// 主体（`commitment.opened` 的 `from`——**声誉记在承诺方头上**）。
    pub principal: String,
    /// as-of 内立下的承诺数。
    pub offered: u64,
    /// 守约收束数。
    pub kept: u64,
    /// 违约收束数。
    pub broken: u64,
    /// 打分（平凡值：守约 − 违约）。
    pub score: i64,
}

/// 声誉视图：按主体排序（`BTreeMap` ⇒ 逐字节确定，N1 双跑一致的前提）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReputationView {
    /// principal → 条目。
    pub by_principal: BTreeMap<String, ReputationEntry>,
}

impl ReputationView {
    /// 某主体的声誉（未立约的主体 ⇒ 空条目，**不是错误**）。
    pub fn of(&self, principal: &str) -> ReputationEntry {
        self.by_principal
            .get(principal)
            .cloned()
            .unwrap_or(ReputationEntry {
                principal: principal.to_string(),
                offered: 0,
                kept: 0,
                broken: 0,
                score: 0,
            })
    }
}

/// 打分函数（平凡值：守约 − 违约；换算法 = 换这个闭包，不加机制）。
pub type ReputationScore = fn(offered: u64, kept: u64, broken: u64) -> i64;

/// 平凡打分：守约 − 违约。
pub fn plain_score(_offered: u64, kept: u64, broken: u64) -> i64 {
    kept as i64 - broken as i64
}

/// `reputation` 投影：从承诺事件算出各主体的声誉表。
///
/// - as-of：`ts > now` 的立约 / 收束不参与（与 `recall` 同口径）；
/// - 配对：收束事件经载荷 `id` 找回立约方（`from` 是立约时的事实，收束时不重抄）；
/// - 排序：`BTreeMap` 按主体字典序——同一份事件切片永远产出同一张表（N1）。
pub fn reputation() -> Projection<ReputationView> {
    reputation_with(plain_score)
}

/// 带自定义打分函数的声誉投影（算法是参数，架构只提供位置）。
pub fn reputation_with(score: ReputationScore) -> Projection<ReputationView> {
    Projection::new(move |events: &[Event], now, _budget: Budget| {
        let mut offered_of: BTreeMap<String, u64> = BTreeMap::new();
        // commitment id → (from, ts)
        let mut open: BTreeMap<String, (String, i64)> = BTreeMap::new();
        let mut kept_of: BTreeMap<String, u64> = BTreeMap::new();
        let mut broken_of: BTreeMap<String, u64> = BTreeMap::new();
        for e in events {
            if e.ts > now || e.entity != Entity::Commitment {
                continue;
            }
            let id = e
                .payload
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            match e.kind.as_str() {
                EVENT_COMMITMENT_OFFERED => {
                    let from = e
                        .payload
                        .get("from")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    *offered_of.entry(from.clone()).or_insert(0) += 1;
                    open.insert(id, (from, e.ts));
                }
                EVENT_COMMITMENT_RELEASED | EVENT_COMMITMENT_BROKEN => {
                    if let Some((from, _)) = open.get(&id) {
                        let from = from.clone();
                        if e.kind == EVENT_COMMITMENT_BROKEN {
                            *broken_of.entry(from).or_insert(0) += 1;
                        } else {
                            *kept_of.entry(from).or_insert(0) += 1;
                        }
                    }
                }
                _ => {}
            }
        }
        let mut by_principal = BTreeMap::new();
        for (principal, offered) in offered_of {
            let kept = *kept_of.get(&principal).unwrap_or(&0);
            let broken = *broken_of.get(&principal).unwrap_or(&0);
            by_principal.insert(
                principal.clone(),
                ReputationEntry {
                    principal,
                    offered,
                    kept,
                    broken,
                    score: score(offered, kept, broken),
                },
            );
        }
        View {
            value: ReputationView { by_principal },
            degraded: false,
            used: Budget::new(0, 0),
        }
    })
}
