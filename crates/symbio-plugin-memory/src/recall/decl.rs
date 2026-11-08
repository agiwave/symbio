//! 本能力包交出的**声明视图**：`memory.live`（DE-026 D-268）。
//!
//! ## 它是给谁读的
//!
//! 巩固（名册 #2 的后半）的格位是债（D-268 订正）：申报与执行都在本包
//! （[`crate::debt`] / [`crate::port`]），执行体直接调本包的折叠。声明视图
//! `memory.live` 留给跨插件读方——D-387 Q3 的 `memory` 报表按名字读同一副折叠。
//!
//! ```text
//! 本包 ──ViewDecl{eval = live::memory_live} + PLUGIN.yml 的 shared_views──▶ 名字公开
//! 报表 ──ViewRead::named("memory.live")──▶ 冻结表里取 decl ──▶ read(events, now)
//! ```
//!
//! 这是 `D-221` 已有的机制，**不新增任何面**：视图是 owner 自己交的纯函数，
//! 公开是 owner 的显式动作，读方按名字读、不抄口径。
//!
//! ## 一副折叠，不是第二份实现
//!
//! `eval` 与包内检索调的是同一个函数（[`super::live::live`]）。视图是它的**第二种
//! 读法**——若记忆真是四个 crate，这副折叠就得跨 crate，那时只能靠声明视图；
//! 现在它是同一个 crate 里的一个函数 + 一张声明。
//!
//! ## 一处登记在案的缺口
//!
//! 排除边（`memory.forgotten` 的 `produced_by`）读的是**信封**字段，而 `ViewDecl.reads`
//! 只收 `(事实名, 载荷字段)` ⇒ 依赖图少一条「本视图依赖遗忘事实」的边。它不影响读数
//! 正确性（折叠拿到的是完整 `&[Event]`），但装配期的依赖图对这条边是乐观的。
//! 不开 `reads` 的口子——那条缺口住 DE-026 D-268，不住代码注释。

use symbio_plugin_api::events::{EVENT_MEMORY_CONSOLIDATED, EVENT_MEMORY_ENCODED};
use symbio_plugin_api::ViewDecl;

use super::live::memory_live;

/// 视图名——与 `PLUGIN.yml` 的 `shared_views` 逐字相同（`view_names.rs` 由它生成）。
pub(crate) const MEMORY_LIVE: &str = "memory.live";

/// 扫描成本上界。与全仓其余视图同值（`seal` 的 `ViewCostMismatch` 要求切点唯一）：
/// 巩固读的是快照尾部窗口，那个窗口不超过本值。
const COST: u32 = 4096;

pub(crate) const VIEWS: &[ViewDecl] = &[ViewDecl {
    name: MEMORY_LIVE,
    ver: 1,
    // 只声明折叠真正读的载荷字段：正文与标签（代数 / 可见域 / 最初一代由巩固按 seq
    // 点查全史，不经本视图）。排除边见模块文档的「一处登记在案的缺口」。
    reads: &[
        (EVENT_MEMORY_ENCODED, "content"),
        (EVENT_MEMORY_ENCODED, "tag"),
        (EVENT_MEMORY_CONSOLIDATED, "content"),
        (EVENT_MEMORY_CONSOLIDATED, "tag"),
    ],
    // 读出口径不是收束判据：本视图不替任何事实关门（它不出现任何行的 `until` 里）。
    evidence: &[],
    cost: COST,
    eval: memory_live,
    agg: None,
}];
