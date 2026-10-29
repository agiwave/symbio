//! 本插件拥有的事件名、事实形状与对外可读的视图（DE-024 D-197 / DE-025 D-209 / D-221）。
//!
//! `notes.created` 是本插件**唯一**的事实名：名字、格位与载荷形状同在这里，写方
//! （[`super::behavior`]）与读方（[`super::context`] / [`NOTES_RECENT`]）都从本模块取，
//! 不存在第二份拼写。
//!
//! 格位是 `Domain × Opened`——`Domain` 是内核为「新领域」留的扩展位（D-209）；
//! `core` 对这条事实**不解释**，写闸只按本声明校验名字与格位（放行靠声明，理解靠 owner）。
//!
//! ## 视图为什么也在这里
//!
//! [`NOTES_RECENT`] 是本插件交出的纯折叠：从 `notes.created` 折出最近几条便签。
//! 它在 `PLUGIN.yml` 的 `shared_views:` 里登记为**对外可读**，别的插件经
//! `symbio_plugin_api::views` 按名字读——不依赖本 crate，也不在 core 里再写一份
//! （DE-025 D-221）。名字、版本、成本与算法只有这里一个来源。

use serde_json::{json, Value};

use symbio_core::ring::{Decl, Lit, Pattern};
use symbio_core::{
    Entity, Event, EventDecl, FactDecl, LatencyTier, PluginDeclarations, Verb, ViewDecl,
};
use symbio_plugin_api::events::EVENT_ARTIFACT_ADDED;

/// 「记下一条便签」这一事实。落在 `domain × opened`。
pub(crate) const NOTES_CREATED: &str = "notes.created";

/// 本插件的 LLM 可见工具名。工具面与行表（`notes.index` 的 `owes` 收窄）都取它，
/// 不存在第二份拼写。
pub(crate) const NOTES_TOOL: &str = "notes_add";

/// 轮内索引事实：`notes_add` 的产物落格后，本能力的一行把这条结果再记一笔。
///
/// 它与 [`NOTES_CREATED`] 的分工：后者是「用户让我记」的领域事实（轮末由行为落），
/// 前者是「这次工具结果已被本能力处理」的派生事实（轮内由通用行执行体落）。
pub(crate) const NOTES_INDEXED: &str = "notes.indexed";

/// 行名：把一条成功的 `notes_add` 产物收进索引。`by = plugin_row`（D-241）。
pub(crate) const NOTES_INDEX: &str = "notes.index";

/// 轮内反思事实：本能力对一条已索引便签的洞察（D-242 的模型往返产物）。
pub(crate) const NOTES_INSIGHT: &str = "notes.insight";

/// 行名：对一条已索引便签做一次模型往返，落一条洞察。`by = plugin_row`（D-242）。
pub(crate) const NOTES_REFLECT: &str = "notes.reflect";

/// 对外可读的视图：本能力在轮内反思出的洞察。
pub(crate) const NOTES_INSIGHTS: &str = "notes.insights";

/// 对外可读的视图：最近几条便签（新→旧）。
pub(crate) const NOTES_RECENT: &str = "notes.recent";

/// 对外可读的视图：本能力在轮内索引过的工具结果。
pub(crate) const NOTES_DIGEST: &str = "notes.digest";

/// 视图最多保留的条数。超出只影响**输出长度**，不影响扫描成本上界。
const RECENT_LIMIT: usize = 20;

/// 扫描成本上界（事件条数）。与义务环的视图同一口径：超界不静默截断，读方拿到
/// `OverBudget`。4096 覆盖通常的会话快照；长会话按需要由读方决定怎么降级。
const RECENT_COST: u32 = 4096;

/// owner —— 与目录名同形（`crates/symbio-plugin-notes`），装配表按它排序与报缺口。
pub(crate) const OWNER: &str = "crates/symbio-plugin-notes";

