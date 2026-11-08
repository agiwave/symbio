//! 本插件拥有的事实名、格位、归宿行与对外可读的视图（DE-024 D-197 / DE-025 D-209 / D-221）。
//!
//! 一条事实 `toolsmith.defined`：**「某个工具名被定义成这份描述与 schema」**。名字、格位、
//! 载荷形状与归宿行同在这里，写方（[`super::behavior`]）与读方（[`TOOLSMITH_RECIPES`]）
//! 都从本模块取，不存在第二份拼写。
//!
//! ## 载荷里的 `owner` 为什么是常量
//!
//! 「只能造自己插件的工具」在形状上落实：`owner` 不是入参，是写方写死的 `"toolsmith"`。
//! 视图再按它过滤一遍——即便有别的写方塞进一条 owner 不同的 `toolsmith.defined`，
//! 也读不出来。两个方向都堵上，限制才不依赖调用方守规矩。
//!
//! ## 格位为什么是 `Domain × Asserted`
//!
//! 内核**不解释**这条事实：写闸只按本声明校验名字与格位（D-209）。用 `Asserted` 而不是
//! `Opened`，因为一次登记是一条**定义**——同名再登记按后写覆盖（描述或 schema 可以
//! 不同），视图按登记序取最后一条。
//!
//! ## 视图回答什么
//!
//! [`TOOLSMITH_RECIPES`] 折出「每个工具名当前的定义」：`count` 是工具数，`observations`
//! 是登记次数（含被修订覆盖的）。它**不声称**这些定义已经进入可调用工具面——运行期
//! 装配不存在（D-217），那是 #42 的边界。

use serde_json::{json, Value};

use symbio_plugin_api::ring::{Decl, Pattern};
use symbio_plugin_api::{
    Entity, Event, EventDecl, FactDecl, LatencyTier, PluginDeclarations, Verb, ViewDecl,
};

/// 「本插件自己的一个工具定义」。落在 `domain × asserted`。
pub(crate) const TOOLSMITH_DEFINED: &str = "toolsmith.defined";

/// 本插件的 LLM 可见工具名。
pub(crate) const TOOLSMITH_TOOL: &str = "toolsmith_define";

/// 对外可读的视图：本插件的工具定义清单。
pub(crate) const TOOLSMITH_RECIPES: &str = "toolsmith.recipes";

/// 定义的 owner 恒为它。写方写死、视图过滤，两处共用这一个常量。
pub(crate) const OWNER_PLUGIN: &str = "toolsmith";

/// 视图最多保留的工具数。超出只影响**输出长度**，不影响扫描成本上界。
const LIMIT: usize = 50;

/// 扫描成本上界（事件条数）；跨插件读口不查它（D-284）。
const COST: u32 = 4096;

/// owner —— 与目录名同形（`crates/symbio-plugin-toolsmith`）。
pub(crate) const OWNER: &str = "crates/symbio-plugin-toolsmith";

pub(crate) const EVENTS: &[EventDecl] = &[EventDecl::new(
    TOOLSMITH_DEFINED,
    (Entity::Domain, Verb::Asserted),
)];

pub(crate) const FACTS: &[FactDecl] = &[FactDecl {
    name: TOOLSMITH_DEFINED,
    port: None,
    key: &[],
    expand: &[],
    fields: &["name", "description", "input_schema", "owner"],
}];

/// 本插件事实的**归宿行**：有登记即闭合，没有则不欠。
pub(crate) const ROWS: &[Decl] = &[Decl {
    name: TOOLSMITH_DEFINED,
    by: None,
    tier: LatencyTier::Reflex,
    expand: None,
    owes: Pattern {
        source: TOOLSMITH_DEFINED,
        key: &[],
        eq: &[],
    },
    until: &[Pattern {
        source: TOOLSMITH_DEFINED,
        key: &[],
        eq: &[],
    }],
    unless: &[],
}];

pub(crate) const VIEWS: &[ViewDecl] = &[ViewDecl {
    name: TOOLSMITH_RECIPES,
    ver: 1,
    reads: &[
        (TOOLSMITH_DEFINED, "name"),
        (TOOLSMITH_DEFINED, "description"),
        (TOOLSMITH_DEFINED, "input_schema"),
        (TOOLSMITH_DEFINED, "owner"),
    ],
    // 读出口径不是收束判据：本视图不替任何事实关门。
    evidence: &[],
    cost: COST,
    eval: recipes,
    agg: None,
}];

/// 纯折叠：`toolsmith.defined` → `{ count, observations, tools }`。
///
/// 口径是三句话：
///
/// 1. **同名后写覆盖**——清单回答「这个工具现在是什么」，定义可以修订。
/// 2. `observations` 是登记次数（含被覆盖的），`count` 是工具数：前者是证据量，后者是清单规模。
/// 3. **只收 owner = `toolsmith` 的定义**：限制不靠写方守规矩，视图自己再筛一遍。
fn recipes(events: &[Event], now: i64) -> Value {
    let mut names: Vec<String> = Vec::new();
    // (name, description, input_schema)
    let mut latest: Vec<(String, String, Value)> = Vec::new();
    let mut observations: u64 = 0;

    for e in events {
        if e.kind != TOOLSMITH_DEFINED || e.ts > now {
            continue;
        }
        let (Some(name), Some(description), Some(schema), Some(owner)) = (
            e.payload.get("name").and_then(Value::as_str),
            e.payload.get("description").and_then(Value::as_str),
            e.payload.get("input_schema"),
            e.payload.get("owner").and_then(Value::as_str),
        ) else {
            continue;
        };
        let name = name.trim();
        let description = description.trim();
        if owner != OWNER_PLUGIN || name.is_empty() || description.is_empty() || !schema.is_object()
        {
            continue;
        }
        observations += 1;
        if !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
        let row = (name.to_string(), description.to_string(), schema.clone());
        if let Some(slot) = latest.iter_mut().find(|(n, _, _)| n == name) {
            *slot = row;
        } else {
            latest.push(row);
        }
    }

    if names.len() > LIMIT {
        names.truncate(LIMIT);
    }

    let tools: Vec<Value> = names
        .iter()
        .map(|name| {
            let (description, schema) = latest
                .iter()
                .find(|(n, _, _)| n == name)
                .map(|(_, d, s)| (d.as_str(), s.clone()))
                .unwrap_or(("", json!({})));
            json!({
                "name": name,
                "description": description,
                "input_schema": schema,
                "owner": OWNER_PLUGIN,
            })
        })
        .collect();

    json!({
        "count": tools.len(),
        "observations": observations,
        "tools": tools,
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
