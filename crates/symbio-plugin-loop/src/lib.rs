//! 单环驱动器插件（`loop`）：纯机制环。
//!
//! 只做七职：选债 / 派发 / 等待 / 配对 / 记账 / 预算与租约 / 重放。
//! 对外只认两份数据——债表行与端口执行体注册表；端口名与载荷全是值，
//! 环不生成身份、不读序号、不认阶段。
//!
//! 旧编排（幕机 / 入口 / 阶段事件 / 行表阶段号）已整块删除；
//! 新链路只经这条机制环。

mod plugin;
pub mod ring;
mod vocab;

pub use plugin::LoopPlugin;
pub use ring::{
    AppendError, DebtKey, DebtRow, DebtSource, DriveOutcome, Fact, FactKind, Ledger, Limits,
    Registry, Ring, RunReport, StopReason, Table,
};
