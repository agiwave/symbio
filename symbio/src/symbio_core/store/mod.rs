//! `Store` —— 一切事实的唯一写入口与读取面（v2 ④，[plan/01 §2](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
//!
//! ## 冻结锚点 F1
//!
//! trait 签名（`append` / `range` / `head`）**冻结**：修改它 = 架构变更
//! （[plan/03 §1](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）。
//!
//! ## 五条语义与强制方式
//!
//! | 语义 | 保证 | 强制方式 |
//! |---|---|---|
//! | 单写者 | 每 thread 一个 FIFO，串行 | 运行时（实现自带进程内锁；持久化实现另有**跨进程写者令牌**）+ CI |
//! | 单调 seq | thread 内严格递增、**无跳号** | [`crate::symbio_core::event::Seq`] 私有构造 + 本实现 |
//! | append-only | 接口上**无** `update` / `delete` | 编译期（trait 就没有这些方法） |
//! | 幂等 | `event_id` 重复 ⇒ `Duplicate` | 运行时 |
//! | 原子 | 单事件要么完整在、要么不在 | 运行时 |
//!
//! **实现替换不构成架构改动**：内存 / 持久文件 / 分片 / 分布式都是 `impl Store`，
//! 调用方零改动（[plan/05 §4](../../../../docs/003-design/018-模块架构.md "DE-018-模块架构")）。
//!
//! 具体实现住在需要它的插件里——持久化布局（文件格式 / 锁 / 归档段）不是机制层的
//! 词汇。本模块只留端口与平凡值 `MemoryStore`。

use std::collections::HashSet;
use std::sync::RwLock;

use super::event::{Cell, Event, EventEnvelope, Seq};

/// 写侧闸门拒绝的是**哪一条**不变量。
///
/// 具名而不是字符串：调用方要能按条分流（I1 写坏了网格 / I2 不可追溯 / I3 没到点必答），
/// 日志与测试也要能钉住「拒的是哪一条」，而不是匹配一段人话。
///
/// **为什么住在 `store` 域**：它是**端口拒绝词汇**的一部分（[`AppendError`] 的载荷），
/// 不是读侧体检的实现。放进 `invariants` 会让 `store` 依赖 `invariants`，
/// 在 core 的域图里闭出一条新环（`store → invariants → projection → store`）——
/// 读侧体检可以认识「哪一条不变量」，写侧端口只认「拒绝的理由有个名字」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreInvariant {
    /// I1 单通道：每轮一条开轮、一条收束，且只经唯一写入口。
    I1,
    /// I2 无溯源不声明：声明类事件必须带 `produced_by`。
    I2,
    /// I3 到点必答：开过的轮必须收束，不得被下一轮越过。
    I3,
}

impl std::fmt::Display for StoreInvariant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            StoreInvariant::I1 => "I1 单通道",
            StoreInvariant::I2 => "I2 无溯源不声明",
            StoreInvariant::I3 => "I3 到点必答",
        })
    }
}

/// 唯一写入口的失败形态（[plan/01 §2](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppendError {
    /// 幂等去重命中：同 `event_id` 已存在。
    Duplicate,
    /// 并发写冲突（期望的 head 与实际不符——并发 / WAL 实现用）。
    Conflict { expected: Seq, actual: Seq },
    /// 违反单写者约束：本实例**不持**该事实源的写者令牌。
    ///
    /// 谁发这个令牌是各实现的事（持久化实现用旁挂文件锁，**跨进程**）；trait 只
    /// 承诺「拿不到令牌的写入口会**返回**它，而不是静默写坏网格」。在
    /// plan/11 批 0 之前，
    /// 这个形态全仓零构造——错误定义在那儿，却永远不可达。
    NotTheWriter,
    /// **写侧不变量闸门**拒绝：
    /// 这条事件会把事实网格写坏（同轮第二条 final / 声明缺溯源）。
    ///
    /// 与 `Duplicate` 的分界：后者是**同一幂等键重放**（幂等语义，
    /// 重复本身无害）；本形态拒绝的是一条**新的**事件——写进去就是一份坏事实。
    Invariant {
        /// 违反的是哪一条（具名，不是字符串）。
        invariant: StoreInvariant,
        /// 被拒事件的幂等键（与读侧 `Violation.event_id` 同一口径）。
        event_id: String,
        /// 人话原因（与读侧 `Violation.why` 同一套措辞来源）。
        why: String,
    },
    /// 名字与格位对不上（段四）：
    /// `kind` 不在事件名字表上，或事件自述的 `entity × verb` 不是表上那一格。
    ///
    /// **为什么它不是 `Invariant` 的一条**：I1/I2/I3 判的是事实**之间**
    /// 的关系（同轮不重复、声明要溯源、开了要收束），本形态判的是这条事件**自己的契约
    /// 自述**——名字与格子是不是同一个位置。它是 J3 的运行侧：枚举把行与列钉在编译期，
    /// 名字这一维是数据（01 §6），数据的对齐只能在入库处判（表在
    /// [`crate::symbio_core::event_cell`]）。
    ///
    /// 读侧投影按 `(entity, verb, kind)` 三目取事实，格位漂了的表现是**静默少一条事实**
    /// （没有任何东西会红）——这正是本形态要在写入口拒掉的东西。
    OffGrid {
        event_id: String,
        /// 事件自报的名字。
        kind: String,
        /// 名字表给的格位；`None` = 这个名字**不在表上**（加名字忘登记就是这个形状）。
        declared: Option<Cell>,
        /// 事件自己声明的格位。
        given: Cell,
    },
}

/// 事实源（④）。**依赖清单**：只 import 事件契约，无任何出边（F1 的构造侧）。
pub trait Store: Send + Sync {
    type Event: EventEnvelope;

