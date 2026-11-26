//! 纯机制环的模块树。
//!
//! 五类事实、事实投影、债表与驱动入口；所有端口名都从行数据读入，代码里没有字面量。
//!
//! 全部类型**只在 crate 内可见**：外泄为零，宿主经 `plugin_api::drive` 的通用契约
//! 与环交互（`DE-051` 机制形状）。

pub(crate) mod facts;
pub(crate) mod table;

pub(crate) mod engine;

pub(crate) use self::engine::{drive, DriveCtx, DriveOutcome};
pub(crate) use self::facts::{DebtKey, Fact, FactKind, FactView};
pub(crate) use self::table::{DebtRow, DebtSource, DeclInput};
