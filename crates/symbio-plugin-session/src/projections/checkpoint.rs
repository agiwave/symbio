//! `checkpoint` 投影（[DE-018 §4](../../../../docs/003-design/018-模块架构.md "DE-018-模块架构")：③ 加 checkpoint；
//! [DE-001 §8](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构") `projection = checkpoint`）。
//!
//! **checkpoint 可序列化**（DE-035 §5 的 J3 条目：序列化失败有信号，无需额外强制）——
//! 它是「断点」的数据形态：恢复 = 重放 WAL 到断点 seq 之后 + 投影纯函数，**不需要
//! 任何状态迁移代码**（[DE-035 §2](../../../../docs/003-design/035-长会话与断点恢复.md "DE-035-长会话与断点恢复")）。

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use symbio_core::{Event, Projection};

/// 可序列化的断点状态（`projection = checkpoint` 的产出）。
///
/// **断点是派生视图，不是落盘的锚**：恢复 = 重放 WAL + 投影纯函数（见模块头），
/// 所以没有「把状态再写回事件」的写方——判定即以此为准（要落锚，属 S4，
/// 不是本投影的事）。
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

/// `checkpoint` 投影：按入参顺序扫描事件，产出截至最后一条事件的可序列化状态。
///
/// 确定性（N1）：同一事件序列 ⇒ 逐字节相同的 [`CheckpointState`]。
pub fn checkpoint() -> Projection<CheckpointState> {
    Projection::new(|events: &[Event], _now| {
        let mut last_seq = None;
        let mut kind_counts: BTreeMap<String, usize> = BTreeMap::new();
        for e in events {
            if let Some(seq) = e.seq {
                last_seq = Some(seq.value());
            }
            *kind_counts.entry(e.kind.clone()).or_insert(0) += 1;
        }
        CheckpointState {
            last_seq,
            event_count: events.len(),
            kind_counts,
        }
    })
}
