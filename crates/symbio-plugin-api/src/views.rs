//! 视图读不出来的**缺席原因**。
//!
//! 声明视图的公开读口只有一条：owner 用 `submit_reports!` 把视图登记成报表列
//! （`ReportDecl.views`，`D-387`），读方遍历 [`crate::reports`] 取得整条读数。
//! 按名字的**共享清单**（`PLUGIN.yml` 的 `shared_views` → `view_names.rs`）已退役：
//! 它给同一条视图制造了第二个读口，而迁移完成后它已没有条目。
//!
//! 本模块只留「读不到」的两种原因，供报表读口与诊断列共用——缺席必须说得出为什么，
//! 不能让空值冒充读数（`D-281` 理由 3）。视图名本身只有 owner 的 [`crate::reports::ReportDecl`]
//! 一处。

use std::fmt;

/// 一条已声明视图读不出来的原因。
///
/// 只保留**与共享清单无关**的两种态：报表登记之后，「没有公开」不再是一种原因
/// ——公开就是登记本身。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ViewUnavailable {
    /// 声明表还没冻结（装配未完成）。
    NotSealed,
    /// 冻结表里没有这个名字：owner 没交这条 `ViewDecl`。
    Unknown { name: String },
}

impl ViewUnavailable {
    /// 线格式的原因词（诊断列的 `reason` 字段）。
    pub fn reason(&self) -> &'static str {
        match self {
            ViewUnavailable::NotSealed => "not_sealed",
            ViewUnavailable::Unknown { .. } => "unknown",
        }
    }
}

impl fmt::Display for ViewUnavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ViewUnavailable::NotSealed => write!(f, "声明表还没冻结，读不出任何视图"),
            ViewUnavailable::Unknown { name } => {
                write!(f, "冻结表里没有视图 `{name}`（owner 没交这条 ViewDecl）")
            }
        }
    }
}

impl std::error::Error for ViewUnavailable {}