    /// 唯一写入口。分配单调 `Seq` 并落库；幂等键命中 ⇒ `Duplicate`。
    fn append(&self, event: Self::Event) -> Result<Seq, AppendError>;

    /// 读取 `[from, head)` 的事件区间，返回不可变数据（快照，不持有内部锁）。
    ///
    /// **`from` 是号，不是下标**：实现内部的窗口首号可以不是 0（持久化实现把被快照
    /// 取代的前缀移出了当前窗口），按下标切会把「从 seq k 起」读成「从窗口第 k 条起」。
    ///
    /// **一个读口，一种口径**：`from` 起的全部事实**含被取代的前缀**，按号序。
    /// 「被取代」是**折叠侧**的事——投影从最近一份快照起折，前缀自然不参与
    /// （见 `mutations_latest_snapshot`）；它不是存储层的第二种读法。按布局分叉的
    /// 两个口（只给窗口 / 含归档）会把「这条会话压缩过没有」的知识漏给调用方
    /// （[plan/14 §2.2](../../../../docs/003-design/014-单环一表.md "DE-014-单环一表")
    /// 推论 1）；要窗口就按 `Seq` 自己切。
    ///
    /// 缺号的事件（旧版本写下的行）不属于任何 `[from, head)` 区间，只在 `from = 0`
    /// 时出现。
    fn range(&self, from: Seq) -> Vec<Self::Event>;

    /// 下一个待分配的 seq（= 已有事件数）。
    fn head(&self) -> Seq;
}

/// 内存实现（[plan/05 §4](../../../../docs/003-design/018-模块架构.md "DE-018-模块架构") S0：「先 `memory` 实现」；
/// 权威参数表 `store = memory`，平凡值）。
///
/// `RwLock` 串行化写入 ⇒ append 内「查重 → 赋 seq → 落库」是原子区间，
/// 单调性与无跳号由构造直接成立，不靠约定。
///
/// **不执行 I1/I2/I3**：三条不变量判的是事实**之间**的关系（同轮至多一条 final /
/// 声明必须有更早的 `produced_by` / 开过的轮必须收束），需要轮号索引与溯源解析。
/// 执行方是持久化实现的唯一写入口 `WalStore::gate`；本类型只服务测试与 S0 基线。
/// 生产权威恒为持久化实现——把本类型当生产权威的路径必须先换掉，而不是给它补一套闸门
/// （[D-134](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
pub struct MemoryStore<E: EventEnvelope> {
    inner: RwLock<MemoryInner<E>>,
}

struct MemoryInner<E: EventEnvelope> {
    events: Vec<E>,
    /// 已入库的幂等键。
    ids: HashSet<String>,
    /// 下一个待分配 seq。
    head: u64,
}

impl<E: EventEnvelope> MemoryStore<E> {
    pub fn new() -> Self {
        MemoryStore {
            inner: RwLock::new(MemoryInner {
                events: Vec::new(),
                ids: HashSet::new(),
                head: 0,
            }),
        }
    }

    /// 已入库事件数（== `head().value()`；测试与断言用）。
    pub fn len(&self) -> usize {
        self.inner.read().expect("MemoryStore 锁中毒").events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<E: EventEnvelope> Default for MemoryStore<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: EventEnvelope> Store for MemoryStore<E> {
    type Event = E;

    fn append(&self, mut event: Self::Event) -> Result<Seq, AppendError> {
        let mut inner = self.inner.write().expect("MemoryStore 锁中毒");
        if inner.ids.contains(event.event_id()) {
            return Err(AppendError::Duplicate);
        }
        let seq = Seq::new(inner.head);
        event.assign_seq(seq);
        // 刻意**不**做时刻兜底：MemoryStore 是平凡基线（`store = memory`，S0），
        // 大量测试靠 `ts = 0`（时间起点）+ 小 `now` 做 as-of 断言——兜底会毁掉
        // 这份确定性。时刻完整性由**权威事实源**（持久化实现）负责。
        inner.ids.insert(event.event_id().to_string());
        inner.events.push(event);
        inner.head += 1;
        Ok(seq)
    }

    fn range(&self, from: Seq) -> Vec<Self::Event> {
        // 与持久化实现同一条契约（**`from` 是号不是下标**、缺号只在 `from = 0` 出现）。
        // 内存实现里 `append` 恒赋号，两者今天恰好等价——但等价不是契约，
        // 各写一遍就会各漂一次。
        let from = from.value();
        let inner = self.inner.read().expect("MemoryStore 锁中毒");
        if from == 0 {
            return inner.events.clone();
        }
        inner
            .events
            .iter()
            .filter(|e| e.seq().is_some_and(|s| s.value() >= from))
            .cloned()
            .collect()
    }

    fn head(&self) -> Seq {
        Seq::new(self.inner.read().expect("MemoryStore 锁中毒").head)
    }
}

/// 便捷别名：装 [`crate::symbio_core::event::Event`] 信封的事实源。
pub type EventStore = MemoryStore<Event>;

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;

// ── panic 面登记（PN-001…003）─────────────────────────────────────────
// 本文件每一处 `unwrap` / `expect` / `panic!` / `unreachable!` 的理由。登记放在
// 文件内而不是集中一张表：理由与它解释的那段代码会一起被 review、一起被删。
// 判据见 `scripts/panic-audit.mjs`。**加一处 panic 必须同时加一行登记，理由非空。**
// panic-allow symbio/src/symbio_core/store/mod.rs::len: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/symbio_core/store/mod.rs::append: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/symbio_core/store/mod.rs::range: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/symbio_core/store/mod.rs::head: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
