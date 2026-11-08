//! 本插件拥有的事实名、格位、归宿行与包内视图（DE-024 D-197 / DE-025 D-209 / D-221）。
//!
//! 一条事实 `idempotency.recorded`：**「这个键已经按这个值记过一次」**。名字、格位、
//! 载荷形状与归宿行同在这里，写方（[`super::behavior`]）与读方（[`IDEMPOTENCY_LEDGER`]
//! 视图）都从本模块取，不存在第二份拼写。
//!
//! ## 格位为什么是 `Domain × Asserted`
//!
//! `Domain` 是内核给「新领域」留的扩展位（D-209）：内核**不解释**这条事实，写闸只按
//! 本声明校验名字与格位。用 `Asserted` 而不是 `Opened`，因为这是「这个键被记过」的
//! 断言——它可以被读方引用，但不是一条新链路。
//!
//! ## 视图为什么也在这里
//!
//! [`IDEMPOTENCY_LEDGER`] 是纯折叠：从 `idempotency.recorded` 按键去重折出台账。它只在
//! 本包内折，名字不进 `PLUGIN.yml` 的 `shared_views:`：今天没有第二个读者（`D-221`）。

use serde_json::{json, Value};

use symbio_plugin_api::ring::{Decl, Pattern};
use symbio_plugin_api::{
    Entity, Event, EventDecl, FactDecl, LatencyTier, PluginDeclarations, Verb, ViewDecl,
};

/// 「这个键已经记过一次」这一断言。落在 `domain × asserted`。
pub(crate) const IDEMPOTENCY_RECORDED: &str = "idempotency.recorded";

/// 本插件的 LLM 可见工具名。工具面与行为声明都取它，不存在第二份拼写。
pub(crate) const IDEMPOTENCY_TOOL: &str = "idempotent_record";

/// 包内视图：按键去重后的幂等台账；名字不公开（没有第二个读者）。
pub(crate) const IDEMPOTENCY_LEDGER: &str = "idempotency.ledger";

/// 视图最多保留的条数。超出只影响**输出长度**，不影响扫描成本上界。
const LIMIT: usize = 50;

/// 扫描成本上界（事件条数）。它是环内窗口求值的预算；跨插件读口不查它——读侧按
/// 调用方给的切片全量折（`D-284`）。4096 覆盖通常的会话快照。
const COST: u32 = 4096;

/// owner —— 与目录名同形（`crates/symbio-plugin-idempotency`），装配表按它排序与报缺口。
pub(crate) const OWNER: &str = "crates/symbio-plugin-idempotency";

pub(crate) const EVENTS: &[EventDecl] = &[EventDecl::new(
    IDEMPOTENCY_RECORDED,
    (Entity::Domain, Verb::Asserted),
)];

pub(crate) const FACTS: &[FactDecl] = &[FactDecl {
    name: IDEMPOTENCY_RECORDED,
    port: None,
    key: &[],
    expand: &[],
    fields: &["key", "value"],
}];

/// 本插件事实的**归宿行**：`owes` 与 `until` 同源——有记录即闭合，没有则不欠。
///
/// 没有它，`seal` 会把 `idempotency.recorded` 判成 `OrphanFact`（每条事实都要有归宿行）。
pub(crate) const ROWS: &[Decl] = &[Decl {
    name: IDEMPOTENCY_RECORDED,
    by: None,
    tier: LatencyTier::Reflex,
    expand: None,
    owes: Pattern {
        source: IDEMPOTENCY_RECORDED,
        key: &[],
        eq: &[],
    },
    until: &[Pattern {
        source: IDEMPOTENCY_RECORDED,
        key: &[],
        eq: &[],
    }],
    unless: &[],
}];

pub(crate) const VIEWS: &[ViewDecl] = &[ViewDecl {
    name: IDEMPOTENCY_LEDGER,
    ver: 1,
    reads: &[
        (IDEMPOTENCY_RECORDED, "key"),
        (IDEMPOTENCY_RECORDED, "value"),
    ],
    // 读出口径不是收束判据：本视图不替任何事实关门。
    evidence: &[],
    cost: COST,
    eval: idempotency_ledger,
    agg: None,
}];

/// 纯折叠：`idempotency.recorded` → `{ count, entries: [{ key, value }], keys: [...] }`。
///
/// **首写为准**：同一个键只留第一次记录的那条。写入侧本就在 `before` 拒重复，折叠再按
/// 键去重是第二条防线——它让「同轮同键由写口去重」的形态在读数上也只算一格。
///
/// 顺序按记录序（写入序），超过 [`LIMIT`] 取最后若干条——台账里最新的键最有价值。
fn idempotency_ledger(events: &[Event], now: i64) -> Value {
    let mut entries: Vec<Value> = Vec::new();
    let mut seen: Vec<&str> = Vec::new();

    for e in events {
        if e.kind != IDEMPOTENCY_RECORDED || e.ts > now {
            continue;
        }
        let (Some(key), Some(value)) = (
            e.payload.get("key").and_then(Value::as_str),
            e.payload.get("value").and_then(Value::as_str),
        ) else {
            continue;
        };
        if seen.contains(&key) {
            continue;
        }
        seen.push(key);
        entries.push(json!({ "key": key, "value": value }));
    }

    if entries.len() > LIMIT {
        let drop = entries.len() - LIMIT;
        entries.drain(..drop);
    }

    let keys: Vec<&str> = entries
        .iter()
        .filter_map(|entry| entry.get("key").and_then(Value::as_str))
        .collect();

    json!({ "count": entries.len(), "entries": entries, "keys": keys })
}

// 装配口的提交：宿主链接本 crate 即生效。本插件不认识也不依赖任何兄弟插件。
symbio_plugin_api::submit_declarations!(OWNER, submitted_declarations);

fn submitted_declarations() -> PluginDeclarations<'static> {
    PluginDeclarations {
        events: EVENTS,
        facts: FACTS,
        views: VIEWS,
        ports: &[],
        actors: Vec::new(),
        rows: ROWS,
    }
}

#[cfg(test)]
#[path = "decl.test.rs"]
mod tests;
