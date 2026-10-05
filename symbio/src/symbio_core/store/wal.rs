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
//! ## 单写者：跨进程写者令牌
//!
//! 「每 thread 一个 FIFO，串行」（[`Store`](super::Store) 五条语义之首）在
//! S4 之前只由**进程内** `RwLock` 保证：两个进程各持一份 `WalStore`，各自
//! `head` 从文件重放算出，`append` 只 append 不校验 ⇒ seq 重号 / 事件交错，
//! 且**没有任何错误信号**（[plan/11 §2-B](../../../../docs/plan/11-多执行器与多主体加固实施方案.md)）。
//!
//! 本实现把它升级为**跨进程**的写者令牌：`open` 对一个**旁挂锁文件**
//! （`<wal>.lock`）取一次非阻塞独占锁（[`File::try_lock`](std::fs::File::try_lock)：
//! Unix `flock` / Windows `LockFileEx`，随句柄关闭自动释放）。
//!
//! | 入口 | 令牌 | `append` |
//! |---|---|---|
//! | [`open`](Self::open) 拿到锁 | 持 | 正常落盘 |
//! | [`open`](Self::open) 拿不到锁 | 不持 | [`AppendError::NotTheWriter`](super::AppendError::NotTheWriter) |
//! | [`open_readonly`](Self::open_readonly) | 从不取锁 | 同上（读方在类型上就不该写） |
//!
//! 拿不到令牌**不报错、降级成「非写者」**：读照常（`range` / `head` 不看令牌），
//! 只有写被拒。这样「第二个写者」拿到的是一个可判别的信号，而不是一个静默的
//! 重复 seq——[plan/11 §4](../../../../docs/plan/11-多执行器与多主体加固实施方案.md)
//! 要的正是这个反向用例。
//!
//! ### 锁为什么落在旁挂文件、而不是 WAL 文件自身
//!
//! 这是 Windows 逼出来的（实测，非偏好）：`LockFileEx` 是**强制锁**——它不只拦
//! 别的写者，也拦**别的句柄的读**。把锁打在 WAL 上，会话进行中的读数口
//! （[`session/stats`](../../plugins/session/stats.rs) 会在任意时刻 `open_readonly`
//! 同一文件）会直接 `ERROR_LOCK_VIOLATION`。Unix 的 `flock` 是劝告锁、没有这个
//! 问题，但两平台必须同一套语义，所以锁打在一个**只有写者会碰**的旁挂文件上：
//! WAL 自身永不被锁，读方照旧自由读。
//!
//! 同一实测还给出第二条约束：`.append(true)` 打开的句柄在 Windows 上**没有**
//! `GENERIC_WRITE`，`set_len` 会 `PermissionDenied`。所以截断（恢复语义）用
//! 独立的写句柄做，锁句柄只负责"我是写者"这一个事实。
//!
//! **重放必须在持锁之后做**：先重放再取锁的话，「重放 → 取锁」之间别的写者落一条，
//! `head` 就落后一格 ⇒ seq 重号。取锁成功者才重放，`head` 才是权威的。
//!
//! ## 提交边界与撕裂行
//!
//! 「已提交」= 完整落在盘上的最后一行。进程写到一半被杀 ⇒ 最后一行是撕裂的
//! （不完整 JSON）⇒ 恢复时丢弃并截断到最后一行完整记录——这是**恢复语义的
//! 一部分**，不是错误。反向用例（S05 §6.4）证明该边界真实存在。
//!
//! 截断是**写方**的语义，因此只有持令牌者才做（[`open_readonly`](Self::open_readonly)
//! 从不截断，拿不到令牌的降级写者也不截断）。

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use super::{AppendError, Seq, Store};
use crate::symbio_core::event::EventEnvelope;

/// 持久化 Store（`store = wal`，[plan/01 §8](../../../../docs/plan/01-核心架构.md) 参数表取值）。
pub struct WalStore<E: EventEnvelope> {
    path: PathBuf,
    /// 写者令牌：`Some` = 本实例持有该事实源的跨进程独占锁，`None` = 非写者。
    ///
    /// 句柄指向**旁挂锁文件**（见模块头），不是 WAL 本身——WAL 永不被锁，
    /// 读方才能随时进。锁随句柄关闭自动释放，故不需要显式解锁。
    writer: Option<File>,
    inner: RwLock<WalInner<E>>,
}

