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
        let (events, good_len) = Self::replay(&path)?;
        if path.exists() && good_len < path.metadata()?.len() {
            // 截断撕裂部分——恢复语义的一部分。
            let f = std::fs::OpenOptions::new().write(true).open(&path)?;
            f.set_len(good_len)?;
            f.sync_all()?;
        }
        Ok(Self::build(path, events))
    }

    /// **只读**打开：只解析，不动文件。
    ///
    /// 与 [`open`](Self::open) 的差别只在「谁有资格写」：恢复（截断撕裂尾行）
    /// 是**写方**的语义，读方不能顺手做——读数口在会话进行中随时可能被调用，
    /// 而 [`Store::append`] 用的是 append 模式（写点恒在当前 EOF）：读方若在
    /// 写方落一行的中途截断了它，剩下的字节会接在被截掉的位置之后 ⇒ 那一行
    /// 静默损坏。解析器仍是同一个（[`replay`](Self::replay)），撕裂尾行同样
    /// 丢弃，只是**不改文件**。
    ///
    /// 文件不存在 ⇒ 空仓（「还没有事实源」是常态，读方不该为了读而创建文件）。
    pub fn open_readonly(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let (events, _) = Self::replay(&path)?;
        Ok(Self::build(path, events))
    }

    /// 重放到最后一行完整记录（撕裂尾行丢弃——**提交边界**，S05 §6.1）。
    ///
    /// 返回 `(事件, 最后一条完整记录的结束偏移)`；文件不存在 ⇒ 空。
    /// 不写文件：写不写由调用方决定（[`open`](Self::open) 截断 /
    /// [`open_readonly`](Self::open_readonly) 不动）。
    fn replay(path: &Path) -> std::io::Result<(Vec<E>, u64)> {
        let mut events = Vec::new();
        if !path.exists() {
            return Ok((events, 0));
        }
        let file = std::fs::File::open(path)?;
        let mut offset = 0u64;
        let mut good_len = 0u64;
        for line in BufReader::new(file).split(b'\n') {
            let bytes = line?;
            let total = bytes.len() as u64 + 1; // 含换行（末行可能无）
            match serde_json::from_slice::<E>(&bytes) {
                Ok(event) => {
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
        Ok((events, good_len))
    }

    /// 由已重放的事件构造内存形态（`ids` / `head` 都是它的派生量）。
    fn build(path: PathBuf, events: Vec<E>) -> Self {
        let ids = events.iter().map(|e| e.event_id().to_string()).collect();
        let head = events.len() as u64;
        WalStore {
            path,
            inner: RwLock::new(WalInner { events, ids, head }),
        }
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
