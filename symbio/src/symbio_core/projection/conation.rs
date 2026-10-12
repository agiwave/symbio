//! `conation` 投影（S12 第 21 步，[roadmap/S12 §3](../../../../docs/stage/ST-012-自主层与长期目标.md "ST-012-自主层与长期目标")、
//! [02 §2.3](../../../../docs/plan/PL-002-能力坐标系.md "PL-002-能力坐标系")）。
//!
//! **「欲」的读侧形态**：从事件序列算出「当前表达了哪些欲、各自有没有升格成任务」。
//! 与 [`readyset`](super::readyset) 同形——就绪判定是纯函数，这里「欲的升格状态」也是：
//! 同一份事件切片永远算出同一个视图（N1），调度 / 去重都不需要内部状态。
//!
//! ## 为什么这是一条**投影**而不是插件里的一次全表扫
//!
//! 判定「这条长目标声明过没有」原先写在消费方（`plugins/session/heartbeat`）里，直接
//! `range(0).any(…)` 扫事实源。把它收进 ③ 有两个后果，都是好的：判定与它读的事实
//! 同一处（改口径只动一个文件）、消费方拿的是 `View` 而不是自己拼的布尔。可执行论证
//! 见 [`verify/conation_minimal.rs`](../../../../docs/plan/verify/conation_minimal.rs) 的
//! `current_intents`（同一条口径：`(日志, now, budget) → 意图集合`）。
//!
//! ## 两条口径
//!
//! - **升格靠溯源认**：一条欲「已升格」当且仅当存在一条 `task.opened` 的 `produced_by`
//!   指向它——这正是 [`IntentGate`](crate::symbio_core::IntentGate) 批准后
//!   `AutonomousInitiator::open_long_goal` 落格的形状（`from_seq` = 欲事件 seq）。
//!   不靠 goal 字符串相等：字符串相等会把**别的**来源开的同目标任务也算成这条欲的产物。
//! - **as-of**：形参 `now` 就是 as-of 锚（`ts > now` 的欲与升格一概不见），与其余投影同口径。
//!   全量复算（「这条目标**曾经**声明过吗」）传 `i64::MAX`——与 `session/stats` 的复算一致。
//!
//! ## 边界（诚实划界）
//!
//! - **溯源合法性不在这里判**：`conation.expressed` 缺 `produced_by` 时
//!   [`ConationCandidate::from_event`](crate::symbio_core::ConationCandidate::from_event)
//!   返回 `None`（I2 强化）。本投影只读事实，不替闸门做准入——来路不明的欲照样入视图
//!   （它是**事实**，藏起来才是撒谎），但它永远不会有 `task_id`（升不了格）。
//! - **「完成」没有独立事件**：今天没有 `conation.fulfilled`，欲一旦升格就一直是
//!   「已声明」——所以本视图只表达「已表达 / 已升格」两态，不假装知道完成与否。

use super::super::event::{Entity, Event, Timestamp, EVENT_CONATION_EXPRESSED, EVENT_TASK_OPENED};
use super::super::view::{Budget, View};
use super::Projection;

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// 一条已表达的「欲」及其升格状态。
///
/// **可序列化**：`session/stats` 的主动性列（02 §3.1 外部效标）原样下发——
/// 与 `CalibrationState` / `CheckpointState` 同族，J3（序列化失败有信号）的前提。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConationIntent {
    /// `conation.expressed` 事件的 seq——也是候选意图的溯源锚（I2）。
    pub seq: u64,
    /// 表达时刻（Unix 毫秒）。
    pub ts: Timestamp,
    /// 欲的内容（`conation.expressed` 载荷的 `goal`）。
    pub goal: String,
    /// 已升格为任务 ⇒ 派生任务的 id；未升格 ⇒ `None`。
    pub task_id: Option<String>,
}

/// 欲视图：按 seq 升序（欲是**流**，顺序即发生顺序——BTreeMap 收集 ⇒ 逐字节确定）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ConationView {
    pub intents: Vec<ConationIntent>,
}

impl ConationView {
    /// 该目标是否**已声明**（存在一条已升格为任务的同目标欲）。
    ///
    /// 生产消费方：`plugins/session/heartbeat` 的长目标去重判据——心跳按 `interval_seconds`
    /// 周期性表达同一条欲，每次都开格会让就绪集无界增长，而 `readyset` 是模型唯一的调度
    /// 候选来源。故「声明过就不再声明」（重新发起是 S03 返工机制的事，不由心跳每 tick 重开）。
    pub fn is_declared(&self, goal: &str) -> bool {
        self.intents
            .iter()
            .any(|i| i.goal == goal && i.task_id.is_some())
    }

