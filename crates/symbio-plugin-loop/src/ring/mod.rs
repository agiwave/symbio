//! 纯机制环的模块树。
//!
//! 五类事实、事实投影、债表与驱动入口；所有端口名都从行数据读入，代码里没有字面量。

pub mod facts;
pub mod table;

mod engine;

pub use engine::{drive, DriveCtx, DriveOutcome, RunReport};
pub use facts::{DebtKey, Fact, FactKind, FactView};
pub use table::{DebtRow, DebtSource, Table};
