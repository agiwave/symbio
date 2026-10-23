//! `Projection` —— 纯函数视图（v2 ③，[plan/01 §3](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
//!
//! ## 冻结锚点 F2（构造形状，不是签名约定）
//!
//! 投影 = 一族纯函数。**字段私有 + 唯一构造入口 `new`**，而 `new` 的泛型约束
//! `F: Fn(&[Event], Timestamp) -> V` 就是「纯净性」在类型层的全部表达：
//!
//! - 形参里**没有** `&Store` / `&mut` / `Clock` / `Principal` —— 想拿这些句柄，
//!   构造那一刻就编译不过（一级强制，J3）；
//! - 闭包**只返回 `V`**——「可降级」与「预算」由 [`Projection::apply`] 统一执行
//!   （[D-105](../../../../docs/003-design/001-核心架构.md#d-105-投影预算语义收敛到框架层闭包只返回-v窗口截断由-apply-统一执行 "DE-001-核心架构")）：19 份「我要不要置
//!   `degraded`」的决策收敛成框架里的 1 份，投影作者没有自觉可用。
//! ## 预算语义（框架层统一，[D-105](../../../../docs/003-design/001-核心架构.md#d-105-投影预算语义收敛到框架层闭包只返回-v窗口截断由-apply-统一执行 "DE-001-核心架构")）
//!
//! `apply` 的预算是**输入侧窗口截断**：`budget.ms` 为扫描配额（1 事件 = 1ms，
//! 沿用 recall 的记账口径），超窗只把**切片头部** `budget.ms` 条交给闭包
//! （调用方给定的切片顺序就是它的事件主张顺序，窗口只截尾——recall 旧配额的
//! 「按序扫、扫满即停」由此归位成同一处），`degraded = true`，`used.ms` = 窗口
//! 实际长度。选输入侧而非输出侧裁剪：框架不知道 `V` 的形状，只有它能在不新增
//! trait 的前提下统一执行。平凡预算
//! `Budget::generous`（全部生产调用方的现状）
//! ⇒ 窗口 = 全量 ⇒ 行为与框架接管前逐字一致。
//!
//! 一级强制拦得住「句柄进不来」，拦不住「体内调全局函数」（形参表合法）——
//! 那由静态扫描（`scripts/projection-purity.mjs`，门禁事实）与双跑比对兜底。分级是
//! 诚实划界，不是含糊。可执行论证（含**编译不过**的反向用例）见
//! `docs/003-design/verify/projection_purity.rs`。

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
    /// 预算语义（[D-105]）：`budget.ms` 为扫描配额（1 事件 = 1ms，下限 1——
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

    /// **全量读数**：`apply(events, i64::MAX, Budget::generous())` 的具名口径。
    ///
    /// 两个参数各是一条**口径**，不是一次选择：
    /// - `now = i64::MAX`：WAL 是**落盘**事实源，文件里每一条都已发生，没有「未来的
    ///   事件」可滤（as-of 口径见 `plugins/session/stats.rs` §as-of）；写小一寸不是
    ///   「更保守」，是**少读一寸事实**，而且没有任何东西会为此变红；
    /// - `Budget::generous()`：窗口 = 全量 ⇒ `degraded` 恒假（框架接管前全部生产调用方
    ///   的现状，见本模块头）。
    ///
    /// 具名之前，这一对在**生产侧 26 个读数点**逐字写着（`stats` 一处的 16 列 +
    /// 各插件的 10 处直调）：读侧闸要落在哪、as-of 口径归谁定，散在调用方各说一遍。
    /// 具名之后，**加一道读侧闸或改一次口径只有一个地方改**
    /// （⑧）。
    ///
    /// **有意的窄读数不许用它**：扫描配额（召回的 `RECALL_SCAN_QUOTA`）与当前窗口
    /// （`turnstate` 只读本轮）各自是一条判据，那些调用方继续写 `apply`——
    /// 名字要留给「全量」这一种，否则「窄」就从例外变成看不见的默认。
    pub fn read_all(&self, events: &[Event]) -> View<V> {
        self.apply(events, i64::MAX, Budget::generous())
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