    /// 已升格为任务的欲的条数（「被采纳」：`IntentGate` 批准后开了格）。
    pub fn escalated(&self) -> usize {
        self.intents.iter().filter(|i| i.task_id.is_some()).count()
    }

    /// **自发发起且被采纳率** = 已升格 / 已表达（[02 §3.1 外部效标 · 主动性](../../../../docs/plan/PL-002-能力坐标系.md "PL-002-能力坐标系")）。
    ///
    /// 分母含「被否决 / 被忽略」的欲（表达过但没升格——升不了格的欲照常入视图，
    /// 它是事实，见模块头「边界」）。无表达 ⇒ 0.0（**有据的零**：没主动发起过，
    /// 不是「采纳率未知」）。投影只报数，达标判定归读方。
    pub fn adoption_rate(&self) -> f64 {
        if self.intents.is_empty() {
            return 0.0;
        }
        self.escalated() as f64 / self.intents.len() as f64
    }
}

/// 跨会话汇总（实例级聚合列 `fleet` 的主动性族）。
///
/// 只收分子 / 分母（率由 `adoption_rate` 从汇总值重算）——与
/// [`FallbackRateView::merge`](super::fallback::FallbackRateView::merge) 同款：
/// 合并的是**可加的量**，率是同一套算式在汇总值上的再应用，不是另一份口径。
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ConationSummary {
    /// 各会话已表达的欲的总数。
    pub expressed: usize,
    /// 其中已升格的总数。
    pub escalated: usize,
}

impl ConationSummary {
    /// 跨会话采纳率（无表达 ⇒ 0.0，与 [`ConationView::adoption_rate`] 同口径）。
    pub fn adoption_rate(&self) -> f64 {
        if self.expressed == 0 {
            return 0.0;
        }
        self.escalated as f64 / self.expressed as f64
    }
}

impl ConationView {
    /// 合并多份视图（**跨会话聚合**，与 `SloLatencyView::merge` 同地位：
    /// 统计口径单点，不进调用方）。输入为空 ⇒ 空汇总（有据的空）。
    pub fn merge(parts: &[ConationView]) -> ConationSummary {
        let mut expressed = 0usize;
        let mut escalated = 0usize;
        for p in parts {
            expressed += p.intents.len();
            escalated += p.escalated();
        }
        ConationSummary { expressed, escalated }
    }
}

/// `conation` 投影：从事件序列算出当前表达过的欲及其升格状态。
///
/// 预算**不设配额**（同 [`readyset`](super::readyset)）：欲是**流**，截断它等于悄悄
/// 丢掉「想要过什么」的事实；`used` 仍如实记扫描条数（I3 的记账口径）。
pub fn conation() -> Projection<ConationView> {
    Projection::new(|events: &[Event], now, _budget: Budget| {
        let mut intents: Vec<ConationIntent> = Vec::new();
        // 欲 seq → 在 `intents` 里的下标（升格事件可能先于/晚于欲事件到达，故两趟合并）。
        let mut index: BTreeMap<u64, usize> = BTreeMap::new();
        // 欲 seq → 派生任务 id（task.opened 的 produced_by 指回欲）。
        let mut promoted: BTreeMap<u64, String> = BTreeMap::new();
        let mut scanned = 0u64;
        for e in events {
            if e.ts > now {
                continue; // as-of：未来的欲与升格一概不见
            }
            match e.entity {
                Entity::Conation => {
                    if e.kind.as_str() == EVENT_CONATION_EXPRESSED {
                        scanned += 1;
                        let seq = e.seq.map(|s| s.value()).unwrap_or(u64::MAX);
                        let goal = e
                            .payload
                            .get("goal")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        index.insert(seq, intents.len());
                        intents.push(ConationIntent {
                            seq,
                            ts: e.ts,
                            goal,
                            task_id: None,
                        });
                    }
                }
                Entity::Task => {
                    if e.kind.as_str() == EVENT_TASK_OPENED {
                        scanned += 1;
                        if let (Some(src), Some(tid)) = (
                            e.produced_by,
                            e.payload.get("task_id").and_then(|v| v.as_str()),
                        ) {
                            promoted.insert(src, tid.to_string());
                        }
                    }
                }
                _ => {}
            }
        }
        for (src, tid) in promoted {
            if let Some(&i) = index.get(&src) {
                intents[i].task_id = Some(tid);
            }
        }
        // 欲是流：按发生顺序（seq 升序）排，不打分、不按时间倒序。
        intents.sort_by_key(|i| i.seq);
        View {
            value: ConationView { intents },
            degraded: false,
            used: Budget::new(0, scanned),
        }
    })
}

#[cfg(test)]
#[path = "conation.test.rs"]
mod tests;
