//! 视图与预算 —— 投影产出的共享类型层（[plan/01 §3](../../../../docs/plan/01-核心架构.md)）。
//!
//! 与 [`crate::symbio_core::event`] 同理：`projections`（③）与将来的 `actors`（②）
//! 都只 import 本模块的类型定义，互不持有句柄（[plan/05 §3.1](../../../../docs/plan/05-模块架构.md)）。
//!
//! ## 冻结锚点
//!
//! **F2** 的后半句：投影返回 `View` 而**不是** `Result`——「可降级」是类型义务
//! 而不是约定：超预算的投影返回 `degraded: true`，对话永不因状态爆炸而中断。

/// 投影预算（I3 记账口径）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Budget {
    pub tokens: u32,
    pub ms: u64,
}

impl Budget {
    pub const fn new(tokens: u32, ms: u64) -> Self {
        Budget { tokens, ms }
    }

    /// 平凡值（[plan/03 §2.1](../../../../docs/plan/03-演进与验证.md)：慢但正确）。
    pub const fn generous() -> Self {
        Budget {
            tokens: u32::MAX,
            ms: u64::MAX,
        }
    }
}

/// 投影的产出：值 + 是否降级 + 实际消耗。
///
/// **只能**是 `View`，不是 `Result`（构造即锁定，见 [`crate::symbio_core::projection::Projection`]）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View<V> {
    pub value: V,
    pub degraded: bool,
    pub used: Budget,
}

impl<V> View<V> {
    pub fn ok(value: V, used: Budget) -> Self {
        View {
            value,
            degraded: false,
            used,
        }
    }

    pub fn degraded(value: V, used: Budget) -> Self {
        View {
            value,
            degraded: true,
            used,
        }
    }
}
