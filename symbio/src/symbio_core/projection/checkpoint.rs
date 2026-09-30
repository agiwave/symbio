//! `checkpoint` 投影（S4 第 9 步，[plan/05 §4](../../../../docs/plan/05-模块架构.md)：③ 加 checkpoint；
//! [plan/01 §8](../../../../docs/plan/01-核心架构.md) `projection = checkpoint`）。
//!
//! **checkpoint 可序列化**（S05 §5 的 J3 条目：序列化失败有信号，无需额外强制）——
//! 它是「断点」的数据形态：恢复 = 重放 WAL 到断点 seq 之后 + 投影纯函数，**不需要
//! 任何状态迁移代码**（[roadmap/S05 §2](../../../../docs/plan/roadmap/S05-长会话与断点恢复.md)）。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::super::event::{Event, EVENT_THREAD_CHECKPOINT};
use super::super::view::{Budget, View};
use super::Projection;

/// 可序列化的断点状态（`projection = checkpoint` 的产出）。
///
/// `BTreeMap` 保证序列化顺序确定（同一事件序列 ⇒ 逐字节相同的断点，N1 不因
/// 断点本身被破坏）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointState {
    /// 断点覆盖到的最后一条事件 seq。
    pub last_seq: Option<u64>,
    /// 已见事件总数。
    pub event_count: usize,
    /// 按事件名字的计数（内容摘要，可扩展；有序 ⇒ 序列化确定）。
    pub kind_counts: BTreeMap<String, usize>,
}

impl CheckpointState {
    /// 把断点状态打包成 `thread.checkpoint` 事件（**断点也是一条普通事件**，
    /// 落 `thread × progressed` 格子——它自己同样受 I2/I3 约束）。
    pub fn to_event(&self, event_id: impl Into<String>, actor: impl Into<String>) -> Event {
        Event::pending(
            event_id,
            EVENT_THREAD_CHECKPOINT,
            crate::symbio_core::event::Entity::Thread,
            crate::symbio_core::event::Verb::Progressed,
            0,
            actor,
        )
        .with_payload(serde_json::to_value(self).expect("CheckpointState 必可序列化"))
    }
}

/// `checkpoint` 投影：按入参顺序扫描事件，产出截至最后一条事件的可序列化状态。
///
/// 确定性（N1）：同一事件序列 ⇒ 逐字节相同的 [`CheckpointState`]。
pub fn checkpoint() -> Projection<CheckpointState> {
    Projection::new(|events: &[Event], _now, _b: Budget| {
        let mut last_seq = None;
        let mut kind_counts: BTreeMap<String, usize> = BTreeMap::new();
        for e in events {
            if let Some(seq) = e.seq {
                last_seq = Some(seq.value());
            }
            *kind_counts.entry(e.kind.clone()).or_insert(0) += 1;
        }
        View::ok(
            CheckpointState {
                last_seq,
                event_count: events.len(),
                kind_counts,
            },
            Budget::new(0, 0),
        )
    })
}
