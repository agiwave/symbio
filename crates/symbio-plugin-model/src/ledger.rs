//! 本插件写的那两本账：`response{port=model}` 自带的 `usage` / `spend`。
//!
//! ## 为什么这两本账住 model 而不是读它的会话（D-461）
//!
//! 数据的**折叠跟着它的写方走**，出口跟着 `slot` 收（`D-387`）。这条不是偏好，
//! 是义务环判据的推论：一个元素若「读 A 写的载荷、在自己的插件里折成读数」，
//! 它在判定式四行里**一行都占不上**——不欠债（不是 decl）、不执行一笔债（不是
//! provider）、不是环的七职，只能降成数据。降成数据的第一条就是：**谁写谁折**，
//! 否则 A 改一次载荷形状，B 静默读错，而声明表校验不到这条跨插件的隐式契约
//! （`usage` / `spend` 不是任何 `FactDecl` 的字段，`reads` 查不出来）。
//!
//! 搬迁前：`model` 写载荷、`model` 的债行声明「进台账」（`accounts_usage` /
//! `accounts_spend` 两枚章）、**会话**折账、会话的 `stats` 出列。一件事两个插件分持。
//! 搬迁后四段全在本插件；会话的出口按 `slot` 收，本文件改动与它无关。
//!
//! ## 为什么是视图而不是新事实
//!
//! token 的产生点在 adapter 边界，已经随 `response` 事实入格；再写一条「用量事实」
//! 等于同一笔测量存两份。所以这里只有累计：同一份切片永远得出同一本账（N1），
//! as-of 与其余视图同口径。
//!
//! ## 「没测量」与「测到零」
//!
//! `usage` 缺席（provider 没给）与 `usage: { input: 0 }`（真测到 0）是两件事。
//! 本视图只累计**带 `usage` 键**的应答，`measured` 数的是它们——于是
//! 「本会话没有任何一次实测」读成 `measured: 0` + 三个零，与「每次都是零 token」
//! 由 `measured` 分辨。补 0 会把前者读成后者。
//!
//! ## 钱与 token 为什么两列
//!
//! 观察对象不同（token vs 钱）、归组键不同（内容主 vs 内容主 + 币种）、
//! 存在性条件不同（钱要 Provider 配了价格）。合成一列会得出「一条事件一个成本
//! 条目」这种既不真也不可用的形状。与 `cost.ledger`（时间）是三列，见该插件的
//! 模块文档。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use symbio_plugin_api::debt::{STAMP_KEY, STAMP_SPEND, STAMP_USAGE};
use symbio_plugin_api::reports::{ReportCtx, ReportDecl};
use symbio_plugin_api::{Event, ViewDecl, EVENT_RESPONSE};

/// 用量视图名。与事实名共用名字空间（视图是合成事实），全仓只出现一次（`SV-001`）。
pub(crate) const TOKEN_LEDGER: &str = "model.usage";

/// 金额视图名。
pub(crate) const SPEND_LEDGER: &str = "model.spend";

/// 扫描成本上界（事件条数）。声明表要求全仓同一个切点，故不按「台账通常很短」调小。
const COST: u32 = 4096;

// ── 用量 ──────────────────────────────────────────────────────────────────

/// 一个主体的累计用量。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TokenEntry {
    /// 内容的归属主体（应答载荷的 `principal`；缺席时退到事件的 `actor`）。
    pub principal: String,
    /// 累计输入 token。
    pub input: u64,
    /// 累计输出 token。
    pub output: u64,
    /// 带实测用量（`usage` 键）的应答条数——不是「这一轮发了几次请求」。
    pub measured: u64,
}

/// 用量视图：按主体字典序（BTreeMap ⇒ 逐字节确定）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TokenLedgerView {
    pub by_principal: BTreeMap<String, TokenEntry>,
    /// 全部主体合计。
    pub input: u64,
    pub output: u64,
    pub calls: u64,
}

// ── 金额 ──────────────────────────────────────────────────────────────────

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

/// 金额视图：按币种字典序（BTreeMap ⇒ 逐字节确定）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SpendLedgerView {
    pub by_currency: BTreeMap<String, CurrencySpend>,
}

// ── 求值 ──────────────────────────────────────────────────────────────────

/// 收束应答的公共扫描：`ts` / `kind` / 盖章 / 主体四处判据只在这里有一个答案。
///
/// 盖章是**债行**声明的（`accounts_usage` / `accounts_spend`），环原样盖上；
/// 读侧只认章与载荷，不比较端口名——否则每加一个世界端口都要在这里补一处分支。
pub(crate) fn stamped_responses<'a>(
    events: &'a [Event],
    now: i64,
    stamp: &'a str,
) -> impl Iterator<Item = (&'a Value, String)> + 'a {
    events.iter().filter_map(move |e| {
        if e.ts > now || e.kind != EVENT_RESPONSE {
            return None;
        }
        let payload = &e.payload;
        if !payload
            .get(STAMP_KEY)
            .and_then(|s| s.get(stamp))
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return None;
        }
        let principal = payload
            .get("principal")
            .and_then(|v| v.as_str())
            .unwrap_or(e.actor.as_str())
            .to_string();
        Some((payload, principal))
    })
}

