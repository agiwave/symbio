//! 本插件拥有的事实名、格位、归宿行与包内视图（DE-024 D-197 / DE-025 D-209 / D-221）。
//!
//! 两条事实：
//!
//! - `protocol.proposed`：**「某个协议被提出，内容是这份标题/目的/消息序列」**；
//! - `protocol.joined`：**「某个参与者加入了某个协议」**。
//!
//! 名字、格位、载荷形状与归宿行同在这里，写方（[`super::behavior`]）、读方
//! （[`PROTOCOL_CATALOG`]）与路由（[`super::plugin`]）都从本模块取，不存在第二份拼写。
//!
//! ## 格位为什么是 `Domain × Asserted`
//!
//! 内核**不解释**协议：写闸只按本声明校验名字与格位（D-209）。用 `Asserted` 而不是
//! `Opened`：提案可以被同名再提案覆盖（目的或消息序列调整），加入是一条独立断言。
//!
//! ## 「第二方」在视图里的判据
//!
//! 提案只来自一方；`participants` 按**不同参与者**去重。1 个仍是 `draft`，2 个及以上
//! 才是 `established`——这就是「协议需要第二方」的可执行形式，不需要额外的状态机。
//!
//! ## 路由与视图共用同一个折叠函数
//!
//! `protocol/catalog` 路由与 `protocol.catalog` 视图调用的是同一个 [`catalog`]：
//! 路由给第二方看的东西必须与视图给插件看的东西逐字一致，否则「第二方看到的成立状态」
//! 会与宿主里的判断分叉。

use serde_json::{json, Map, Value};

use symbio_plugin_api::ring::{Decl, Pattern};
use symbio_plugin_api::{
    Entity, Event, EventDecl, FactDecl, LatencyTier, PluginDeclarations, Verb, ViewDecl,
};

/// 「某个协议被提出」。落在 `domain × asserted`。
pub(crate) const PROTOCOL_PROPOSED: &str = "protocol.proposed";

/// 「某个参与者加入了某个协议」。落在 `domain × asserted`。
pub(crate) const PROTOCOL_JOINED: &str = "protocol.joined";

/// 提案工具名。
pub(crate) const PROTOCOL_PROPOSE_TOOL: &str = "protocol_propose";

/// 加入工具名。
pub(crate) const PROTOCOL_JOIN_TOOL: &str = "protocol_join";

/// 包内视图：协议清单与成立状态；外部第二方走 `protocol/catalog` 路由（名字不公开）。
pub(crate) const PROTOCOL_CATALOG: &str = "protocol.catalog";

/// 成立所需的**不同参与者**数。1 个不算协议，2 个才是双方。
pub(crate) const ESTABLISHED_AT: usize = 2;

/// 视图最多保留的协议数。超出只影响**输出长度**，不影响扫描成本上界。
const LIMIT: usize = 50;

/// 扫描成本上界（事件条数）；跨插件读口不查它（D-284）。
const COST: u32 = 4096;

/// owner —— 与目录名同形（`crates/symbio-plugin-protocol`）。
pub(crate) const OWNER: &str = "crates/symbio-plugin-protocol";

pub(crate) const EVENTS: &[EventDecl] = &[
    EventDecl::new(PROTOCOL_PROPOSED, (Entity::Domain, Verb::Asserted)),
    EventDecl::new(PROTOCOL_JOINED, (Entity::Domain, Verb::Asserted)),
];

pub(crate) const FACTS: &[FactDecl] = &[
    FactDecl {
        name: PROTOCOL_PROPOSED,
        port: None,
        key: &[],
        expand: &[],
        fields: &["name", "title", "purpose", "messages"],
    },
    FactDecl {
        name: PROTOCOL_JOINED,
        port: None,
        key: &[],
        expand: &[],
        fields: &["name", "participant"],
    },
];

/// 两条事实各自的**归宿行**：有提案 / 有加入即闭合，没有则不欠。
pub(crate) const ROWS: &[Decl] = &[
    Decl {
        name: PROTOCOL_PROPOSED,
        by: None,
        tier: LatencyTier::Reflex,
        expand: None,
        owes: Pattern {
            source: PROTOCOL_PROPOSED,
            key: &[],
            eq: &[],
        },
        until: &[Pattern {
            source: PROTOCOL_PROPOSED,
            key: &[],
            eq: &[],
        }],
        unless: &[],
    },
    Decl {
        name: PROTOCOL_JOINED,
        by: None,
        tier: LatencyTier::Reflex,
        expand: None,
        owes: Pattern {
            source: PROTOCOL_JOINED,
            key: &[],
            eq: &[],
        },
        until: &[Pattern {
            source: PROTOCOL_JOINED,
            key: &[],
            eq: &[],
        }],
        unless: &[],
    },
];

