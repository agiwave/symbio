//! 本插件拥有的事实名、格位、归宿行与对外可读的视图（DE-024 D-197 / DE-025 D-209 / D-221）。
//!
//! 一条事实 `world.observed`：**「某个实体的某个属性被观察到是这个值」**。名字、格位、
//! 载荷形状与归宿行同在这里，写方（[`super::behavior`]）与读方（[`WORLD_MODEL`] 视图）
//! 都从本模块取，不存在第二份拼写。
//!
//! ## 格位为什么是 `Domain × Asserted`
//!
//! 世界状态不是内核认识的核心语义，内核**不解释**这条事实：写闸只按本声明校验名字与
//! 格位（D-209）。用 `Asserted` 而不是 `Opened`，因为一次观察是一条**断言**——它可以
//! 被后来的观察压过；重放时「谁先看到什么」与「现在是什么」是两件可读的事。
//!
//! ## 视图为什么也在这里
//!
//! [`WORLD_MODEL`] 是纯折叠：从 `world.observed` 折出「每个实体当前各属性」。它按
//! 观测序取**最后一条**（同实体同属性后写覆盖），并把被覆盖的次数留在 `observations`
//! 里——读方既能看见当前值，也能看见「这个值改过几次」。
//!
//! 它在 `PLUGIN.yml` 的 `shared_views:` 登记为**对外可读**，别的插件经
//! `symbio_plugin_api::views` 按名字读——不依赖本 crate，也不在任何控制面里再写一份
//! （D-221）。名字、版本、成本与算法只有这里一个来源。

use serde_json::{json, Map, Value};

use symbio_plugin_api::ring::{Decl, Pattern};
use symbio_plugin_api::{
    Entity, Event, EventDecl, FactDecl, LatencyTier, PluginDeclarations, Verb, ViewDecl,
};

/// 「某个实体的某个属性是这个值」这一断言。落在 `domain × asserted`。
pub(crate) const WORLD_OBSERVED: &str = "world.observed";

/// 本插件的 LLM 可见工具名。工具面与行为声明的连接都取它，不存在第二份拼写。
pub(crate) const WORLD_TOOL: &str = "world_observe";

/// 对外可读的视图：当前世界状态的读数。
pub(crate) const WORLD_MODEL: &str = "world.model";

/// 视图最多保留的实体数。超出只影响**输出长度**，不影响扫描成本上界。
const LIMIT: usize = 50;

/// 扫描成本上界（事件条数）。它是环内窗口求值的预算；跨插件读口不查它——读侧按
/// 调用方给的切片全量折（`D-284`）。4096 覆盖通常的会话快照。
const COST: u32 = 4096;

/// owner —— 与目录名同形（`crates/symbio-plugin-world_model`），装配表按它排序与报缺口。
pub(crate) const OWNER: &str = "crates/symbio-plugin-world_model";

pub(crate) const EVENTS: &[EventDecl] = &[EventDecl::new(
    WORLD_OBSERVED,
    (Entity::Domain, Verb::Asserted),
)];

pub(crate) const FACTS: &[FactDecl] = &[FactDecl {
    name: WORLD_OBSERVED,
    port: None,
    key: &[],
    expand: &[],
    // `confidence` 是 0–100 的整数（写方与读方共用同一口径，见 `behavior::observation`）。
    fields: &["entity", "attribute", "value", "confidence"],
}];

/// 本插件事实的**归宿行**：`owes` 与 `until` 同源——有观察即闭合，没有则不欠。
///
/// 行表是义务，不是触发器；这条自读行只回答「这条事实归谁看护」。没有它，`seal`
/// 会把 `world.observed` 判成 `OrphanFact`（每条事实都要有归宿行）。
pub(crate) const ROWS: &[Decl] = &[Decl {
    name: WORLD_OBSERVED,
    by: None,
    tier: LatencyTier::Reflex,
    expand: None,
    owes: Pattern {
        source: WORLD_OBSERVED,
        key: &[],
        eq: &[],
    },
    until: &[Pattern {
        source: WORLD_OBSERVED,
        key: &[],
        eq: &[],
    }],
    unless: &[],
}];

pub(crate) const VIEWS: &[ViewDecl] = &[ViewDecl {
    name: WORLD_MODEL,
    ver: 1,
    reads: &[
        (WORLD_OBSERVED, "entity"),
        (WORLD_OBSERVED, "attribute"),
        (WORLD_OBSERVED, "value"),
        (WORLD_OBSERVED, "confidence"),
    ],
    // 读出口径不是收束判据：本视图不替任何事实关门。
    evidence: &[],
    cost: COST,
    eval: world_model,
    agg: None,
}];

/// 纯折叠：`world.observed` → `{ count, observations, entities: [{ entity, attributes }] }`。
///
/// 口径是三句话：
///
/// 1. **同实体同属性后写覆盖**——世界模型回答「现在是什么」，不保存时间线；
///    历史留在转写里，要时间线的读方按 `ts` 自己切。
/// 2. `observations` 是**被看到过多少次**（含被覆盖的），`count` 是**实体数**：前者
///    回答证据量，后者回答世界有多大，两者不互相推导。
/// 3. 实体顺序按**首次观察序**（写入序），超过 [`LIMIT`] 只留最早的若干实体——
///    世界模型是长期累积的读数，先进入世界的实体比后来的更可能是读方要找的那个。
fn world_model(events: &[Event], now: i64) -> Value {
    let mut entities: Vec<String> = Vec::new();
    // (entity, attribute, value, confidence)
    let mut latest: Vec<(String, String, String, u64)> = Vec::new();
    let mut observations: u64 = 0;

    for e in events {
        if e.kind != WORLD_OBSERVED || e.ts > now {
            continue;
        }
        let (Some(entity), Some(attribute), Some(value)) = (
            e.payload.get("entity").and_then(Value::as_str),
            e.payload.get("attribute").and_then(Value::as_str),
            e.payload.get("value").and_then(Value::as_str),
        ) else {
            continue;
        };
        // 缺字段的载荷静默跳过：视图不能因为一条坏事实就整个算不出来。
        let entity = entity.trim();
        let attribute = attribute.trim();
        let value = value.trim();
        if entity.is_empty() || attribute.is_empty() || value.is_empty() {
            continue;
        }
        observations += 1;
        if !entities.iter().any(|e| e == entity) {
            entities.push(entity.to_string());
        }
        let confidence = e
            .payload
            .get("confidence")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .min(100);
        if let Some(slot) = latest
            .iter_mut()
            .find(|(en, at, _, _)| en == entity && at == attribute)
        {
            *slot = (
                entity.to_string(),
                attribute.to_string(),
                value.to_string(),
                confidence,
            );
        } else {
            latest.push((
                entity.to_string(),
                attribute.to_string(),
                value.to_string(),
                confidence,
            ));
        }
    }

    if entities.len() > LIMIT {
        entities.truncate(LIMIT);
    }

    let rows: Vec<Value> = entities
        .iter()
        .map(|entity| {
            let mut attributes = Map::new();
            for (en, at, value, confidence) in &latest {
                if en == entity {
                    attributes.insert(
                        at.clone(),
                        json!({ "value": value, "confidence": confidence }),
                    );
                }
            }
            json!({ "entity": entity, "attributes": attributes })
        })
        .collect();

    json!({
        "count": rows.len(),
        "observations": observations,
        "entities": rows,
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
