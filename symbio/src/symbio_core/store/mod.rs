//! `Store` —— 一切事实的唯一写入口与读取面（v2 ④，[plan/01 §2](../../../../docs/plan/01-核心架构.md)）。
//!
//! ## 冻结锚点 F1
//!
//! trait 签名（`append` / `range` / `head`）**冻结**：修改它 = 架构变更
//! （[plan/03 §1](../../../../docs/plan/03-演进与验证.md)）。
//!
//! ## 五条语义与强制方式
//!
//! | 语义 | 保证 | 强制方式 |
//! |---|---|---|
//! | 单写者 | 每 thread 一个 FIFO，串行 | 运行时（`MemoryStore` 进程内锁；[`wal::WalStore`] **跨进程写者令牌**）+ CI |
//! | 单调 seq | thread 内严格递增、**无跳号** | [`crate::symbio_core::event::Seq`] 私有构造 + 本实现 |
//! | append-only | 接口上**无** `update` / `delete` | 编译期（trait 就没有这些方法） |
//! | 幂等 | `event_id` 重复 ⇒ `Duplicate` | 运行时 |
//! | 原子 | 单事件要么完整在、要么不在 | 运行时 |
//!
//! **实现替换不构成架构改动**：内存 / WAL / 分片 / 分布式都是 `impl Store`，
//! 调用方零改动（S4 长会话的 `wal` 实现从这里长出来，[plan/05 §4](../../../../docs/plan/05-模块架构.md)）。

use std::collections::HashSet;
use std::sync::RwLock;

use super::event::{Event, EventEnvelope, Seq};

/// 唯一写入口的失败形态（[plan/01 §2](../../../../docs/plan/01-核心架构.md)）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppendError {
    /// 幂等去重命中：同 `event_id` 已存在。
    Duplicate,
    /// 并发写冲突（期望的 head 与实际不符——并发 / WAL 实现用）。
    Conflict { expected: Seq, actual: Seq },
    /// 违反单写者约束：本实例**不持**该事实源的写者令牌。
    ///
    /// 谁发这个令牌是各实现的事（[`wal::WalStore`] 用文件锁，**跨进程**）；trait 只
    /// 承诺「拿不到令牌的写入口会**返回**它，而不是静默写坏网格」。在
    /// [plan/11 批 0](../../../../docs/plan/11-多执行器与多主体加固实施方案.md) 之前，
    /// 这个形态全仓零构造——错误定义在那儿，却永远不可达。
    NotTheWriter,
}

/// 事实源（④）。**依赖清单**：只 import 事件契约，无任何出边（F1 的构造侧）。
pub trait Store: Send + Sync {
    type Event: EventEnvelope;

    /// 唯一写入口。分配单调 `Seq` 并落库；幂等键命中 ⇒ `Duplicate`。
    fn append(&self, event: Self::Event) -> Result<Seq, AppendError>;

    /// 读取 `[from, head)` 的事件区间，返回不可变数据（快照，不持有内部锁）。
    fn range(&self, from: Seq) -> Vec<Self::Event>;

    /// 下一个待分配的 seq（= 已有事件数）。
    fn head(&self) -> Seq;
}

/// 内存实现（[plan/05 §4](../../../../docs/plan/05-模块架构.md) S0：「先 `memory` 实现」；
/// 权威参数表 `store = memory`，平凡值）。
///
/// `RwLock` 串行化写入 ⇒ append 内「查重 → 赋 seq → 落库」是原子区间，
/// 单调性与无跳号由构造直接成立，不靠约定。
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
        inner.ids.insert(event.event_id().to_string());
        inner.events.push(event);
        inner.head += 1;
        Ok(seq)
    }

    fn range(&self, from: Seq) -> Vec<Self::Event> {
        let inner = self.inner.read().expect("MemoryStore 锁中毒");
        let start = (from.value() as usize).min(inner.events.len());
        inner.events[start..].to_vec()
    }

    fn head(&self) -> Seq {
        Seq::new(self.inner.read().expect("MemoryStore 锁中毒").head)
    }
}

/// 便捷别名：装 [`crate::symbio_core::event::Event`] 信封的事实源。
pub type EventStore = MemoryStore<Event>;

pub mod wal;

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