pub(crate) const EVENTS: &[EventDecl] = &[
    EventDecl::new(NOTES_CREATED, (Entity::Domain, Verb::Opened)),
    // 索引事实是「这次结果已处理」的断言，不是开新领域：`Domain × Asserted`。
    EventDecl::new(NOTES_INDEXED, (Entity::Domain, Verb::Asserted)),
    // 洞察同理：本能力对自己处理过的那条便签的断言。
    EventDecl::new(NOTES_INSIGHT, (Entity::Domain, Verb::Asserted)),
];

pub(crate) const FACTS: &[FactDecl] = &[
    FactDecl {
        name: NOTES_CREATED,
        port: None,
        key: &[],
        expand: &[],
        fields: &["text"],
    },
    // 键在行上（`notes.index` 按 `tool_call_id` 关账），事实声明不需要自己的键。
    FactDecl {
        name: NOTES_INDEXED,
        port: None,
        key: &[],
        expand: &[],
        fields: &["tool_call_id", "text"],
    },
    // 反思事实同样按行上的 `tool_call_id` 关账；`status` 记这次往返答成了没有
    // （失败也要关账，D-242）。
    FactDecl {
        name: NOTES_INSIGHT,
        port: None,
        key: &[],
        expand: &[],
        fields: &["tool_call_id", "text", "status"],
    },
];

/// 本插件事实的**归宿行**：`owes` 与 `until` 同源——有便签即闭合，没有便签则不欠。
///
/// 行表是义务，不是触发器；这条自读行只回答「这条事实归谁看护」。没有它，`seal`
/// 会把 `notes.created` 判成 `OrphanFact`（每条事实都要有归宿行）。
pub(crate) const ROWS: &[Decl] = &[
    Decl {
        name: NOTES_CREATED,
        prio: 200,
        by: None,
        tier: LatencyTier::Reflex,
        expand: None,
        owes: Pattern {
            source: NOTES_CREATED,
            key: &[],
            eq: &[],
        },
        until: &[Pattern {
            source: NOTES_CREATED,
            key: &[],
            eq: &[],
        }],
        unless: &[],
    },
    // 轮内索引：一条成功的 `notes_add` 产物落格之后，本能力再记一笔
    // `notes.indexed`（换的是本行的 `ActorDecl` = `plugin_row`，实现见 `row`）。
    //
    // `owes` 只收窄到**本能力自己的工具**且成功的结果——`eq` 是行的前件，不是实现
    // 里的 if：不成立时这笔债根本不开，环不会选中它。`prio` 45 让它在工具批收尾
    // （72）之后、收束（55）之前被选中：这一笔在**同一轮**里看得见产物。
    Decl {
        name: NOTES_INDEX,
        prio: 45,
        by: Some(symbio_plugin_api::rows::PLUGIN_ROW),
        tier: LatencyTier::Fast,
        expand: None,
        owes: Pattern {
            source: EVENT_ARTIFACT_ADDED,
            key: &["tool_call_id"],
            eq: &[("tool", Lit::Str(NOTES_TOOL)), ("success", Lit::Bool(true))],
        },
        until: &[Pattern {
            source: NOTES_INDEXED,
            key: &["tool_call_id"],
            eq: &[],
        }],
        unless: &[],
    },
    // 轮内反思：一条 `notes.indexed` 落格之后，本能力借宿主的一次模型往返写一条
    // `notes.insight`（`by = plugin_row`，实现交 `RowStep::Ask`，D-242）。
    //
    // `prio` 46 紧接索引行（45）：索引产物一入格，本行就是最小的可还债；`until`
    // 与 `owes` 同键，答成与失败都关账——失败也回一条 `status = failed` 的洞察，
    // 不把债留在原地再打一次模型。
    Decl {
        name: NOTES_REFLECT,
        prio: 46,
        by: Some(symbio_plugin_api::rows::PLUGIN_ROW),
        // 这一行真的要一次模型往返：最宽档（`plugin_row` 的执行体档）。
        tier: LatencyTier::Deep,
        expand: None,
        owes: Pattern {
            source: NOTES_INDEXED,
            key: &["tool_call_id"],
            eq: &[],
        },
        until: &[Pattern {
            source: NOTES_INSIGHT,
            key: &["tool_call_id"],
            eq: &[],
        }],
        unless: &[],
    },
];