pub(crate) const VIEWS: &[ViewDecl] = &[ViewDecl {
    name: PROTOCOL_CATALOG,
    ver: 1,
    reads: &[
        (PROTOCOL_PROPOSED, "name"),
        (PROTOCOL_PROPOSED, "title"),
        (PROTOCOL_PROPOSED, "purpose"),
        (PROTOCOL_PROPOSED, "messages"),
        (PROTOCOL_JOINED, "name"),
        (PROTOCOL_JOINED, "participant"),
    ],
    // 读出口径不是收束判据：本视图不替任何事实关门。
    evidence: &[],
    cost: COST,
    eval: catalog,
    agg: None,
}];

/// 纯折叠：两条事实 → `{ count, observations, by_status, protocols }`。
///
/// 口径是四句话：
///
/// 1. **同名再提案后写覆盖**——清单回答「这个协议现在是什么」；
/// 2. `participants` 按不同参与者去重，顺序按首次加入序；
/// 3. 不同参与者 ≥ [`ESTABLISHED_AT`] ⇒ `established`，否则 `draft`；
///    `by_status` 只统计当前状态；
/// 4. 加入一个没有提案的名字不产生条目（协议必须先被提出）；`observations` 只记
///    有效提案数与有效加入数。
pub(crate) fn catalog(events: &[Event], now: i64) -> Value {
    let mut names: Vec<String> = Vec::new();
    // (name, title, purpose, messages)
    let mut proposals: Vec<(String, String, String, Vec<String>)> = Vec::new();
    let mut observations: u64 = 0;

    for e in events {
        if e.kind != PROTOCOL_PROPOSED || e.ts > now {
            continue;
        }
        let (Some(name), Some(title), Some(purpose)) = (
            e.payload.get("name").and_then(Value::as_str),
            e.payload.get("title").and_then(Value::as_str),
            e.payload.get("purpose").and_then(Value::as_str),
        ) else {
            continue;
        };
        let name = name.trim();
        let title = title.trim();
        let purpose = purpose.trim();
        let Some(messages) = messages_of(&e.payload) else {
            continue;
        };
        if name.is_empty() || title.is_empty() || purpose.is_empty() {
            continue;
        }
        observations += 1;
        if !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
        let row = (
            name.to_string(),
            title.to_string(),
            purpose.to_string(),
            messages,
        );
        if let Some(slot) = proposals.iter_mut().find(|(n, _, _, _)| n == name) {
            *slot = row;
        } else {
            proposals.push(row);
        }
    }

    // 参与者按协议名分桶：加入没有提案的名字不产生条目。
    let mut participants: Vec<(String, Vec<String>)> = Vec::new();
    for e in events {
        if e.kind != PROTOCOL_JOINED || e.ts > now {
            continue;
        }
        let (Some(name), Some(participant)) = (
            e.payload.get("name").and_then(Value::as_str),
            e.payload.get("participant").and_then(Value::as_str),
        ) else {
            continue;
        };
        let name = name.trim();
        let participant = participant.trim();
        if name.is_empty() || participant.is_empty() || !names.iter().any(|n| n == name) {
            continue;
        }
        observations += 1;
        if let Some((_, list)) = participants.iter_mut().find(|(n, _)| n == name) {
            if !list.iter().any(|p| p == participant) {
                list.push(participant.to_string());
            }
        } else {
            participants.push((name.to_string(), vec![participant.to_string()]));
        }
    }

    if names.len() > LIMIT {
        names.truncate(LIMIT);
    }

    let mut by_status = Map::new();
    by_status.insert("draft".to_string(), json!(0));
    by_status.insert("established".to_string(), json!(0));

    let protocols: Vec<Value> = names
        .iter()
        .map(|name| {
            let (title, purpose, messages) = proposals
                .iter()
                .find(|(n, _, _, _)| n == name)
                .map(|(_, t, p, m)| (t.clone(), p.clone(), m.clone()))
                .unwrap_or_default();
            let people = participants
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, p)| p.clone())
                .unwrap_or_default();
            let status = if people.len() >= ESTABLISHED_AT {
                "established"
            } else {
                "draft"
            };
            if let Some(cell) = by_status.get_mut(status) {
                if let Some(n) = cell.as_u64() {
                    *cell = json!(n + 1);
                }
            }
            json!({
                "name": name,
                "title": title,
                "purpose": purpose,
                "messages": messages,
                "participants": people,
                "status": status,
            })
        })
        .collect();

    json!({
        "count": protocols.len(),
        "observations": observations,
        "by_status": by_status,
        "protocols": protocols,
    })
}

/// 消息序列：非空字符串数组，去空白、按首次出现去重。
fn messages_of(payload: &Value) -> Option<Vec<String>> {
    let raw = payload.get("messages")?.as_array()?;
    let mut out: Vec<String> = Vec::new();
    for item in raw {
        let m = item.as_str()?.trim();
        if m.is_empty() {
            return None;
        }
        if !out.iter().any(|x| x == m) {
            out.push(m.to_string());
        }
    }
    if out.is_empty() {
        return None;
    }
    Some(out)
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
