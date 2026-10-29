//! 用量台账：`response{port=model}` 自带的 `usage` 按主体累计。
//!
//! ## 为什么住会话插件（DE-023 D-180）
//!
//! 这个视图的**唯一生产消费方**是 `crates/symbio-plugin-session/src/stats.rs` 的用量列。
//! 按 D-180「视图类型与投影函数跟消费方走」，它不该占内核的根出口——
//! 内核只提供 [`Projection`] 这个构造器（机制），一本账的口径归读它的插件。
//! 与 [`super::spend_ledger`] 同一次裁决：单消费方、无协议语义 ⇒ 下沉。
//!
//! ## 为什么与 `cost_ledger` 分开
//!
//! `cost_ledger` 读的是**事件的 `cost_ms`**（adapter 边界的耗时，按 `actor` 归组），
//! 它回答「这一轮花了多久」。本投影读的是**载荷里的 `usage`**（provider 实测的
//! token，按内容的 `principal` 归组），回答「这一次往返用了多少 token」。
//!
//! 两件事的**观察对象不同**（时间 vs token）、**归组键不同**（写方 vs 内容主）——
//! 合成一个视图就会得出「一条事件一个成本条目」这种既不真也不可用的形状。
//!
//! ## 为什么它是纯投影而不是一件新事实
//!
//! token 的产生点在 adapter 边界，已经随 `response` 事实入格；再写一条「用量事实」
//! 等于同一笔测量存两份。所以这里只有累计：同一份切片永远得出同一本账（N1），
//! as-of 与其余投影同口径。
//!
//! ## 「没测量」与「测到零」
//!
//! `usage` 缺席（provider 没给）与 `usage: { input: 0 }`（真测到 0）是两件事。
//! 本投影只累计**带 `usage` 键**的应答，`measured` 数的是它们——于是
//! 「本会话没有任何一次实测」读成 `measured: 0` + 三个零，与「每次都是零 token」
//! 由 `measured` 分辨。补 0 会把前者读成后者。

use symbio_core::{Event, Projection, EVENT_RESPONSE};

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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

/// 用量台账视图：按主体字典序（BTreeMap ⇒ 逐字节确定）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TokenLedgerView {
    pub by_principal: BTreeMap<String, TokenEntry>,
    /// 全部主体合计。
    pub input: u64,
    pub output: u64,
    pub calls: u64,
}

/// 用量台账投影：从事件切片累计各主体实测的 token。
///
/// 只认 `response` 且 `payload.port == "model"` 的那一批：`turn` / `tool` 端口的
/// 应答不带用量（它们的「代价」是别的东西，见各自的端口声明）。
pub fn token_ledger() -> Projection<TokenLedgerView> {
    Projection::new(|events: &[Event], now| {
        let mut by_principal: BTreeMap<String, TokenEntry> = BTreeMap::new();
        let mut total_in = 0u64;
        let mut total_out = 0u64;
        let mut calls = 0u64;
        for e in events {
            if e.ts > now || e.kind != EVENT_RESPONSE {
                continue;
            }
            let payload = &e.payload;
            if payload.get("port").and_then(|v| v.as_str()) != Some("model") {
                continue;
            }
            let Some(usage) = payload.get("usage") else {
                continue;
            };
            let input = usage.get("input").and_then(|v| v.as_u64()).unwrap_or(0);
            let output = usage.get("output").and_then(|v| v.as_u64()).unwrap_or(0);
            let principal = payload
                .get("principal")
                .and_then(|v| v.as_str())
                .unwrap_or(e.actor.as_str())
                .to_string();
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
        TokenLedgerView {
            by_principal,
            input: total_in,
            output: total_out,
            calls,
        }
    })
}