pub(crate) const VIEWS: &[ViewDecl] = &[
    ViewDecl {
        name: NOTES_RECENT,
        ver: 1,
        reads: &[(NOTES_CREATED, "text")],
        // 读出口径不是收束判据：本视图不替任何事实关门。
        evidence: &[],
        cost: RECENT_COST,
        eval: recent_notes,
    },
    // 行执行体写下的那笔事实的读回：`notes.indexed` → 本能力在轮内收进索引的
    // 工具结果。它证明「通用执行体交出的草稿真的入了格」，读方不必认识行或执行体。
    ViewDecl {
        name: NOTES_DIGEST,
        ver: 1,
        reads: &[(NOTES_INDEXED, "tool_call_id")],
        evidence: &[],
        cost: RECENT_COST,
        eval: indexed_notes,
    },
    // 反思那一笔的读回：`notes.insight` → 本能力对已索引便签的洞察。读方同样不必
    // 认识行、执行体或模型往返——它读到的是「一次能力请求模型之后落了什么」。
    ViewDecl {
        name: NOTES_INSIGHTS,
        ver: 1,
        reads: &[(NOTES_INSIGHT, "tool_call_id")],
        evidence: &[],
        cost: RECENT_COST,
        eval: notes_insights,
    },
];

/// 纯折叠：`notes.created` → `{ count, items: [{ text, turn, ts }] }`。
///
/// 顺序按事件序（写入序）取**最后** `RECENT_LIMIT` 条，输出仍按时间正序——读方要
/// 「最近几条」时自己倒序即可，投影不替它选展示顺序。
fn recent_notes(events: &[Event], now: i64) -> Value {
    let mut texts: Vec<Value> = Vec::new();
    for e in events {
        if e.kind != NOTES_CREATED || e.ts > now {
            continue;
        }
        let Some(text) = e.payload.get("text").and_then(Value::as_str) else {
            continue;
        };
        texts.push(json!({ "text": text, "turn": e.turn, "ts": e.ts }));
    }
    if texts.len() > RECENT_LIMIT {
        texts.drain(..texts.len() - RECENT_LIMIT);
    }
    json!({ "count": texts.len(), "items": texts })
}

/// 纯折叠：`notes.indexed` → `{ count, items: [{ text, tool_call_id }] }`。
///
/// 与 [`recent_notes`] 同一口径（写入序、最新 `RECENT_LIMIT` 条），但读的是行执行体
/// 落的那笔事实——两个视图各自只读自己那条事实名。
fn indexed_notes(events: &[Event], _now: i64) -> Value {
    let mut items: Vec<Value> = Vec::new();
    for e in events {
        if e.kind != NOTES_INDEXED {
            continue;
        }
        items.push(json!({
            "tool_call_id": e.payload.get("tool_call_id").and_then(Value::as_str).unwrap_or(""),
            "text": e.payload.get("text").and_then(Value::as_str).unwrap_or(""),
        }));
    }
    if items.len() > RECENT_LIMIT {
        items.drain(..items.len() - RECENT_LIMIT);
    }
    json!({ "count": items.len(), "items": items })
}

/// 纯折叠：`notes.insight` → `{ count, items: [{ text, tool_call_id, status }] }`。
///
/// `status` 原样透出（`ok` / `failed`）：失败也是一次往返的产物，读方要能把它与
/// 「还没反思过」区分开——后者根本不在这一格里。
fn notes_insights(events: &[Event], _now: i64) -> Value {
    let mut items: Vec<Value> = Vec::new();
    for e in events {
        if e.kind != NOTES_INSIGHT {
            continue;
        }
        items.push(json!({
            "tool_call_id": e.payload.get("tool_call_id").and_then(Value::as_str).unwrap_or(""),
            "text": e.payload.get("text").and_then(Value::as_str).unwrap_or(""),
            "status": e.payload.get("status").and_then(Value::as_str).unwrap_or(""),
        }));
    }
    if items.len() > RECENT_LIMIT {
        items.drain(..items.len() - RECENT_LIMIT);
    }
    json!({ "count": items.len(), "items": items })
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
