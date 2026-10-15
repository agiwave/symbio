//! `store = wal` —— 持久化事实源（v2 阶段 S4 第 9 步，[roadmap/S05 §2](../../../../docs/003-stage/005-长会话与断点恢复.md "ST-005-长会话与断点恢复")）。
//!
//! ## F1 冻结的第一次兑现
//!
//! [`Store`](super::Store) trait 在 S0 冻结（append / range / head）——本实现
//! **零接口改动**换掉内存形态：调用方把 `EventStore` 换成 `WalStore`，其余代码
//! 一行不动（[plan/05 §4](../../../../docs/004-plan/005-模块架构.md "PL-005-模块架构")：实现替换不构成架构改动）。
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
//! 且**没有任何错误信号**（[plan/11 §2-B](../../../../docs/004-plan/009-多执行器与多主体加固实施方案.md "PL-009-多执行器与多主体加固实施方案")）。
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
//! 重复 seq——[plan/11 §4](../../../../docs/004-plan/009-多执行器与多主体加固实施方案.md "PL-009-多执行器与多主体加固实施方案")
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

/// 持久化 Store（`store = wal`，[plan/01 §8](../../../../docs/004-plan/001-核心架构.md "PL-001-核心架构") 参数表取值）。
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

/// 归档段的存放子目录（相对 WAL 所在目录）。
///
/// 与 [`plugins::session::paths`](../../plugins/session/paths.rs) 的
/// `TRANSCRIPTS_SUBDIR` **同字面量**，但**不 import** 它：`symbio_core` 不得依赖
/// plugins（分层，core-naming-audit N-002）。这是有意保留的一处重复——改这里
/// 必须同步改 `paths.rs`（那边有对应注释指向本常量）。
///
/// 放在 `transcripts/` 之下而不是与会话文件同级：VDFS 会话树是**显式构造**的
/// （不走 `read_dir`），故这个子目录不会漏进前端的文件列表。
const WAL_ARCHIVE_SUBDIR: &str = "transcripts/wal-archive";

