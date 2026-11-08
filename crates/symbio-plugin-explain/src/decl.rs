//! 本插件拥有的事实名、格位、归宿行与包内视图（DE-024 D-197 / DE-025 D-209 / D-221）。
//!
//! 一条事实 `explain.rationale`：**「这次决定为什么这样做」被记下了**。名字、格位、载荷
//! 形状与归宿行同在这里，写方（[`super::behavior`]）与读方（[`EXPLAIN_RATIONALES`] 视图）
//! 都从本模块取，不存在第二份拼写。
//!
//! ## 格位为什么是 `Domain × Asserted`
//!
//! `Domain` 是内核给「新领域」留的扩展位（D-209）：可解释不是内核认识的核心语义，
//! 内核**不解释**这条事实，写闸只按本声明校验名字与格位。用 `Asserted` 而不是
//! `Opened`，因为这是「我主张这次决定的理由是这个」，一条可以被后来更准确的记录
//! 压过的断言，而不是开了一条新链路。
//!
//! ## 视图为什么也在这里
//!
//! [`EXPLAIN_RATIONALES`] 是纯折叠：从 `explain.rationale` 折出理由台账与去重后的
//! 决定列表。它只在本包内折，名字不进任何报表登记：今天没有第二个
//! 读者（`D-221` 的公开动作只对真有读方的视图做）。名字、版本、成本与算法只有这里一个来源。

use serde_json::{json, Value};

use symbio_plugin_api::ring::{Decl, Pattern};
use symbio_plugin_api::{
    Entity, Event, EventDecl, FactDecl, LatencyTier, PluginDeclarations, Verb, ViewDecl,
};

/// 「这次决定的理由」这一断言。落在 `domain × asserted`。
pub(crate) const EXPLAIN_RATIONALE: &str = "explain.rationale";

/// 本插件的 LLM 可见工具名。工具面与行为声明的连接都取它，不存在第二份拼写。
pub(crate) const EXPLAIN_TOOL: &str = "explain_record";

/// 包内视图：理由台账的读回（条目 + 去重决定 + 计数）；名字不公开（没有第二个读者）。
pub(crate) const EXPLAIN_RATIONALES: &str = "explain.rationales";

/// 视图最多保留的条数。超出只影响**输出长度**，不影响扫描成本上界。
const LIMIT: usize = 50;

/// 扫描成本上界（事件条数）。它是环内窗口求值的预算；跨插件读口不查它——读侧按
/// 调用方给的切片全量折（`D-284`）。4096 覆盖通常的会话快照。
const COST: u32 = 4096;

/// owner —— 与目录名同形（`crates/symbio-plugin-explain`），装配表按它排序与报缺口。
pub(crate) const OWNER: &str = "crates/symbio-plugin-explain";

pub(crate) const EVENTS: &[EventDecl] = &[EventDecl::new(
    EXPLAIN_RATIONALE,
    (Entity::Domain, Verb::Asserted),
)];

pub(crate) const FACTS: &[FactDecl] = &[FactDecl {
    name: EXPLAIN_RATIONALE,
    port: None,
    key: &[],
    expand: &[],
    // `evidence` 是可选观测（缺省空串）；写方与读方共用同一口径（见 `behavior::triple`）。
    fields: &["decision", "why", "evidence"],
}];

/// 本插件事实的**归宿行**：`owes` 与 `until` 同源——有记录即闭合，没有则不欠。
///
/// 行表是义务，不是触发器；这条自读行只回答「这条事实归谁看护」。没有它，`seal`
/// 会把 `explain.rationale` 判成 `OrphanFact`（每条事实都要有归宿行）。
pub(crate) const ROWS: &[Decl] = &[Decl {
    name: EXPLAIN_RATIONALE,
    by: None,
    tier: LatencyTier::Reflex,
    expand: None,
    owes: Pattern {
        source: EXPLAIN_RATIONALE,
        key: &[],
        eq: &[],
    },
    until: &[Pattern {
        source: EXPLAIN_RATIONALE,
        key: &[],
        eq: &[],
    }],
    unless: &[],
}];

pub(crate) const VIEWS: &[ViewDecl] = &[ViewDecl {
    name: EXPLAIN_RATIONALES,
    ver: 1,
    reads: &[(EXPLAIN_RATIONALE, "decision"), (EXPLAIN_RATIONALE, "why")],
    // 读出口径不是收束判据：本视图不替任何事实关门。
    evidence: &[],
    cost: COST,
    eval: rationale_ledger,
    agg: None,
}];

/// 纯折叠：`explain.rationale` → `{ count, entries: [{ decision, why, evidence }], decisions: [...] }`。
///
/// `decisions` 是 `entries` 里**去重后的决定短句列表**，按写入序保留首次出现。它不是
/// 「哪些决定被解释过」之外的判断——本插件不认识任何领域，只交记录与读数。
///
/// 顺序按记录序（写入序），超过 [`LIMIT`] 取最后若干条——台账里最新的理由最有价值，
/// 早期记录被后来的修正压过时，读方看到的是修正后的版本。
fn rationale_ledger(events: &[Event], now: i64) -> Value {
    let mut entries: Vec<Value> = Vec::new();

    for e in events {
        if e.kind != EXPLAIN_RATIONALE || e.ts > now {
            continue;
        }
        let (Some(decision), Some(why)) = (
            e.payload.get("decision").and_then(Value::as_str),
            e.payload.get("why").and_then(Value::as_str),
        ) else {
            continue;
        };
        let evidence = e
            .payload
            .get("evidence")
            .and_then(Value::as_str)
            .unwrap_or("");
        entries.push(json!({ "decision": decision, "why": why, "evidence": evidence }));
    }

    if entries.len() > LIMIT {
        let drop = entries.len() - LIMIT;
        entries.drain(..drop);
    }

    let mut decisions: Vec<String> = Vec::new();
    for entry in &entries {
        if let Some(decision) = entry.get("decision").and_then(Value::as_str) {
            if !decisions.iter().any(|seen| seen.as_str() == decision) {
                decisions.push(decision.to_string());
            }
        }
    }

    json!({ "count": entries.len(), "entries": entries, "decisions": decisions })
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
