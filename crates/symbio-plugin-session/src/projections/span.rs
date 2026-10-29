//! `span` 投影（[DE-008 §3.1 外部效标 · 时域跨度](../../../../docs/003-design/008-能力注册表与能力坐标系.md "DE-008-能力注册表与能力坐标系")：
//! 「可完成任务时长」的系统内复算口径）。
//!
//! 会话级跨度 = 末事件 `ts` − 首事件 `ts`（Unix 毫秒）。`ts` 是**入库时间**
//! （[`Event`] 信封的元数据，不是载荷自填）——所以跨度天然含系统自身开销，
//! 与 `cost_ms` 同源（D-044：实测与判据同源）。
//!
//! 单会话一次 apply 产出一个 span；跨会话中位数由 [`SpanView::merge`] 收齐各会话
//! 的 span 后按 P50 取（最近邻取法与 `TierLatency::percentile`
//! 同款——「P50 是什么」全仓一个答案）。
//!
//! ## 为什么「一个事件的会话」不进中位数
//!
//! 0 / 1 事件的会话跨度恒为 0，进分母会把中位数系统性往下拉——那不是「任务更快」，
//! 是「还没有任务」被读成了「零时长任务」。故 `merge` 只收 `events ≥ 2` 的会话，
//! 计数与样本因此同源（`sessions == spans.len()`）。
//!
//! ## 边界（诚实划界）
//!
//! - **跨度含等待**：会话挂在后台几小时不说话，跨度照算——它度量的是「这份事实源
//!   覆盖了多长时间」，不是「模型思考了多久」。读方要后者请用时延列（`slo_report`）。
//! - **不判「完成」**：有没有跑完是消费方的问题（C2 每轮一条 final 是判据的事），
//!   本投影只报时间事实。

use serde::{Deserialize, Serialize};

use symbio_core::{Event, Projection};

/// 会话级时间跨度（无事件 ⇒ 全零——**有据的零**，不是错误）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SpanView {
    /// 参与跨度计算的事件条数（as-of 之后的；投影口径的一部分，不是附加信息）。
    pub events: u64,
    /// 跨度（毫秒）= 末事件 `ts` − 首事件 `ts`。
    pub span_ms: u64,
}

/// 跨会话汇总（实例级聚合列 `fleet` 的 span 族）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SpanSummary {
    /// 参与合并的会话数（有跨度的才算——与 `spans.len()` 恒等）。
    pub sessions: usize,
    /// 各会话跨度（毫秒，升序；`median_ms` 的前提）。
    pub spans: Vec<u64>,
}

impl SpanSummary {
    /// 时长中位数（P50，最近邻取法）。
    ///
    /// 无样本 ⇒ `None`（**有据的空**，不是 0——「实例里没有会话」与「会话时长
    /// 0ms」是两件事，给 0 就读混了）。
    pub fn median_ms(&self) -> Option<u64> {
        if self.spans.is_empty() {
            return None;
        }
        let n = self.spans.len();
        // 与 TierLatency::percentile 同一取法：ceil(50/100 × n) − 1
        let idx = ((50 * n as u64).div_ceil(100) as usize).min(n) - 1;
        Some(self.spans[idx])
    }
}

impl SpanView {
    /// 合并多份视图（**跨会话聚合**，与 `SloLatencyView::merge` /
    /// `FallbackRateView::merge` 同一地位：统计口径单点，不进调用方）。
    ///
    /// 只收 `events ≥ 2` 的会话（见模块头「为什么」）；输入为空 ⇒ 空汇总。
    pub fn merge(parts: &[SpanView]) -> SpanSummary {
        let mut spans: Vec<u64> = parts
            .iter()
            .filter(|p| p.events >= 2)
            .map(|p| p.span_ms)
            .collect();
        spans.sort_unstable();
        SpanSummary {
            sessions: spans.len(),
            spans,
        }
    }
}

/// `span` 投影：as-of 口径（`ts > now` 一概不见，与其余投影同）。
///
/// 确定性（N1）：同一事件序列 ⇒ 逐字节相同的 [`SpanView`]。
pub fn span() -> Projection<SpanView> {
    Projection::new(|events: &[Event], now| {
        let mut first: Option<i64> = None;
        let mut last: Option<i64> = None;
        let mut n = 0u64;
        for e in events {
            let ts = e.ts; // Timestamp = i64（Unix 毫秒）
            if ts > now {
                continue;
            }
            n += 1;
            first = Some(first.map_or(ts, |f| f.min(ts)));
            last = Some(last.map_or(ts, |l| l.max(ts)));
        }
        SpanView {
            events: n,
            // last ≥ first 恒成立（同一次遍历里赋值），max(0) 只是不赌
            span_ms: (last.unwrap_or(0) - first.unwrap_or(0)).max(0) as u64,
        }
    })
}

#[cfg(test)]
#[path = "span.test.rs"]
mod tests;
