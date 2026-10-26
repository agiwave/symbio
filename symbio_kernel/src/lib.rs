//! 内核 crate：**封闭语法**的最小实现（DE-023 / D-188 C2a）
//!
//! 这里只装语法：事件信封与 Entity×Verb、视图与预算、事件源、投影构造器、声明容器与
//! 环求解器、时延档位。词汇（事实名 / 视图名 / 插件 id / 路由）与行为（投影函数 /
//! actor 策略 / 执行体）仍住 `symbio_core` 与各 owner 插件——加一项词汇或行为不应
//! 落进本 crate（DE-023 D-177）。
//!
//! `symbio_core` 依赖本 crate，并在 `symbio_core/mod.rs` 里用同名私有模块把这里的
//! 模块重新导出，所以仓内既有路径 `crate::symbio_core::event::…` 不变；根账本仍由
//! `symbio_core` 负责（C-003 / C-004）。
//!
//! ## 内核文件集
//!
//! 下面的 `core-kernel-files:` 标记是 CS-001（形状）与 CS-004（插件 id 字面量）
//! 的**判定范围**，由 `scripts/core-shape-audit.mjs` 读取。C2a 只装已闭合子集；
//! C2b 把五个封闭语法 trait 及其签名类型也搬进来后，标记才会整份迁到本文件。
// core-kernel-files: event/mod.rs event/grid.rs view/mod.rs store/mod.rs projection/mod.rs decl/mod.rs decl/ring.rs adapters.rs

pub mod adapters;
pub mod decl;
pub mod event;
pub mod projection;
pub mod store;
pub mod view;

pub use adapters::LatencyTier;
pub use event::{Entity, Event, EventEnvelope, Seq, Timestamp, Verb};
pub use projection::Projection;
