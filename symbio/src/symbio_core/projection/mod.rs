//! `Projection` —— 纯函数视图（v2 ③，[plan/01 §3](../../../../docs/plan/01-核心架构.md)）。
//!
//! ## 冻结锚点 F2（构造形状，不是签名约定）
//!
//! 投影 = 一族纯函数。**字段私有 + 唯一构造入口 `new`**，而 `new` 的泛型约束
//! `F: Fn(&[Event], Timestamp, Budget) -> View<V>` 就是「纯净性」在类型层的全部表达：
//!
//! - 形参里**没有** `&Store` / `&mut` / `Clock` / `Principal` —— 想拿这些句柄，
//!   构造那一刻就编译不过（一级强制，J3）；
//! - 返回 [`View`](crate::symbio_core::view::View) 而非 `Result` —— 「可降级」是类型义务。
//!
//! 一级强制拦得住「句柄进不来」，拦不住「体内调全局函数」（形参表合法）——
//! 那由二级（CI 双跑比对，N1）与静态扫描兜底。分级是诚实划界，不是含糊。
//!
//! 可执行论证（含**编译不过**的反向用例）见
//! [`docs/plan/verify/projection_purity.rs`](../../../../docs/plan/verify/projection_purity.rs)。

use std::marker::PhantomData;

use super::event::{Event, Timestamp};
use super::view::{Budget, View};

/// 投影闭包的冻结形状（F2）——独立成类型别名，让「纯净性约束」有一个名字。
type PureFn<V> = Box<dyn Fn(&[Event], Timestamp, Budget) -> View<V> + Send + Sync>;

/// 纯函数投影：唯一入口 `new`，应用时调用方给什么预算 / 时间就得什么。
pub struct Projection<V> {
    f: PureFn<V>,
    _p: PhantomData<V>,
}

impl<V: 'static> Projection<V> {
    /// **唯一构造入口**。不满足纯净形状的函数进不来——这就是编译期强制。
    pub fn new<F>(f: F) -> Self
    where
        F: Fn(&[Event], Timestamp, Budget) -> View<V> + Send + Sync + 'static,
    {
        Projection {
            f: Box::new(f),
            _p: PhantomData,
        }
    }

    /// 应用投影。同一事件序列 + 同一参数 ⇒ 逐字节相同的输出（N1，二级由双跑比对守）。
    pub fn apply(&self, events: &[Event], now: Timestamp, budget: Budget) -> View<V> {
        (self.f)(events, now, budget)
    }
}

pub mod checkpoint;
pub mod consolidate;
pub mod cost;
pub mod readyset;
pub mod recall;
pub mod reputation;
pub mod turnstate;

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
