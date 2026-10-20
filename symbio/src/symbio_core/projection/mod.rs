//! `Projection` —— 纯函数视图（v2 ③，[plan/01 §3](../../../../docs/004-plan/001-核心架构.md "PL-001-核心架构")）。
//!
//! ## 冻结锚点 F2（构造形状，不是签名约定）
//!
//! 投影 = 一族纯函数。**字段私有 + 唯一构造入口 `new`**，而 `new` 的泛型约束
//! `F: Fn(&[Event], Timestamp) -> V` 就是「纯净性」在类型层的全部表达：
//!
//! - 形参里**没有** `&Store` / `&mut` / `Clock` / `Principal` —— 想拿这些句柄，
//!   构造那一刻就编译不过（一级强制，J3）；
//! - 闭包**只返回 `V`**——「可降级」与「预算」由 [`Projection::apply`] 统一执行
//!   （[ADR-105](../../../../docs/010-adr/105.md "ADR-105")）：19 份「我要不要置
//!   `degraded`」的决策收敛成框架里的 1 份，投影作者没有自觉可用。
//! ## 预算语义（框架层统一，[ADR-105](../../../../docs/010-adr/105.md "ADR-105")）
//!
//! `apply` 的预算是**输入侧窗口截断**：`budget.ms` 为扫描配额（1 事件 = 1ms，
//! 沿用 recall 的记账口径），超窗只把**切片头部** `budget.ms` 条交给闭包
//! （调用方给定的切片顺序就是它的事件主张顺序，窗口只截尾——recall 旧配额的
//! 「按序扫、扫满即停」由此归位成同一处），`degraded = true`，`used.ms` = 窗口
//! 实际长度。选输入侧而非输出侧裁剪：框架不知道 `V` 的形状，只有它能在不新增
//! trait 的前提下统一执行。平凡预算
//! [`Budget::generous`](crate::symbio_core::view::Budget::generous)（全部生产调用方的现状）
//! ⇒ 窗口 = 全量 ⇒ 行为与框架接管前逐字一致。
//!
//! 一级强制拦得住「句柄进不来」，拦不住「体内调全局函数」（形参表合法）——
//! 那由静态扫描（`scripts/projection-purity.mjs`，门禁事实）与双跑比对兜底。分级是
//! 诚实划界，不是含糊。可执行论证（含**编译不过**的反向用例）见
//! [`docs/004-plan/verify/projection_purity.rs`](../../../../docs/004-plan/verify/projection_purity.rs)。

use std::marker::PhantomData;

use super::event::{Event, Timestamp};
use super::view::{Budget, View};

/// 投影闭包的冻结形状（F2）——独立成类型别名，让「纯净性约束」有一个名字。
type PureFn<V> = Box<dyn Fn(&[Event], Timestamp) -> V + Send + Sync>;

/// 纯函数投影：唯一入口 `new`，应用时调用方给什么预算 / 时间就得什么。
pub struct Projection<V> {
    f: PureFn<V>,
    _p: PhantomData<V>,
}

impl<V: 'static> Projection<V> {
    /// **唯一构造入口**。不满足纯净形状的函数进不来——这就是编译期强制。
    pub fn new<F>(f: F) -> Self
    where
        F: Fn(&[Event], Timestamp) -> V + Send + Sync + 'static,
    {
        Projection {
            f: Box::new(f),
            _p: PhantomData,
        }
    }

    /// 应用投影。同一事件序列 + 同一参数 ⇒ 逐字节相同的输出（N1，二级由静态扫描守）。
    ///
    /// 预算语义（[ADR-105]）：`budget.ms` 为扫描配额（1 事件 = 1ms，下限 1——
    /// 「空且不标降级」永不出现）；超窗只交出**切片头部**（给定顺序即主张顺序，
    /// 窗口只截尾），截断 ⇒ `degraded`，`used.ms` = 窗口实际长度。
    pub fn apply(&self, events: &[Event], now: Timestamp, budget: Budget) -> View<V> {
        let quota = (budget.ms as usize).max(1);
        let window = if events.len() > quota {
            &events[..quota]
        } else {
            events
        };
        View {
            degraded: window.len() < events.len(),
            used: Budget::new(0, window.len() as u64),
            value: (self.f)(window, now),
        }
    }
}

pub mod autonomy;
pub mod calibration;
pub mod checkpoint;
pub mod conation;
pub mod consolidate;
pub mod cost;
pub mod fallback;
pub mod mutations;
pub mod prompt_fidelity;
pub mod readyset;
pub mod rebuild;
pub mod recall;
mod recall_score;
pub mod reputation;
pub mod slo;
pub mod span;
pub mod transcript;
pub mod turns;
pub mod turnstate;

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
