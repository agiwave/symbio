//! `store = wal` —— 持久化事实源（v2 阶段 S4 第 9 步，[roadmap/S05 §2](../../../../docs/plan/roadmap/S05-长会话与断点恢复.md)）。
//!
//! ## F1 冻结的第一次兑现
//!
//! [`Store`](super::Store) trait 在 S0 冻结（append / range / head）——本实现
//! **零接口改动**换掉内存形态：调用方把 `EventStore` 换成 `WalStore`，其余代码
//! 一行不动（[plan/05 §4](../../../../docs/plan/05-模块架构.md)：实现替换不构成架构改动）。
//!
//! ## 形态
//!
//! JSONL append-only 文件：每次 append = 一行 JSON + flush。恢复 = 重放文件；
//! **投影是纯函数，恢复只需要重放，不需要状态迁移**（S05 §2 的关键回报）。
//!
//! ## 提交边界与撕裂行
//!
//! 「已提交」= 完整落在盘上的最后一行。进程写到一半被杀 ⇒ 最后一行是撕裂的
//! （不完整 JSON）⇒ 恢复时丢弃并截断到最后一行完整记录——这是**恢复语义的
//! 一部分**，不是错误。反向用例（S05 §6.4）证明该边界真实存在。

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use super::{AppendError, Seq, Store};
use crate::symbio_core::event::EventEnvelope;

/// 持久化 Store（`store = wal`，[plan/01 §8](../../../../docs/plan/01-核心架构.md) 参数表取值）。
pub struct WalStore<E: EventEnvelope> {
    path: PathBuf,
    inner: RwLock<WalInner<E>>,
}

struct WalInner<E: EventEnvelope> {
    events: Vec<E>,
    ids: HashSet<String>,
    head: u64,
}

impl<E> WalStore<E>
where
    E: EventEnvelope + serde::Serialize + serde::de::DeserializeOwned,
{
    /// 打开（或创建）一个 WAL 文件。**恢复在这里发生**：
    /// 重放所有完整行；撕裂的尾行被丢弃并截断（提交边界，S05 §6.1）。
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let mut events = Vec::new();
        let mut ids = HashSet::new();
        let mut good_len = 0u64;
        if path.exists() {
            let file = std::fs::File::open(&path)?;
            let mut offset = 0u64;
            for line in BufReader::new(file).split(b'\n') {
                let bytes = line?;
                let total = bytes.len() as u64 + 1; // 含换行（末行可能无）
                match serde_json::from_slice::<E>(&bytes) {
                    Ok(event) => {
                        ids.insert(event.event_id().to_string());
                        events.push(event);
                        offset += total;
                        good_len = offset;
                    }
                    Err(_) => {
                        // 撕裂尾行：停止重放（后面的内容一律不可信）。
                        break;
                    }
                }
            }
            if good_len < path.metadata()?.len() {
                // 截断撕裂部分——恢复语义的一部分。
                let f = std::fs::OpenOptions::new().write(true).open(&path)?;
                f.set_len(good_len)?;
                f.sync_all()?;
            }
        }
        let head = events.len() as u64;
        Ok(WalStore {
            path,
            inner: RwLock::new(WalInner { events, ids, head }),
        })
    }

    /// 已入库事件数（== `head().value()`；测试与断言用）。
    pub fn len(&self) -> usize {
        self.inner.read().expect("WalStore 锁中毒").events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<E> Store for WalStore<E>
where
    E: EventEnvelope + serde::Serialize + serde::de::DeserializeOwned,
{
    type Event = E;

    fn append(&self, mut event: Self::Event) -> Result<Seq, AppendError> {
        let mut inner = self.inner.write().expect("WalStore 锁中毒");
        if inner.ids.contains(event.event_id()) {
            return Err(AppendError::Duplicate);
        }
        // 先落盘、再提交内存：盘上是唯一权威（崩溃 ⇒ 已落的还在，没落的不算）。
        // 写盘失败 = 不可继续的存储层灾难，直接 panic（不吞错）——静默返回会让
        // 调用方误以为事件已提交，比崩溃更糟的静默失效。
        //
        // seq 必须在序列化**之前**赋上（2026-09-30 实测事故）：否则盘上每行
        // `seq: null`，重开恢复的事件全部丢序——readyset 排序退化为 u64::MAX、
        // seq_monotonic 从 0 重新计数，N2（重放一致性）静默破裂。
        let seq = Seq::new(inner.head);
        event.assign_seq(seq);
        let line = serde_json::to_vec(&event).expect("事件序列化失败（serde 不可能败于自有类型）");
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .expect("WAL 打开失败（存储层灾难，不可静默）");
        file.write_all(&line)
            .and_then(|_| file.write_all(b"\n"))
            .and_then(|_| file.flush())
            .and_then(|_| file.sync_data())
            .expect("WAL 写入失败（存储层灾难，不可静默）");
        inner.ids.insert(event.event_id().to_string());
        inner.events.push(event);
        inner.head += 1;
        Ok(seq)
    }

    fn range(&self, from: Seq) -> Vec<Self::Event> {
        let inner = self.inner.read().expect("WalStore 锁中毒");
        let start = (from.value() as usize).min(inner.events.len());
        inner.events[start..].to_vec()
    }

    fn head(&self) -> Seq {
        Seq::new(self.inner.read().expect("WalStore 锁中毒").head)
    }
}

/// 便捷别名：装 [`crate::symbio_core::event::Event`] 信封的持久化事实源。
pub type EventWalStore = WalStore<crate::symbio_core::event::Event>;

#[cfg(test)]
#[path = "wal.test.rs"]
mod tests;