struct WalInner<E: EventEnvelope> {
    events: Vec<E>,
    ids: HashSet<String>,
    head: u64,
}

/// 旁挂锁文件的路径：`<wal>.lock`（同目录、同名 + `.lock`）。
///
/// 用 `with_file_name` 追加后缀而不是 `with_extension`：后者会把 `v2-events.wal`
/// 变成 `v2-events.lock`——丢掉 `.wal` 这一段，两个不同的事实源可能撞到同一个锁文件。
fn lock_path(wal: &Path) -> PathBuf {
    let mut name = wal.file_name().unwrap_or_default().to_os_string();
    name.push(".lock");
    wal.with_file_name(name)
}

impl<E> WalStore<E>
where
    E: EventEnvelope + serde::Serialize + serde::de::DeserializeOwned,
{
    /// 打开（或创建）一个 WAL 文件，并**尝试取得写者令牌**。
    ///
    /// 拿不到令牌（另一个写者持有）⇒ 返回一个可读、但 `append` 恒
    /// [`NotTheWriter`](super::AppendError::NotTheWriter) 的实例——降级不是失败，
    /// 是「这次你不是写者」这个事实本身。
    ///
    /// **恢复在这里发生**：持令牌者重放所有完整行，撕裂的尾行被丢弃并截断
    /// （提交边界，S05 §6.1）；非写者只重放，不动文件。
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        // 先取令牌、再重放（见模块头「重放必须在持锁之后做」）。锁文件用 `create`
        // 是必需的：还不存在的事实源也要能被锁定，否则两个进程会同时认为
        // 「文件还没有 ⇒ 我是第一个写者」。
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(lock_path(&path))?;
        let writer = match lock.try_lock() {
            Ok(()) => Some(lock),
            Err(_) => None,
        };

        let (events, good_len) = Self::replay(&path)?;
        // 只有持令牌的写者才恢复；且 WAL 可能还不存在（锁文件已建、事实源未建是常态）。
        if writer.is_some() && path.exists() {
            let len = std::fs::metadata(&path)?.len();
            if good_len < len {
                // 截断撕裂部分——恢复语义的一部分，只有持令牌的写者有资格做。
                // 用独立的写句柄：锁句柄是为「我是写者」而开的，不兼职改文件
                // （且 `.append(true)` 的句柄在 Windows 上没有 `GENERIC_WRITE`，
                // `set_len` 会 `PermissionDenied`——实测）。
                let f = std::fs::OpenOptions::new().write(true).open(&path)?;
                f.set_len(good_len)?;
                f.sync_all()?;
            }
        }
        Ok(Self::build(path, events, writer))
    }

    /// **只读**打开：只解析，不动文件、不取令牌。
    ///
    /// 与 [`open`](Self::open) 的差别只在「谁有资格写」：恢复（截断撕裂尾行）
    /// 是**写方**的语义，读方不能顺手做——读数口在会话进行中随时可能被调用，
    /// 而写点恒在当前 EOF：读方若在写方落一行的中途截断了它，剩下的字节会接在
    /// 被截掉的位置之后 ⇒ 那一行静默损坏。解析器仍是同一个（[`replay`](Self::replay)），
    /// 撕裂尾行同样丢弃，只是**不改文件**。
    ///
    /// 文件不存在 ⇒ 空仓（「还没有事实源」是常态，读方不该为了读而创建文件）。
    pub fn open_readonly(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let (events, _) = Self::replay(&path)?;
        Ok(Self::build(path, events, None))
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
    fn build(path: PathBuf, events: Vec<E>, writer: Option<File>) -> Self {
        let ids = events.iter().map(|e| e.event_id().to_string()).collect();
        let head = events.len() as u64;
        WalStore {
            path,
            writer,
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
        // 单写者闸门（跨进程）：非写者一律拒写。这是 I1 在并发下的硬边界——
        // 在此之前这里只有进程内 RwLock，第二个进程会静默地写出重复 seq。
        if self.writer.is_none() {
            return Err(AppendError::NotTheWriter);
        }
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
