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

use symbio_core::ring::{Decl, Pattern};
use symbio_core::{
    Entity, Event, EventDecl, FactDecl, LatencyTier, PluginDeclarations, Verb, ViewDecl,
};

/// 「记下一条便签」这一事实。落在 `domain × opened`。
pub(crate) const NOTES_CREATED: &str = "notes.created";

/// 对外可读的视图：最近几条便签（新→旧）。
pub(crate) const NOTES_RECENT: &str = "notes.recent";

/// 视图最多保留的条数。超出只影响**输出长度**，不影响扫描成本上界。
const RECENT_LIMIT: usize = 20;

/// 扫描成本上界（事件条数）。与义务环的视图同一口径：超界不静默截断，读方拿到
/// `OverBudget`。4096 覆盖通常的会话快照；长会话按需要由读方决定怎么降级。
const RECENT_COST: u32 = 4096;

/// owner —— 与目录名同形（`crates/symbio-plugin-notes`），装配表按它排序与报缺口。
pub(crate) const OWNER: &str = "crates/symbio-plugin-notes";

pub(crate) const EVENTS: &[EventDecl] = &[EventDecl::new(
    NOTES_CREATED,
    (Entity::Domain, Verb::Opened),
)];

pub(crate) const FACTS: &[FactDecl] = &[FactDecl {
    name: NOTES_CREATED,
    port: None,
    key: &[],
    expand: &[],
    fields: &["text"],
}];

/// 本插件事实的**归宿行**：`owes` 与 `until` 同源——有便签即闭合，没有便签则不欠。
///
/// 行表是义务，不是触发器；这条自读行只回答「这条事实归谁看护」。没有它，`seal`
/// 会把 `notes.created` 判成 `OrphanFact`（每条事实都要有归宿行）。
pub(crate) const ROWS: &[Decl] = &[Decl {
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
}];

pub(crate) const VIEWS: &[ViewDecl] = &[ViewDecl {
    name: NOTES_RECENT,
    ver: 1,
    reads: &[(NOTES_CREATED, "text")],
    // 读出口径不是收束判据：本视图不替任何事实关门。
    evidence: &[],
    cost: RECENT_COST,
    eval: recent_notes,
}];

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
