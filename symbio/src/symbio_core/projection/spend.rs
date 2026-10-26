//! 金额台账：`response{port=model}` 自带的 `spend` 按**币种**分桶、桶内按主体累计。
//!
//! ## 为什么与 [`super::token`] / [`super::cost`] 都是分开的
//!
//! 三者各回答一件事，观察对象不同：
//!
//! | 投影 | 读什么 | 回答 |
//! |---|---|---|
//! | `cost_ledger` | 事件的 `cost_ms`（按写方） | 这一轮花了多少时间 |
//! | `token_ledger` | 应答载荷的 `usage`（按内容主） | 用了多少 token |
//! | `spend_ledger` | 应答载荷的 `spend`（按币种 + 内容主） | 花了多少钱 |
//!
//! 时间、token、钱三者没有换算关系（价格是配置，不是常量），合成一列就会让
//! 一个读数冒充另一个。
//!
//! ## 为什么按币种分桶
//!
//! 金额只有在**同一币种**下才可加。用户可以在两个端点用两种计价币种——把它们
//! 加成一个数需要汇率，而汇率不是这份事实源里的东西。桶内再按主体摊开
//! （子智能体 / 委派各花了多少），键序与同族投影一致（BTreeMap ⇒ 逐字节确定）。
//!
//! ## 「没定价」与「明确免费」
//!
//! 写方只在实际折算过金额时才写 `spend` 键（价格缺席不写、没有实测用量也不写）。
//! 本投影只累计**带 `spend`** 的应答，`charges` 数的是它们——于是
//! 「本会话没有价格配置」读成 `by_currency: {}`，与「每次都是 0 元」由
//! `charges` 分辨。补 0 会把前者读成后者（与 `token_ledger` 的 `measured` 同源）。

use super::super::event::Event;
use super::Projection;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 一个主体在一种币种下的累计金额。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SpendEntry {
    /// 内容的归属主体（应答载荷的 `principal`；缺席时退到事件的 `actor`）。
    pub principal: String,
    /// 累计金额，单位 1e-9 个货币单位。
    pub amount_nano: u64,
    /// 带金额的应答条数——不是「这一轮发了几次请求」。
    pub charges: u64,
}

/// 一种币种的累计金额（含按主体的摊开）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CurrencySpend {
    /// 币种代码（事实里已归一成大写）。
    pub currency: String,
    /// 本币种累计金额。
    pub amount_nano: u64,
    /// 带金额的应答条数。
    pub charges: u64,
    /// 按主体摊开（`amount_nano` 之和 = 本桶的 `amount_nano`）。
    pub by_principal: BTreeMap<String, SpendEntry>,
}

/// 金额台账视图：按币种字典序（BTreeMap ⇒ 逐字节确定）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SpendLedgerView {
    pub by_currency: BTreeMap<String, CurrencySpend>,
}

/// 金额台账投影：从事件切片累计各币种、各主体的实测金额。
///
/// 只认 `response` 且 `payload.port == "model"`、且带 `spend` 对象的那一批。
/// `spend` 缺 `amount_nano`（不是无符号整数）或缺非空 `currency` 的记录**跳过**：
/// 金额没有币种就没有可加性，猜一个桶比少一条账更糟（写方保证形状，见
/// `turn_runner/facts.rs`）。
pub fn spend_ledger() -> Projection<SpendLedgerView> {
    Projection::new(|events: &[Event], now| {
        let mut by_currency: BTreeMap<String, CurrencySpend> = BTreeMap::new();
        for e in events {
            if e.ts > now || e.kind != crate::symbio_core::EVENT_RESPONSE {
                continue;
            }
            let payload = &e.payload;
            if payload.get("port").and_then(|v| v.as_str()) != Some("model") {
                continue;
            }
            let Some(spend) = payload.get("spend") else {
                continue;
            };
            let Some(amount_nano) = spend.get("amount_nano").and_then(|v| v.as_u64()) else {
                continue;
            };
            let Some(currency) = spend
                .get("currency")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            else {
                continue;
            };
            let currency = currency.to_uppercase();
            let principal = payload
                .get("principal")
                .and_then(|v| v.as_str())
                .unwrap_or(e.actor.as_str())
                .to_string();
            let bucket = by_currency
                .entry(currency.clone())
                .or_insert_with(|| CurrencySpend {
                    currency,
                    ..Default::default()
                });
            bucket.amount_nano = bucket.amount_nano.saturating_add(amount_nano);
            bucket.charges += 1;
            let entry = bucket
                .by_principal
                .entry(principal.clone())
                .or_insert_with(|| SpendEntry {
                    principal,
                    ..Default::default()
                });
            entry.amount_nano = entry.amount_nano.saturating_add(amount_nano);
            entry.charges += 1;
        }
        SpendLedgerView { by_currency }
    })
}

#[cfg(test)]
#[path = "spend.test.rs"]
mod tests;
