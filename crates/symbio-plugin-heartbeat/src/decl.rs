//! 名册 #48「节奏与时机」的最小能力面（`DE-026` 的逐行处置）。
//!
//! ## 为什么是事实，不是行为
//!
//! 「什么时候该开一轮」今天只有一条算术（[`super::scheduler`]）：空闲达到间隔即触发。
//! 名册 #48 要的是更上一层的东西——**在几种可发起的时机里挑一个**（刚完成一批工作、
//! 用户离开、有未回答的问题……）。那是一条判据，判据的产出应当先落成事实，再由写方接
//! 生产；把判据直接长进扫描循环会把「时机」和「间隔」混成同一个 if。
//!
//! 本批只交**声明面**：名字、格位、载荷字段、归宿行。写方还没有——所以行是 `by = None`，
//! 名字也不进 `shared_events`（没有第二个消费方，`CS-007`）。
//!
//! ## 格位为什么是 `Control × Asserted`
//!
//! 一条节拍判定说的是「**这个时刻**可以发起」，不是打开一条待定意图（那是
//! `conation.expressed` 的形状），也不是推进任何既有任务。它是控制面的一次断言，
//! 与 `task.controlled` 同格（`Control × Opened`）相邻但不同：后者开一次控制，本事实
//! 只记一次判定。
//!
//! ## 写方落地时改哪里
//!
//! 只改 [`ROWS`] 的 `by` 与生产侧；名字、格位与 `reason` 这一格不动。若那时出现第二个
//! 消费方，再把名字投影进 `PLUGIN.yml` 的 `shared_events`，而不是现在提前登记一条没人
//! 读的名字。

use symbio_plugin_api::ring::{Decl, Pattern};
use symbio_plugin_api::{Entity, EventDecl, FactDecl, LatencyTier, PluginDeclarations, Verb};

/// 一次节拍判定：**这个时刻可以发起一轮**。
pub(crate) const EVENT_HEARTBEAT_CADENCE: &str = "heartbeat.cadence";

/// owner —— 与目录名同形（`crates/symbio-plugin-heartbeat`），装配表按它排序与报缺口。
pub(crate) const OWNER: &str = "crates/symbio-plugin-heartbeat";

pub(crate) const EVENTS: &[EventDecl] = &[EventDecl::new(
    EVENT_HEARTBEAT_CADENCE,
    (Entity::Control, Verb::Asserted),
)];

pub(crate) const FACTS: &[FactDecl] = &[FactDecl {
    name: EVENT_HEARTBEAT_CADENCE,
    port: None,
    key: &[],
    expand: &[],
    // 只交「为什么是这个时刻」一格。间隔 / 抖动是机制参数，不是这条事实的载荷。
    fields: &["reason"],
}];

/// 归宿行：自读即闭合（`by = None` 是「这笔账不由引擎执行」的声明位）。
///
/// 没有它，`seal` 会把 `heartbeat.cadence` 判成 `OrphanFact`。
pub(crate) const ROWS: &[Decl] = &[Decl {
    name: EVENT_HEARTBEAT_CADENCE,
    by: None,
    tier: LatencyTier::Reflex,
    expand: None,
    owes: Pattern {
        source: EVENT_HEARTBEAT_CADENCE,
        key: &[],
        eq: &[],
    },
    until: &[Pattern {
        source: EVENT_HEARTBEAT_CADENCE,
        key: &[],
        eq: &[],
    }],
    unless: &[],
}];

// 装配口的提交：宿主链接本 crate 即生效。本插件不认识也不依赖任何兄弟插件。
symbio_plugin_api::submit_declarations!(OWNER, submitted_declarations);

fn submitted_declarations() -> PluginDeclarations<'static> {
    PluginDeclarations {
        events: EVENTS,
        facts: FACTS,
        views: &[],
        ports: &[],
        actors: Vec::new(),
        rows: ROWS,
    }
}

#[cfg(test)]
#[path = "decl.test.rs"]
mod tests;
