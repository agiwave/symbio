//! `readyset` 投影（S7 第 16 步，[roadmap/S03 §3](../../../../docs/plan/roadmap/S03-多步任务与返工.md)、
//! [roadmap/S04](../../../../docs/plan/roadmap/S04-并发调度与租约.md)）。
//!
//! **就绪判定是纯函数，从事件序列算**：`ready = 非终态 ∧ depends_on 全部终态`。
//! 调度器不需要任何内部状态——同一份事件切片永远算出同一个就绪集（N1）。
//!
//! ## 边界（S03/S04 §7 诚实划界）
//!
//! - 优先级、公平性、饥饿——都是 `readyset` 的**打分参数**，不是机制；
//!   选什么打分函数是算法问题，架构不保证（本投影按 `task_id` 字典序，确定性优先）；
//! - 并发抢占与租约（`held` 格的占用判定）是 S04 的增量，本投影先按
//!   「终态 + 依赖闭合」取集——容量参数由调用方的调度器决定。

use super::super::event::{
    Entity, Event, EVENT_TASK_ASSERTED, EVENT_TASK_HELD, EVENT_TASK_OPENED, EVENT_TASK_PROGRESS,
};
use super::super::view::{Budget, View};
use super::Projection;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 一条就绪任务。
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ReadyTask {
    /// 任务 id（`task.opened` 载荷的 `task_id`）。
    pub task_id: String,
    /// 开任务事件的 seq（调度器取事件的定位锚）。
    pub seq: u64,
    /// 依赖（已全部终态——放进就绪集时必然满足）。
    pub depends_on: Vec<String>,
}

/// 就绪集视图：按 `task_id` 字典序（BTreeMap ⇒ 逐字节确定）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReadySetView {
    pub ready: Vec<ReadyTask>,
}

/// `readyset` 投影：从事件序列算出当前就绪的任务。
///
/// - 终态 = `task.asserted`（验收通过才终态——返工节点重开后不算旧任务复活）；
/// - `ts > now` 的事件不参与（与其余投影同口径）；
/// - 环与悬空依赖不在这里判——那是 C14（[`crate::symbio_core::invariants::acyclic_deps`]）
///   的职责；本投影在有环时诚实返回「依赖未闭合」的空子集（跑不完 ≠ 假装能跑）；
/// - **挂起排除**（S07 §5 强制点）：`task.held` 的任务不进就绪集——被挂起的任务
///   被二次调度可能重复执行；恢复（同任务的 `task.progress`）自动解除挂起。
pub fn readyset() -> Projection<ReadySetView> {
    Projection::new(|events: &[Event], now, _budget: Budget| {
        // task_id → (opened 事件 seq, depends_on)
        let mut tasks: BTreeMap<&str, (u64, Vec<String>)> = BTreeMap::new();
        let mut terminal: BTreeMap<&str, ()> = BTreeMap::new();
        let mut held: BTreeMap<&str, ()> = BTreeMap::new();
        let mut scanned = 0u64;
        for e in events {
            if e.ts > now || e.entity != Entity::Task {
                continue;
            }
            let Some(id) = e.payload.get("task_id").and_then(|v| v.as_str()) else {
                continue;
            };
            match e.kind.as_str() {
                EVENT_TASK_OPENED => {
                    scanned += 1;
                    let ds = e
                        .payload
                        .get("depends_on")
                        .and_then(|v| v.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str())
                                .map(String::from)
                                .collect()
                        })
                        .unwrap_or_default();
                    tasks.insert(id, (e.seq.map(|s| s.value()).unwrap_or(u64::MAX), ds));
                }
                EVENT_TASK_ASSERTED => {
                    scanned += 1;
                    terminal.insert(id, ());
                    held.remove(id);
                }
                EVENT_TASK_HELD => {
                    // 挂起：S07 §5 的强制点——挂起期间不得被二次调度。
                    scanned += 1;
                    held.insert(id, ());
                }
                EVENT_TASK_PROGRESS => {
                    // 恢复：挂起是一条事件，恢复也是——progressed 自动解除挂起。
                    held.remove(id);
                }
                _ => {}
            }
        }
        let ready = tasks
            .iter()
            .filter(|(id, (_, ds))| {
                !terminal.contains_key(*id)
                    && !held.contains_key(*id)
                    && ds.iter().all(|d| terminal.contains_key(d.as_str()))
            })
            .map(|(id, (seq, ds))| ReadyTask {
                task_id: (*id).to_string(),
                seq: *seq,
                depends_on: ds.clone(),
            })
            .collect();
        View {
            value: ReadySetView { ready },
            degraded: false,
            used: Budget::new(0, scanned),
        }
    })
}
