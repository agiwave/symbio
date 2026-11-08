//! 本插件拥有的事实名、格位、归宿行与对外可读的视图（DE-024 D-197 / DE-025 D-209 / D-221）。
//!
//! 一条事实 `reversibility.assessed`：**「某个动作被判断为这个可逆等级」**。名字、格位、
//! 载荷形状与归宿行同在这里，写方（[`super::behavior`]）与读方（[`REVERSIBILITY_REGISTER`]）
//! 都从本模块取，不存在第二份拼写。
//!
//! ## 三个等级的口径
//!
//! | 等级 | 含义 | 读方该做什么 |
//! |---|---|---|
//! | `reversible` | 执行后能按原路退回 | 无需额外准备 |
//! | `compensable` | 退不回，但有**说得出做法**的补偿 | 执行前先确认补偿可用 |
//! | `irreversible` | 退不回、也没有补偿 | 需要人确认或干脆不做 |
//!
//! `compensable` 的 `undo` 在工具与行为两层都要求非空：一个「有补偿」却说不清怎么补偿的
//! 标注，比 `irreversible` 更危险——它会让执行方误以为后路存在。
//!
//! ## 格位为什么是 `Domain × Asserted`
//!
//! 内核**不解释**这条事实：写闸只按本声明校验名字与格位（D-209）。用 `Asserted` 而不是
//! `Opened`，因为一次标注是一条**判断**——它可以被后来的判断修正（发现那个目录其实
//! 有客户数据，动作从 `reversible` 升为 `irreversible`），而重放时两次判断都留在转写里。
//!
//! ## 视图为什么也在这里
//!
//! [`REVERSIBILITY_REGISTER`] 从 `reversibility.assessed` 折出「每个动作当前等级」与
//! 等级分布。它按标注序取**最后一条**（同动作后写覆盖），`observations` 保留被覆盖的
//! 证据量。视图在 `PLUGIN.yml` 的 `shared_views:` 登记为对外可读，读方按名字读，
//! 不依赖本 crate（D-221）。

use serde_json::{json, Map, Value};

use symbio_plugin_api::ring::{Decl, Pattern};
use symbio_plugin_api::{
    Entity, Event, EventDecl, FactDecl, LatencyTier, PluginDeclarations, Verb, ViewDecl,
};

/// 「某个动作的可逆等级」这一判断。落在 `domain × asserted`。
pub(crate) const REVERSIBILITY_ASSESSED: &str = "reversibility.assessed";

/// 本插件的 LLM 可见工具名。工具面与行为声明的连接都取它，不存在第二份拼写。
pub(crate) const REVERSIBILITY_TOOL: &str = "reversibility_annotate";

/// 对外可读的视图：当前动作登记表。
pub(crate) const REVERSIBILITY_REGISTER: &str = "reversibility.register";

/// 三个可逆等级。工具、行为与视图共用这一张名单（读方按同一口径判「是否认识」）。
pub(crate) const LEVELS: [&str; 3] = ["reversible", "compensable", "irreversible"];

/// 视图最多保留的动作数。超出只影响**输出长度**，不影响扫描成本上界。
const LIMIT: usize = 50;

/// 扫描成本上界（事件条数）；跨插件读口不查它（D-284）。
const COST: u32 = 4096;

/// owner —— 与目录名同形（`crates/symbio-plugin-reversibility`）。
pub(crate) const OWNER: &str = "crates/symbio-plugin-reversibility";

pub(crate) const EVENTS: &[EventDecl] = &[EventDecl::new(
    REVERSIBILITY_ASSESSED,
    (Entity::Domain, Verb::Asserted),
)];

pub(crate) const FACTS: &[FactDecl] = &[FactDecl {
    name: REVERSIBILITY_ASSESSED,
    port: None,
    key: &[],
    expand: &[],
    // `undo` 恒为非空串（无补偿时为空串），由写方统一成这一形状，读方不必判可空。
    fields: &["action", "level", "undo"],
}];

