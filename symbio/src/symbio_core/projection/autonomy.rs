//! `autonomy` 投影（[02 §3.1 外部效标 · 自主程度](../../../../docs/plan/PL-002-能力坐标系.md "PL-002-能力坐标系")：
//! 「无人干预完成率」的系统内复算口径）。
//!
//! 一条任务闭环（`task.asserted`）之前，**是否从未被挂起过**（`task.held`）。
//! `task.held` 的写方是插话抢占（`session/active`）——它是**外部介入**这个事实在
//! 事实源里唯一的痕迹：人或外部条件改变了任务原本的走向。故：
//!
//! ```text
//! 无人干预完成率 = 闭环且全程未 held 的任务数 / 闭环任务数
//! ```
//!
//! ## 为什么用「曾 held」而不是「当前 held」
//!
//! `task.progress` 会解除挂起（任务回到就绪），但**挂起这件事已经发生了**——
//! 系统确实没能独立走完。用「当前 held」会把「被干预过又恢复」读成「全程自主」，
//! 那是把干预洗掉。此处取保守口径：干预过一次就算干预过。
//!
//! ## 为什么分子分母只数已闭环的
//!
//! 在跑的任务还没给出结论（它此刻自主，可能下一秒被挂起）。把它们算进分母会让
//! 读数随「有多少任务正在跑」漂移——那不是自主程度变了，是并发度变了。
//! 未闭环的任务**既不进分子也不进分母**（与 `span` 的「一个事件的会话不进分母」
//! 同一条纪律：不拿「还没发生」冒充「发生了且为零」）。
//!
//! ## 边界（诚实划界）
//!
//! - **只认 `task.held` 这一种干预**：用户改需求、外部接口超时等若没有走 `held`
//!   事件，本投影看不见。它是「抢占式干预」的下界，不是全部干预的上界。
//! - **不判任务质量**：闭环了就算完成（判质量是 `task.rework_created` 与返工机制
//!   的事）。自主程度与「做得对不对」是两条轴，此处不混。

use serde::{Deserialize, Serialize};

use super::super::event::{Entity, Event, EVENT_TASK_ASSERTED, EVENT_TASK_HELD};
use super::super::view::{Budget, View};
use super::Projection;

/// 自主程度视图（**可加**：跨会话合并 = 分子分母分别相加）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AutonomyView {
    /// 已闭环任务数（分母）。
    pub closed: u64,
    /// 其中全程未被挂起的任务数（分子）。
    pub unattended: u64,
}

impl AutonomyView {
    /// 无人干预完成率 ∈ [0, 1]。
    ///
    /// 无已闭环任务 ⇒ `0.0`（**有据的零**：分母为空时率无意义，给 0 而不是 NaN——
    /// 序列化里没有 NaN，且「没有完成过任务」不该读成「自主完成率未知」）。
    pub fn unattended_rate(&self) -> f64 {
        if self.closed == 0 {
            return 0.0;
        }
        self.unattended as f64 / self.closed as f64
    }

    /// 合并多份视图（**跨会话聚合**）：分子分母可加，率是同一套算式在汇总值上的
    /// 再应用——不是第二份口径（与 `SpanView::merge` / `ConationView::merge` 同款）。
    pub fn merge(parts: &[AutonomyView]) -> AutonomyView {
        let mut closed = 0u64;
        let mut unattended = 0u64;
        for p in parts {
            closed += p.closed;
            unattended += p.unattended;
        }
        AutonomyView { closed, unattended }
    }
}

/// `autonomy` 投影：as-of 口径（`ts > now` 一概不见，与其余投影同）。
///
/// 确定性（N1）：同一事件序列 ⇒ 逐字节相同的 [`AutonomyView`]。
pub fn autonomy() -> Projection<AutonomyView> {
    Projection::new(|events: &[Event], now, _b: Budget| {
        // 一个任务两种状态：曾挂起 / 已闭环。BTreeMap 只为读取顺序确定（值本身是
        // 计数，顺序不影响结果——但遍历顺序确定 ⇒ 同一输入逐字节同一输出）。
        let mut held: std::collections::BTreeMap<&str, ()> = std::collections::BTreeMap::new();
        let mut closed: std::collections::BTreeMap<&str, ()> = std::collections::BTreeMap::new();
        for e in events {
            if e.ts > now || e.entity != Entity::Task {
                continue;
            }
            let Some(id) = e.payload.get("task_id").and_then(|v| v.as_str()) else {
                continue;
            };
            match e.kind.as_str() {
                EVENT_TASK_HELD => {
                    held.insert(id, ());
                }
                EVENT_TASK_ASSERTED => {
                    // 闭环即终态：后面再来的 held 不算（任务已经结束了）
                    closed.insert(id, ());
                }
                _ => {}
            }
        }
        let closed_n = closed.len() as u64;
        // 分子 = 闭环 ∧ 未挂起（曾 held 过的排除——干预过一次就算干预过）
        let unattended_n = closed.keys().filter(|id| !held.contains_key(*id)).count() as u64;
        View::ok(
            AutonomyView {
                closed: closed_n,
                unattended: unattended_n,
            },
            Budget::new(0, 0),
        )
    })
}

#[cfg(test)]
#[path = "autonomy.test.rs"]
mod tests;