/// 归档段的路径：`<wal 所在目录>/transcripts/wal-archive/<wal 文件名>.<first>-<last>`。
///
/// 段名带上首末号：①「这段覆盖哪一号区间」在文件名上直接可读；② 同一次归档
/// 重复执行会落到同一个名字（幂等，`File::create` 覆盖），不同次归档天然不同名，
/// 不会互相覆盖。
fn archive_path(wal: &Path, first: u64, last: u64) -> PathBuf {
    let mut name = wal.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{first}-{last}"));
    wal.parent()
        .unwrap_or_else(|| Path::new("."))
        .join(WAL_ARCHIVE_SUBDIR)
        .join(name)
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
        // **head 是「下一个号」，不是「已有几条」**。
        //
        // 两者只在「号从 0 起、无缺口」时相等——而那正是**归档之前**的形态。
        // 段归档把被取代的前缀移走之后，live 文件里的号从 `cut` 起，条数少于号：
        // 此时按条数算 head 会**倒退**（例：live 段只剩 seq 10/11/12，条数 3 ⇒ head=3
        // ⇒ 下一次 append 分配 3，与既有的 10 撞号且不单调——N2 静默破裂）。
        // 故取两者上界：正常文件得 `max_seq+1`（= 条数），归档后得 `max_seq+1`（> 条数），
        // 旧版本写下的 `seq: null` 行退回条数（与归档前的行为逐字相同）。
        let head = events
            .iter()
            .filter_map(|e| e.seq())
            .map(|s| s.value() + 1)
            .max()
            .unwrap_or(0)
            .max(events.len() as u64);
        WalStore {
            path,
            writer,
            inner: RwLock::new(WalInner { events, ids, head }),
        }
    }

    /// **段归档**：把 `seq < cut` 的事件移入归档段，live 文件从 `cut` 起。
    ///
    /// 由 [`EVENT_SESSION_CONSOLIDATED`](crate::symbio_core::EVENT_SESSION_CONSOLIDATED)
    /// 的写方调用（压缩成功之后）：快照**取代**了它之前的一切，那些事件对 live 读侧
    /// 不再有意义（读侧要的是「当前有效的历史」），而它们仍然**不可删**（审计 / 回溯）。
    /// 于是「上下文压缩」与「WAL 无界增长」由同一个机制解决（ADR-053 决策 6）。
    ///
    /// ## 返回值
    ///
    /// - `Ok(Some(n))`：`n` 条事件移入归档段；
    /// - `Ok(None)`：**没有可归档的前缀**（`cut` 落在第一条上），或本实例不持写者令牌
    ///   ——两者都是「这次不归档」，不是失败。调用方在调用前已成功 `append` 过
    ///   （那是它拿到令牌的证据），故第二条实际不可达，留着只为方法自身可独立使用。
    ///
    /// ## 崩溃安全：**先归档、后换 live**
    ///
    /// 1. 把当前 live 文件的**被取代前缀**逐字节写进归档段（先写 `<archive>.tmp`、
    ///    `sync_all`、再 `rename` ⇒ 归档段原子出现），不重序列化 ⇒ 不引入序列化漂移；
    /// 2. 把**尾部**逐行**原样**写进 `<wal>.new`（同样不重序列化），`sync_all`；
    /// 3. `rename(<wal>.new, <wal>)`——原子替换。
    ///
    /// 任一步失败 ⇒ live 文件**一字未动**（归档是增长优化，不是正确性前提）。
    /// 步骤 1 之前 / 之中崩溃：live 完整，归档段尚未出现（`.tmp` 残留无害）。
    /// 步骤 1 与 3 之间崩溃：live 仍完整（归档段多一份前缀副本，重跑幂等）。
    /// **不存在「归档成功但 live 丢了」的窗口**。
    ///
    /// ## 段与段之间不重叠
    ///
    /// 归档段**只装被取代的前缀**，不装尾部。于是「全部归档段（按号序拼接）+
    /// live」恰好等于完整事件史，**逐条不重不漏**——这是 ADR-053 迁移表 b2 行
    /// 「零回退损失」的可执行依据：回退时把各段与 live 依次拼接即得原 WAL。
    /// （若归档段改成复制整份世界，则相邻段的尾部会重复，拼接就不再是无损还原。）
    ///
    /// ## 号与幂等键
    ///
    /// `head`（下一个号）**不变**——归档挪走的是历史，不是号空间。归档段的
    /// `event_id` 随之离开内存里的幂等键集合：重放一条已归档的事件不会再撞
    /// `Duplicate`。这是可接受的——幂等键防的是**同一次操作重复落格**，而归档
    /// 之后不会再有人重放那些操作。
    ///
    /// ## 归档段保留策略：**不自动删除**
    ///
    /// 归档段是「回退零损失」的依据（ADR-053 迁移表 b2 行）：它们不参与常规读，
    /// 但必须还在。删除策略要单独定——那是在决定「哪些事实可以不再可回溯」，
    /// 与 ADR-049「事实是发生过什么」直接相关，不该顺手做。
    pub fn archive_before(&self, cut: Seq) -> std::io::Result<Option<usize>> {
        let mut inner = self.inner.write().expect("WalStore 锁中毒");
        if self.writer.is_none() {
            return Ok(None);
        }
        if !self.path.exists() {
            return Ok(None);
        }
        // 第一个「不该被归档」的事件。`seq` 缺失（旧版本行）按最大处理 ⇒ 不归档它。
        let cut = cut.value();
        let split = inner
            .events
            .iter()
            .position(|e| e.seq().map(Seq::value).unwrap_or(u64::MAX) >= cut)
            .unwrap_or(inner.events.len());
        if split == 0 {
            return Ok(None);
        }

        // 原文只读一次：前缀与尾部都从这同一份字节切出（不重序列化）。
        let raw = std::fs::read(&self.path)?;
        let mut lines: Vec<&[u8]> = raw.split(|b| *b == b'\n').collect();
        if lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop(); // 末行的换行符切出的空尾
        }
        // `replay` 遇到第一条坏行即停 ⇒ 前 `events.len()` 行**就是**那些事件。
        let good = &lines[..lines.len().min(inner.events.len())];

        // ── 1. 归档段 = 被取代的前缀（逐字节，原子出现）────────────────────
        let (first, last) = (
            inner.events[0].seq().map(Seq::value).unwrap_or(0),
            inner.events[split - 1].seq().map(Seq::value).unwrap_or(0),
        );
        let archive = archive_path(&self.path, first, last);
        if let Some(dir) = archive.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut tmp_name = archive.file_name().unwrap_or_default().to_os_string();
        tmp_name.push(".tmp");
        let archive_tmp = archive.with_file_name(tmp_name);
        {
            let mut f = std::fs::File::create(&archive_tmp)?;
            for line in &good[..split] {
                f.write_all(line)?;
                f.write_all(b"\n")?;
            }
            f.sync_all()?;
        }
        // 归档段先落稳，再动 live——顺序反了就会存在「live 已换、归档未稳」的窗口。
        std::fs::rename(&archive_tmp, &archive)?;

        // ── 2. 尾部逐行原样写出 ────────────────────────────────────────────
        // 逐行**搬运原文**而不是 `serde_json::to_vec` 重序列化：`payload` 是
        // `serde_json::Value`，重序列化的键序与原文不保证一致，而 WAL 是**盘上
        // 权威**——搬运应当逐字节保真，判据（`recovery_*`）也才有意义。
        let mut staged_name = self.path.file_name().unwrap_or_default().to_os_string();
        staged_name.push(".new");
        let staged = self.path.with_file_name(staged_name);
        {
            let mut f = std::fs::File::create(&staged)?;
            for line in &good[split..] {
                f.write_all(line)?;
                f.write_all(b"\n")?;
            }
            f.sync_all()?;
        }

        // ── 3. 原子替换 live ───────────────────────────────────────────────
        std::fs::rename(&staged, &self.path)?;

        // ── 4. 内存跟着走（`head` 不动——见方法文档）────────────────────────
        let kept = inner.events.split_off(split);
        inner.ids = kept.iter().map(|e| e.event_id().to_string()).collect();
        inner.events = kept;
        Ok(Some(split))
    }

    /// **完整事件史**：全部归档段（按号序）+ live 段。
    ///
    /// ## 为什么读侧需要一个「含归档」的口
    ///
    /// [`archive_before`](Self::archive_before) 把被取代的前缀移出 live 段，于是
    /// [`range`](Store::range) 看到的 live 段只是全局号空间的一个**窗口**——首号不是 0。
    /// 对**消息 / 上下文**类读者（转写 / 重建 / 请求包）这正是要的：快照**取代**了
    /// 那段历史，它们读 live 窗口 + 快照即可（`mutations_latest_snapshot` 会把
    /// 快照之前的事件整段跳过）。
    ///
    /// 但对**累计口径**的读者（不变量 C1 的「无跳号」/ SLO 四列 / 声誉 / 成本台账 /
    /// 记忆召回与巩固 / 任务状态）而言，被归档的事件**仍然发生过**——钱花过、声誉
    /// 挣过、记忆记过。只读 live 窗口会让它们**静默**少算（例如 C1 把窗口首号读成
    /// 「应为 0 却得 k」⇒ 每次压缩后读数口凭空多一条违规）。故这类读者走本方法。
    ///
    /// 两种读者读的是**同一份事实**的两个投影，不是两份真源：`history() ⊇ range()`，
    /// 差集恰是被取代的前缀（在归档段里，逐条不重不漏）。
    ///
    /// ## 归档段的读法
    ///
    /// 段名 `<wal 文件名>.<first>-<last>`（见 [`archive_path`]）：`first` 既是段内
    /// 首号、也是排序键。逐段 `replay` 后**按段名首号升序拼接**，再把 live 段接上
    /// ——不重排、不排序。归档段的号恒**严格小于** live 的号（`archive_before` 只搬
    /// `seq < cut` 的前缀），故拼接结果对正常事件就是 `seq` 升序；而对旧版本落下的
    /// `seq: null` 事件，保持它与 live 的**物理相对位置**（`range` 的口径）——
    /// 若按 `seq` 排序，`None` 会被挤到末尾，把「退化到切片末尾」这类兜底判据搞反。
    /// 目录不存在 / 某段读不开 ⇒ 跳过（与 `range` 的「读不开就是没有」同一口径）。
    pub fn history(&self) -> Vec<E> {
        let mut all: Vec<E> = Vec::new();
        // ── 归档段（按段名首号升序读）──────────────────────────────────────
        if let Some(dir) = self.path.parent().map(|p| p.join(WAL_ARCHIVE_SUBDIR)) {
            let mut prefix = self.path.file_name().unwrap_or_default().to_os_string();
            prefix.push(".");
            let prefix = prefix.to_string_lossy().to_string();
            if let Ok(entries) = std::fs::read_dir(&dir) {
                let mut segments: Vec<(u64, PathBuf)> = Vec::new();
                for ent in entries.flatten() {
                    let name = ent.file_name().to_string_lossy().to_string();
                    let Some(rest) = name.strip_prefix(&prefix) else {
                        continue;
                    };
                    let Some((first, _last)) = rest.split_once('-') else {
                        continue;
                    };
                    let Ok(first) = first.parse::<u64>() else {
                        continue;
                    };
                    segments.push((first, ent.path()));
                }
                segments.sort_by_key(|(first, _)| *first);
                for (_, path) in segments {
                    if let Ok((events, _)) = Self::replay(&path) {
                        all.extend(events);
                    }
                }
            }
        }
        // ── live 段（原序接上）────────────────────────────────────────────
        all.extend(
            self.inner
                .read()
                .expect("WalStore 锁中毒")
                .events
                .iter()
                .cloned(),
        );
        all
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
        // 与 `assign_seq` 同点收口（且在序列化**之前**——盘上落的是兜底后的形态）：
        // 没带时刻的事件以落格时刻为准，事实源里不出现 `ts = 0` 的事件
        // （`d781a6c7` 号会话 548 个事件全 ts=0 的教训，见 trait 方法文档）。
        event.fallback_timestamp(crate::symbio_core::clock_now_ms());
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

// ── panic 面登记（PN-001…003）─────────────────────────────────────────
// 本文件每一处 `unwrap` / `expect` / `panic!` / `unreachable!` 的理由。登记放在
// 文件内而不是集中一张表：理由与它解释的那段代码会一起被 review、一起被删。
// 判据见 `scripts/panic-audit.mjs`。**加一处 panic 必须同时加一行登记，理由非空。**
// panic-allow symbio/src/symbio_core/store/wal.rs::len: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/symbio_core/store/wal.rs::range: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/symbio_core/store/wal.rs::head: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/symbio_core/store/wal.rs::append: 存储层灾难不可静默：WAL 打不开 / 写不进时继续跑等于事实源与读侧悄悄分叉。序列化自有类型失败同理。
// panic-allow symbio/src/symbio_core/store/wal.rs::archive_before: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/symbio_core/store/wal.rs::history: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
