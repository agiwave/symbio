//! 视图与预算 —— 投影产出的共享类型层（[plan/01 §3](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
//!
//! 与 [`crate::event`] 同理：`projections`（③）与将来的 `actors`（②）
//! 都只 import 本模块的类型定义，互不持有句柄（[plan/05 §3.1](../../../../docs/003-design/018-模块架构.md "DE-018-模块架构")）。
//!
//! ## 冻结锚点
//!
//! **F2** 的后半句：投影返回 `View` 而**不是** `Result`——「可降级」是类型义务
//! 而不是约定：超预算的投影返回 `degraded: true`，对话永不因状态爆炸而中断。
//!
//! 召回视图与可见域登记表是**词汇**（谁的记忆、哪个 namespace），住 `symbio_core`
//! 的 `view` 模块；本模块只装 [`Budget`] / [`View`] 这两个封闭语法（DE-023 D-188）。

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

    /// 平凡值（[plan/03 §2.1](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")：慢但正确）。
    pub const fn generous() -> Self {
        Budget {
            tokens: u32::MAX,
            ms: u64::MAX,
        }
    }
}

/// 投影的产出：值 + 是否降级 + **扫描记账**。
///
/// `used.ms` 说的不是墙钟耗时，而是**交给闭包的窗口条数**（1 事件 = 1ms 的记账口径，
/// 与 [`super::projection`] 的预算语义同一套）：投影是纯函数，跑多久取决于宿主与机器，
/// 而「扫了多少」由窗口决定、可比可断言。把它读成耗时的人会拿它做延迟判据，
/// 而那正是它承载不了的东西。
///
/// **只能**是 `View`，不是 `Result`（构造即锁定，见 [`crate::projection::Projection`]）。
/// 只由 `Projection::apply` 构造
/// （[D-105](../../../../docs/003-design/001-核心架构.md#d-105-投影预算语义收敛到框架层闭包只返回-v窗口截断由-apply-统一执行 "DE-001-核心架构")：降级与预算在框架层统一执行，
/// `ok` / `degraded` 构造器随闭包签名一并退役）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View<V> {
    pub value: V,
    pub degraded: bool,
    pub used: Budget,
}