/// 本插件事实的**归宿行**：有标注即闭合，没有则不欠。
pub(crate) const ROWS: &[Decl] = &[Decl {
    name: REVERSIBILITY_ASSESSED,
    by: None,
    tier: LatencyTier::Reflex,
    expand: None,
    owes: Pattern {
        source: REVERSIBILITY_ASSESSED,
        key: &[],
        eq: &[],
    },
    until: &[Pattern {
        source: REVERSIBILITY_ASSESSED,
        key: &[],
        eq: &[],
    }],
    unless: &[],
}];

pub(crate) const VIEWS: &[ViewDecl] = &[ViewDecl {
    name: REVERSIBILITY_REGISTER,
    ver: 1,
    reads: &[
        (REVERSIBILITY_ASSESSED, "action"),
        (REVERSIBILITY_ASSESSED, "level"),
        (REVERSIBILITY_ASSESSED, "undo"),
    ],
    // 读出口径不是收束判据：本视图不替任何事实关门。
    evidence: &[],
    cost: COST,
    eval: reversibility_register,
    agg: None,
}];

/// 纯折叠：`reversibility.assessed` → `{ count, observations, by_level, actions }`。
///
/// 口径是三句话：
///
/// 1. **同动作后写覆盖**——登记表回答「这个动作现在被判成什么」，等级可以升降；
///    两次判断都留在转写里，视图不替读方保存判断史。
/// 2. `by_level` 只统计**当前**等级：它回答「现在有多少不可逆动作」，不是「总共判过几次」；
///    `observations` 才是证据量。
/// 3. 动作顺序按**首次标注序**，超过 [`LIMIT`] 只留最早若干——登记表是长期累积的读面。
fn reversibility_register(events: &[Event], now: i64) -> Value {
    let mut actions: Vec<String> = Vec::new();
    // (action, level, undo)
    let mut latest: Vec<(String, String, String)> = Vec::new();
    let mut observations: u64 = 0;

    for e in events {
        if e.kind != REVERSIBILITY_ASSESSED || e.ts > now {
            continue;
        }
        let (Some(action), Some(level)) = (
            e.payload.get("action").and_then(Value::as_str),
            e.payload.get("level").and_then(Value::as_str),
        ) else {
            continue;
        };
        let action = action.trim();
        let level = level.trim();
        if action.is_empty() || !LEVELS.contains(&level) {
            continue;
        }
        // `compensable` 没有补偿做法 = 一条误判，视图不把它当成已登记的可补偿动作。
        let undo = e
            .payload
            .get("undo")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if level == "compensable" && undo.is_empty() {
            continue;
        }
        observations += 1;
        if !actions.iter().any(|a| a == action) {
            actions.push(action.to_string());
        }
        let row = (action.to_string(), level.to_string(), undo.to_string());
        if let Some(slot) = latest.iter_mut().find(|(a, _, _)| a == action) {
            *slot = row;
        } else {
            latest.push(row);
        }
    }

    if actions.len() > LIMIT {
        actions.truncate(LIMIT);
    }

    let mut by_level = Map::new();
    for level in LEVELS {
        by_level.insert(level.to_string(), json!(0));
    }
    for (action, level, _) in &latest {
        if !actions.iter().any(|a| a == action) {
            continue;
        }
        if let Some(cell) = by_level.get_mut(level) {
            if let Some(n) = cell.as_u64() {
                *cell = json!(n + 1);
            }
        }
    }

    let rows: Vec<Value> = actions
        .iter()
        .map(|action| {
            let (level, undo) = latest
                .iter()
                .find(|(a, _, _)| a == action)
                .map(|(_, l, u)| (l.as_str(), u.as_str()))
                .unwrap_or(("irreversible", ""));
            json!({ "action": action, "level": level, "undo": undo })
        })
        .collect();

    json!({
        "count": rows.len(),
        "observations": observations,
        "by_level": by_level,
        "actions": rows,
    })
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
