//! 成本台账能力包的那一条视图（名册 #18，DE-039 §3 的 `projection = budget`）。
//!
//! ## 折出来的是什么
//!
//! [`COST_LEDGER`] 把一份事实切片按**事件写出方**累计成一本账：
//!
//! ```text
//! { "by_principal": { "<主体>": { principal, spent_ms, entries }, … 按主体字典序 },
//!   "total_ms": <全局累计> }
//! ```
//!
//! `spent_ms` 是实测耗时（适配器边界写在事件信封上的 `cost_ms`），`entries` 是笔数
//! （记账透明度：花了多少 = 几笔账加出来的）。未记账的主体读成**零账**而不是错误——
//! 「还没记过账」与「读不出来」是两件事，后者由读口的缺席态承担。
//!
//! ## 为什么 `reads` 是空表
//!
//! 它只读**事件信封**的 `ts` / `actor` / `cost_ms`，一个载荷字段都不碰：`cost_ms` 不是某族
//! 事实的载荷字段，它是每条事件自带的计量列（与 `slo.span` 只读 `ts` 同一形状）。所以
//! `reads: &[]` 是合法声明（`D-282` 那条通道也不触发——没有任何载荷字段可校验）。
//!
//! ## 为什么没有 `agg` 那一半
//!
//! `D-281` 的合并服务于**实例级读数**（分位数 / 率 / 中位数），而成本台账今天没有实例级
//! 的那一列：闸门要的是「这个主体在这会话里花掉多少」，出口 `stats.cost` 是会话级的书。
//! 跨会话合成本台账会得出「整个实例花过多少毫秒」，而它没有读者——交一份没人读的合并
//! 函数只是多一处口径。真要那一列时，`agg` 与它的读方同批落地。

use std::collections::BTreeMap;

use serde_json::{json, Value};

use symbio_plugin_api::reports::{ReportCtx, ReportDecl};
use symbio_plugin_api::{Event, PluginDeclarations, ViewDecl};

/// 对外可读的那条视图。名字只在本 crate 出现一次（`SV-001`）；公开动作是下面的
/// 报表登记（`D-387`），读方按报表的 `views` 取，不依赖本 crate（`D-221`）。
pub(crate) const COST_LEDGER: &str = "cost.ledger";

/// owner —— 与目录名同形（`crates/symbio-plugin-cost`），装配表按它排序与报缺口。
pub(crate) const OWNER: &str = "crates/symbio-plugin-cost";

/// 扫描成本上界（事件条数）。它是环内窗口求值的预算；跨插件读口不查它——读侧按
/// 调用方给的切片全量折（`D-284`）。
///
/// 声明的上界必须与全仓**同一个值**（`seal` 的 `ViewCostMismatch`：切点只有一个），
/// 所以这里不按「台账通常很短」调小——调小会改整张表的切点。
const COST: u32 = 4096;

pub(crate) const VIEWS: &[ViewDecl] = &[ViewDecl {
    name: COST_LEDGER,
    ver: 1,
    // 只读信封：`ts`（as-of）、`actor`（归账的主体）、`cost_ms`（实测耗时）。
    reads: &[],
    // 台账不替任何事实关门：它是读数，不是证据。
    evidence: &[],
    cost: COST,
    eval: ledger,
    agg: None,
}];

/// 按主体累计 `cost_ms`。纯函数 ⇒ 同一切片永远同一本账（N1）；as-of 与其余读数同口径
/// （`now` 之后的事件不算）。
///
/// `cost_ms == 0` 不进账：桩与零延迟路径的诚实值就是 0，把它记成一笔会让 `entries` 读出
/// 「花过钱而金额为零」的假账——与搬出前 `Projection<CostLedgerView>` 的算式逐字相同。
fn ledger(events: &[Event], now: i64) -> Value {
    let mut rows: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    let mut total_ms = 0u64;
    for e in events {
        if e.ts > now || e.cost_ms == 0 {
            continue;
        }
        let row = rows.entry(e.actor.clone()).or_default();
        row.0 += e.cost_ms;
        row.1 += 1;
        total_ms += e.cost_ms;
    }
    let by_principal: serde_json::Map<String, Value> = rows
        .into_iter()
        .map(|(principal, (spent_ms, entries))| {
            (
                principal.clone(),
                json!({ "principal": principal, "spent_ms": spent_ms, "entries": entries }),
            )
        })
        .collect();
    json!({ "by_principal": Value::Object(by_principal), "total_ms": total_ms })
}

// 报表登记（`D-387`）：`session/stats` 的 `cost` 顶层键。`views` 是 owner 自己的
// 常量；空形状留在 [`report`]，读方不再替 owner 猜形状。
symbio_plugin_api::submit_reports!([ReportDecl {
    owner: "cost",
    slot: "cost",
    order: 40,
    views: &[COST_LEDGER],
    fleet: None,
    render: report,
    fleet_render: None,
    panel: None,
}]);

/// 成本列的排版：视图原样交出；读不出来（读方以 `Null` 占位）时交 owner 的空账。
fn report(views: &[Value], _ctx: &ReportCtx<'_>) -> Value {
    views
        .first()
        .filter(|value| !value.is_null())
        .cloned()
        .unwrap_or_else(|| json!({ "by_principal": {}, "total_ms": 0 }))
}

// 装配口的提交：宿主链接本 crate 即生效。本插件不认识也不依赖任何兄弟插件。
symbio_plugin_api::submit_declarations!(OWNER, submitted_declarations);

fn submitted_declarations() -> PluginDeclarations<'static> {
    PluginDeclarations {
        events: &[],
        facts: &[],
        views: VIEWS,
        ports: &[],
        actors: Vec::new(),
        rows: &[],
    }
}

#[cfg(test)]
#[path = "decl.test.rs"]
mod tests;
