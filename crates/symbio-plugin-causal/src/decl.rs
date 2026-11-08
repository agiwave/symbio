//! 本插件拥有的事实名、格位、归宿行与包内视图（DE-024 D-197 / DE-025 D-209 / D-221）。
//!
//! 一条事实 `causal.link`：**「A 导致 B」这件事被断言过**。名字、格位、载荷形状与
//! 归宿行同在这里，写方（[`super::behavior`]）与读方（[`CAUSAL_LINKS`] 视图）都从本
//! 模块取，不存在第二份拼写。
//!
//! ## 格位为什么是 `Domain × Asserted`
//!
//! `Domain` 是内核给「新领域」留的扩展位（D-209）：因果不是内核认识的核心语义，
//! 内核**不解释**这条事实，写闸只按本声明校验名字与格位。用 `Asserted` 而不是
//! `Opened`，因为这**不是**开了一条新链路——它是「我断言这两件事之间有因果关系」，
//! 一条可以被后来更强的证据压过的断言。两者的区别在重放时就是可读的。
//!
//! ## 视图为什么也在这里
//!
//! [`CAUSAL_LINKS`] 是纯折叠：从 `causal.link` 折出「因 → 果」的邻接表与根因。
//! 它只在本包内折，名字不进 `PLUGIN.yml` 的 `shared_views:`：今天没有第二个读者
//! （`D-221` 的公开动作只对真有读方的视图做）。名字、版本、成本与算法只有这里一个来源。

use serde_json::{json, Value};

use symbio_plugin_api::ring::{Decl, Pattern};
use symbio_plugin_api::{
    Entity, Event, EventDecl, FactDecl, LatencyTier, PluginDeclarations, Verb, ViewDecl,
};

/// 「A 导致 B」这一断言。落在 `domain × asserted`。
pub(crate) const CAUSAL_LINK: &str = "causal.link";

/// 本插件的 LLM 可见工具名。工具面与行为声明的连接都取它，不存在第二份拼写。
pub(crate) const CAUSAL_TOOL: &str = "causal_assert";

/// 包内视图：因果图的读回（邻接 + 根因 + 计数）；名字不公开（没有第二个读者）。
pub(crate) const CAUSAL_LINKS: &str = "causal.links";

/// 视图最多保留的条数。超出只影响**输出长度**，不影响扫描成本上界。
const LIMIT: usize = 50;

/// 扫描成本上界（事件条数）。它是环内窗口求值的预算；跨插件读口不查它——读侧按
/// 调用方给的切片全量折（`D-284`）。4096 覆盖通常的会话快照。
const COST: u32 = 4096;

/// owner —— 与目录名同形（`crates/symbio-plugin-causal`），装配表按它排序与报缺口。
pub(crate) const OWNER: &str = "crates/symbio-plugin-causal";

pub(crate) const EVENTS: &[EventDecl] = &[EventDecl::new(
    CAUSAL_LINK,
    (Entity::Domain, Verb::Asserted),
)];

pub(crate) const FACTS: &[FactDecl] = &[FactDecl {
    name: CAUSAL_LINK,
    port: None,
    key: &[],
    expand: &[],
    // `confidence` 是 0–100 的整数（写方与读方共用同一口径，见 `behavior::link_draft`）。
    fields: &["cause", "effect", "confidence"],
}];

/// 本插件事实的**归宿行**：`owes` 与 `until` 同源——有断言即闭合，没有则不欠。
///
/// 行表是义务，不是触发器；这条自读行只回答「这条事实归谁看护」。没有它，`seal`
/// 会把 `causal.link` 判成 `OrphanFact`（每条事实都要有归宿行）。
pub(crate) const ROWS: &[Decl] = &[Decl {
    name: CAUSAL_LINK,
    by: None,
    tier: LatencyTier::Reflex,
    expand: None,
    owes: Pattern {
        source: CAUSAL_LINK,
        key: &[],
        eq: &[],
    },
    until: &[Pattern {
        source: CAUSAL_LINK,
        key: &[],
        eq: &[],
    }],
    unless: &[],
}];

pub(crate) const VIEWS: &[ViewDecl] = &[ViewDecl {
    name: CAUSAL_LINKS,
    ver: 1,
    reads: &[(CAUSAL_LINK, "cause"), (CAUSAL_LINK, "effect")],
    // 读出口径不是收束判据：本视图不替任何事实关门。
    evidence: &[],
    cost: COST,
    eval: causal_graph,
    agg: None,
}];

/// 纯折叠：`causal.link` → `{ count, edges: [{ from, to, confidence }], causes: [...] }`。
///
/// `causes` 是**只出不进**的节点（图上没有入边的起点）。它不是「根本原因」的判定——
/// 那是读方结合自己的领域知识要做的事（本插件不认识任何领域）；这里只交出图的结构
/// 事实：哪些节点在这里没有上游。把结构说成结论，是这类视图最容易犯的错。
///
/// 顺序按断言序（写入序），超过 [`LIMIT`] 取最后若干条——图里最新的断言最有价值，
/// 早期断言被后来的修正压过时，读方看到的是修正后的版本。
fn causal_graph(events: &[Event], now: i64) -> Value {
    let mut edges: Vec<Value> = Vec::new();
    let mut froms: Vec<String> = Vec::new();
    let mut tos: Vec<String> = Vec::new();

    for e in events {
        if e.kind != CAUSAL_LINK || e.ts > now {
            continue;
        }
        let (Some(cause), Some(effect)) = (
            e.payload.get("cause").and_then(Value::as_str),
            e.payload.get("effect").and_then(Value::as_str),
        ) else {
            continue;
        };
        let confidence = e
            .payload
            .get("confidence")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        edges.push(json!({ "from": cause, "to": effect, "confidence": confidence }));
        froms.push(cause.to_string());
        tos.push(effect.to_string());
    }

    if edges.len() > LIMIT {
        let drop = edges.len() - LIMIT;
        edges.drain(..drop);
    }

    // 只出不进 = 出现在 `from` 里、且**不在**任何 `to` 里。集合成员资格是结构事，
    // 不做权重排序：排序会替读方挑重点，而重点取决于它要回答的问题。
    let mut causes: Vec<Value> = Vec::new();
    let mut seen: Vec<&str> = Vec::new();
    for (i, cause) in froms.iter().enumerate() {
        let _ = i;
        if tos.contains(cause) || seen.contains(&cause.as_str()) {
            continue;
        }
        seen.push(cause.as_str());
        causes.push(json!(cause));
    }

    json!({ "count": edges.len(), "edges": edges, "causes": causes })
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
