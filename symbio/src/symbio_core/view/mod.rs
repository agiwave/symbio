//! 视图与预算 —— 投影产出的共享类型层（[plan/01 §3](../../../../docs/plan/01-核心架构.md)）。
//!
//! 与 [`crate::symbio_core::event`] 同理：`projections`（③）与将来的 `actors`（②）
//! 都只 import 本模块的类型定义，互不持有句柄（[plan/05 §3.1](../../../../docs/plan/05-模块架构.md)）。
//!
//! ## 冻结锚点
//!
//! **F2** 的后半句：投影返回 `View` 而**不是** `Result`——「可降级」是类型义务
//! 而不是约定：超预算的投影返回 `degraded: true`，对话永不因状态爆炸而中断。

use serde::{Deserialize, Serialize};
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

// ── 召回视图（S5，[plan/01 §9](../../../../docs/plan/01-核心架构.md)）────────────────
//
// 住 view 域的理由（[plan/05 §3.1](../../../../docs/plan/05-模块架构.md)）：③ 的投影
// 产出它、② 的检索 Translator 消费它——跨包共享的**视图类型**必须住中性层，
// 谁也不持有谁的句柄。

use crate::symbio_core::event::Timestamp;

/// 一条被召回的记忆。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(dead_code)] // dead-code-allow R-002: 视图类型随投影接线（plan/12 批2），接线后摘除
pub struct RecallEntry {
    /// 来源事件（`memory.encoded`）的 seq——溯源到事实。
    pub seq: u64,
    /// 编码时间（Unix 毫秒）。
    pub ts: Timestamp,
    /// 内容（编码时的正文）。
    pub content: String,
    /// 认知内容标签（七类，[plan/01 §9.1](../../../../docs/plan/01-核心架构.md)；
    /// 「记忆类型」不是枚举，是过滤参数——加第六类只加参数）。
    pub tag: String,
}

/// 召回视图：as-of 可见、未被遗忘、按新近度排序的记忆条目。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[allow(dead_code)] // dead-code-allow R-002: 视图类型随投影接线（plan/12 批2），接线后摘除
pub struct RecallView {
    pub entries: Vec<RecallEntry>,
}

impl RecallView {
    /// 是否包含某内容（跨主体隔离断言 C13 的观测面）。
    #[allow(dead_code)] // dead-code-allow R-002: 视图类型随投影接线（plan/12 批2），接线后摘除
    pub fn contains_content(&self, content: &str) -> bool {
        self.entries.iter().any(|e| e.content == content)
    }
}