/// 累计各主体实测的 token。只认 `response` 且盖了 `stamp.usage` 的那一批；
/// 没有 `usage` 数据的记录跳过（章只说明「本笔进这本账」，不伪造读数）。
fn eval_tokens(events: &[Event], now: i64) -> Value {
    let mut by_principal: BTreeMap<String, TokenEntry> = BTreeMap::new();
    let mut total_in = 0u64;
    let mut total_out = 0u64;
    let mut calls = 0u64;
    for (payload, principal) in stamped_responses(events, now, STAMP_USAGE) {
        let Some(usage) = payload.get("usage") else {
            continue;
        };
        let input = usage.get("input").and_then(Value::as_u64).unwrap_or(0);
        let output = usage.get("output").and_then(Value::as_u64).unwrap_or(0);
        let entry = by_principal.entry(principal.clone()).or_insert(TokenEntry {
            principal,
            ..Default::default()
        });
        entry.input += input;
        entry.output += output;
        entry.measured += 1;
        total_in += input;
        total_out += output;
        calls += 1;
    }
    serde_json::to_value(TokenLedgerView {
        by_principal,
        input: total_in,
        output: total_out,
        calls,
    })
    .unwrap_or(Value::Null)
}

/// 累计各币种、各主体的实测金额。只认 `response`、盖了 `stamp.spend`、且带
/// `spend` 对象的那一批。`spend` 缺 `amount_nano`（不是无符号整数）或缺非空
/// `currency` 的记录**跳过**：金额没有币种就没有可加性，猜一个桶比少一条账更糟。
fn eval_spend(events: &[Event], now: i64) -> Value {
    let mut by_currency: BTreeMap<String, CurrencySpend> = BTreeMap::new();
    for (payload, principal) in stamped_responses(events, now, STAMP_SPEND) {
        let Some(spend) = payload.get("spend") else {
            continue;
        };
        let Some(amount_nano) = spend.get("amount_nano").and_then(Value::as_u64) else {
            continue;
        };
        let Some(currency) = spend
            .get("currency")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let currency = currency.to_uppercase();
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
    serde_json::to_value(SpendLedgerView { by_currency }).unwrap_or(Value::Null)
}

// ── 声明 ──────────────────────────────────────────────────────────────────

/// 两条视图：`reads` 是空表，因为两者都只读**协议事实**的载荷——`response` 是环的
/// 事实名，没有 `FactDecl` 的字段表可校验（`D-346` 的线：不登记的格不装字段）。
///
/// 这不是「没声明所以随便读」：搬迁的意义正是**读方与写方同一个 crate**——
/// 载荷形状改一次，编译器与用例就在手边；跨插件的隐式契约因此收敛成插件内契约。
pub(crate) const VIEWS: &[ViewDecl] = &[
    ViewDecl {
        name: TOKEN_LEDGER,
        ver: 1,
        reads: &[],
        evidence: &[],
        cost: COST,
        eval: eval_tokens,
        agg: None,
    },
    ViewDecl {
        name: SPEND_LEDGER,
        ver: 1,
        reads: &[],
        evidence: &[],
        cost: COST,
        eval: eval_spend,
        agg: None,
    },
];

// 会话读数的两列：`usage` 与 `spend`。空形状住在 owner（本文件），读方不替它猜。
symbio_plugin_api::submit_reports!([
    ReportDecl {
        owner: "model",
        slot: "usage",
        order: 41,
        views: &[TOKEN_LEDGER],
        fleet: None,
        render: usage_report,
        fleet_render: None,
        panel: None,
    },
    ReportDecl {
        owner: "model",
        slot: "spend",
        order: 42,
        views: &[SPEND_LEDGER],
        fleet: None,
        render: spend_report,
        fleet_render: None,
        panel: None,
    },
]);

/// 用量列：视图原样交出；读不出来（读方以 `Null` 占位）时交 owner 的空账。
fn usage_report(views: &[Value], _ctx: &ReportCtx<'_>) -> Value {
    views
        .first()
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(
            || serde_json::json!({ "by_principal": {}, "input": 0, "output": 0, "calls": 0 }),
        )
}

/// 金额列：同上。空账是「这条事实源里没有一条带金额的往返」——有据的空。
fn spend_report(views: &[Value], _ctx: &ReportCtx<'_>) -> Value {
    views
        .first()
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(|| serde_json::json!({ "by_currency": {} }))
}

#[cfg(test)]
#[path = "ledger.test.rs"]
mod tests;
