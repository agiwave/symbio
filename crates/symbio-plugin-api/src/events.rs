//! 跨插件事件名常量 —— **由 `scripts/facts/events-rs.mjs` 生成，请勿手改**。
//!
//! 事实源：各插件自己目录里的 `PLUGIN.yml` 的 `shared_events:` 行（**owner** 导出给
//! 别的插件命名的那几条事件名）与 owner 自己的 `EventDecl` 表（格位）。加一条跨插件
//! 事件 = 在 owner 自己的清单里登记一行，并在 owner 的 `EventDecl` 里给它格位；内核与
//! core 的文件零 diff。手改本文件会被 `gen-facts --check` 判红。
//!
//! # 口径（展开见 `docs/003-design/024-能力横向扩充的通路.md`）
//!
//! - **本表只收跨插件消费的名字**。单消费方的名字由那个插件自己声明，不进这里。
//! - **格位取自 owner 的声明**：写闸问的是「这个名字落在哪一格」（DE-024 D-197），所以这里
//!   连 `EventDecl` 一起投影一份 `SHARED_EVENTS`——链接了本 crate 的每个进程都认识它，
//!   不必把 owner 插件也链进同一个二进制（那正是 E-009 禁掉的那条边）。
//! - **内核不认识业务事件名**（DE-024 D-203 / D-204）：内核的两张表只留封闭语法——Entity×Verb
//!   网格与协议五格。名字要跨插件就登记进清单，不要回内核加行（CS-005 / CS-007）。
//! - **角色不在这里**：`FactRole` 是 owner 交给守卫的陈述，仍住 `EventDecl` 由装配表读；
//!   本表只投影「名字 + 格位」两维。
//!
//! # 命名
//!
//! 常量名去掉 `EVENT_` 前缀后必须**逐字等于**值的大写形式（`.` → `_`）：
//! `memory.encoded` → `EVENT_MEMORY_ENCODED`。由 CS-007 检查。

use symbio_core::{Entity, EventDecl, Verb};

// ============ memory ============

/// memory.calibrated — owner `symbio-plugin-loop`，格位 `(Memory, Asserted)`
pub const EVENT_MEMORY_CALIBRATED: &str = "memory.calibrated";

/// memory.encoded — owner `symbio-plugin-loop`，格位 `(Memory, Opened)`
pub const EVENT_MEMORY_ENCODED: &str = "memory.encoded";

// ============ task ============

/// task.asserted — owner `symbio-plugin-loop`，格位 `(Task, Asserted)`
pub const EVENT_TASK_ASSERTED: &str = "task.asserted";

/// task.assigned — owner `symbio-plugin-loop`，格位 `(Task, Asserted)`
pub const EVENT_TASK_ASSIGNED: &str = "task.assigned";

/// task.controlled — owner `symbio-plugin-loop`，格位 `(Control, Opened)`
pub const EVENT_TASK_CONTROLLED: &str = "task.controlled";

/// task.held — owner `symbio-plugin-loop`，格位 `(Task, Held)`
pub const EVENT_TASK_HELD: &str = "task.held";

/// task.opened — owner `symbio-plugin-loop`，格位 `(Task, Opened)`
pub const EVENT_TASK_OPENED: &str = "task.opened";

/// task.progress — owner `symbio-plugin-loop`，格位 `(Task, Progressed)`
pub const EVENT_TASK_PROGRESS: &str = "task.progress";

/// 跨插件共享的事件行（名字 + 格位）——写闸的**共享臂**。
///
/// 每一行的格位来自 owner 自己的 `EventDecl`；角色不在这里（那是 owner 交给守卫的
/// 陈述，经装配表读）。链接了本 crate 的每个进程都认识这些名字，不必把 owner 插件
/// 链进同一个二进制——那正是 E-009 禁掉的边（DE-024 D-204）。
#[rustfmt::skip]
pub const SHARED_EVENTS: &[EventDecl] = &[
    EventDecl::new(EVENT_MEMORY_CALIBRATED, (Entity::Memory, Verb::Asserted)), // memory.calibrated ← loop
    EventDecl::new(EVENT_MEMORY_ENCODED, (Entity::Memory, Verb::Opened)), // memory.encoded ← loop
    EventDecl::new(EVENT_TASK_ASSERTED, (Entity::Task, Verb::Asserted)), // task.asserted ← loop
    EventDecl::new(EVENT_TASK_ASSIGNED, (Entity::Task, Verb::Asserted)), // task.assigned ← loop
    EventDecl::new(EVENT_TASK_CONTROLLED, (Entity::Control, Verb::Opened)), // task.controlled ← loop
    EventDecl::new(EVENT_TASK_HELD, (Entity::Task, Verb::Held)), // task.held ← loop
    EventDecl::new(EVENT_TASK_OPENED, (Entity::Task, Verb::Opened)), // task.opened ← loop
    EventDecl::new(EVENT_TASK_PROGRESS, (Entity::Task, Verb::Progressed)), // task.progress ← loop
];
