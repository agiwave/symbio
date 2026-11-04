//! 纯机制环的模块树。
//!
//! 五类事实、账本、债表与引擎；所有端口名都从行数据读入，代码里没有字面量。

pub mod facts;
pub mod ledger;
pub mod table;

mod engine;

pub use engine::{DriveOutcome, Limits, Registry, Ring, RunReport, StopReason};
pub use facts::{DebtKey, Fact, FactKind};
pub use ledger::{AppendError, Ledger};
pub use table::{DebtRow, DebtSource, Table};
