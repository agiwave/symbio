//! `store = wal` —— 会话的持久化事实源
//!
//! ## 为什么实现在这里，而端口在 core
//!
//! `Store` 是机制层的**端口**（append / range / head，F1
//! 冻结），它只规定「一个事实源要守的五条语义」，不认识 JSONL、不认识 `.lock`、
//! 不认识 `transcripts/wal-archive`。那些是**一种磁盘布局**的细节，而只有会话需要
//! 这种布局 ⇒ 细节住在使用方。分层判据收成编译边界：core 不再导出本类型，
//! 任何别的插件想拿 WAL 去自己开一个事实源，**写不出来**
//! （[D-083](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
//!
//! `Store` trait 冻结之后换实现**零接口改动**：调用方把
//! `EventStore` 换成 `EventWalStore`，其余代码一行不动
//! （[DE-018 §4](../../../../docs/003-design/018-模块架构.md "DE-018-模块架构")：实现替换不构成架构改动）。
//!
//! ## 形态
//!
//! JSONL append-only 文件：每次 append = 一行 JSON + flush。恢复 = 重放文件；
//! **投影是纯函数，恢复只需要重放，不需要状态迁移**（DE-035 §2 的关键回报）。
//!
//! ## 单写者：跨进程写者令牌
//!
//! 「每 thread 一个 FIFO，串行」（`Store` 五条语义之首）由
//! **进程内** `RwLock` 加**跨进程**写者令牌一起保证：两个进程各持一份 `WalStore`，各自
//! `head` 从文件重放算出，`append` 只 append 不校验 ⇒ seq 重号 / 事件交错，
//! 且**没有任何错误信号**。
//!
//! 本实现把它升级为**跨进程**的写者令牌：`open` 对一个**旁挂锁文件**
//! （`<wal>.lock`）取一次非阻塞独占锁（`File::try_lock`：
//! Unix `flock` / Windows `LockFileEx`，随句柄关闭自动释放）。
//!
//! | 入口 | 令牌 | `append` |
//! |---|---|---|
//! | `open` 拿到锁 | 持 | 正常落盘 |
//! | `open` 拿不到锁 | 不持 | `AppendError::NotTheWriter` |
//! | `open_readonly` | 从不取锁 | 同上（读方在类型上就不该写） |
//!
//! 拿不到令牌**不报错、降级成「非写者」**：读照常（`range` / `head` 不看令牌），
//! 只有写被拒。这样「第二个写者」拿到的是一个可判别的信号，而不是一个静默的
//! 重复 seq——反向用例要钉的正是这个失效。
//!
//! ### 锁为什么落在旁挂文件、而不是 WAL 文件自身
//!
//! 这是 Windows 逼出来的（实测，非偏好）：`LockFileEx` 是**强制锁**——它不只拦
//! 别的写者，也拦**别的句柄的读**。把锁打在 WAL 上，会话进行中的读数口
//! （[`stats`](../stats.rs) 会在任意时刻 `open_readonly`
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
//! 一部分**，不是错误。反向用例（DE-035 §6.4）证明该边界真实存在。
//!
//! 截断是**写方**的语义，因此只有持令牌者才做（`open_readonly`
//! 从不截断，拿不到令牌的降级写者也不截断）。

use std::collections::HashSet;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use crate::paths;
use symbio_core::{
    closes_turn, is_turn_final, is_turn_open, provenance_violation, AppendError, Event, EventDecl,
    EventEnvelope as _, Frozen, Seq, Store, StoreInvariant,
};

/// 判据的**词汇面**：认识某个名字的那张表说了算（DE-024 D-203）。
///
/// 三份表按「谁先认识这个名字」排：本实例挂的装配表 → 进程装配表（插件经
/// `submit_declarations!` 交来的事件行）→ **跨插件共享表**（各插件清单 `shared_events`
/// 的生成物，DE-024 D-204）。顺序不能反：一个新插件声明了同名事件时，冻结表才是
/// owner 的陈述；共享表答的是「这条名字在别人的二进制里也叫这个」。格位与角色
/// **取自同一张表**——不出现「格位在冻结表、角色在另一张表」的两侧各读一半。
///
/// ## 谁都不认识 = 网格之外
///
/// 名字与格位只由 owner 声明：三份表都不认识的名字，闸门答 `OffGrid`（fail-closed）。
/// 「谁有资格声明一个名字」的答案只有 owner，内核不替任何人记名字。
struct Vocab<'a> {
    instance: Option<&'a [EventDecl]>,
    process: Option<&'static [EventDecl]>,
}

impl<'a> Vocab<'a> {
    fn of(declarations: Option<&'a Frozen>) -> Self {
        Vocab {
            instance: declarations.map(Frozen::events),
            process: symbio_plugin_api::declared_vocab(),
        }
    }

    /// 第一个认识 `kind` 的表；三份都不认识 ⇒ `None`（网格之外）。
    fn table_for(&self, kind: &str) -> Option<&'a [EventDecl]> {
        let known = |table: &[EventDecl]| table.iter().any(|e| e.name == kind);
        if let Some(table) = self.instance.filter(|t| known(t)) {
            return Some(table);
        }
        if let Some(table) = self.process.filter(|t| known(t)) {
            return Some(table);
        }
        // 跨插件共享表：owner 清单 `shared_events` 的生成物（DE-024 D-204）。它不是
        // 降级——格位来自 owner 自己的 `EventDecl`，只是这个二进制没链那个 owner 插件
        // （E-009 禁那条边），所以经生成物答题。
        let shared: &'a [EventDecl] = symbio_plugin_api::events::SHARED_EVENTS;
        if known(shared) {
            return Some(shared);
        }
        None
    }

    /// 这个名字落在哪一格（与迁移前的三级 `.or_else` 同序、同结论）。
    fn cell(&self, kind: &str) -> Option<(symbio_core::Entity, symbio_core::Verb)> {
        self.table_for(kind)
            .and_then(|table| table.iter().find(|e| e.name == kind))
            .map(|e| e.cell)
    }

    /// 定稿发言（I1「每轮至多一条 final」的口径）。
    fn is_final(&self, e: &Event) -> bool {
        match self.table_for(&e.kind) {
            Some(table) => is_turn_final(table, e),
            None => false,
        }
    }
}

/// 持久化 Store（`store = wal`，[DE-001 §8](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构") 参数表取值）。
///
/// **信封是具体的 `Event`**：写侧闸门要读 `entity` / `verb` / `turn` / `produced_by`
/// 才能判 I1 / I2，而泛型参数上只有
/// `EventEnvelope` 的三件事可用。仓内唯一的
/// 实例化是 `WalStore` 装 `Event`；把信封写实，闸门才能挂在唯一写入口**内部**——
/// 闸门不是一个可选装饰。
pub struct WalStore {
    path: PathBuf,
    /// 写者令牌：`Some` = 本实例持有该事实源的跨进程独占锁，`None` = 非写者。
    ///
    /// 句柄指向**旁挂锁文件**（见模块头），不是 WAL 本身——WAL 永不被锁，
    /// 读方才能随时进。锁随句柄关闭自动释放，故不需要显式解锁。
    writer: Option<File>,
    inner: RwLock<WalInner>,
    /// 归档段的并集（按段名首号升序），`None` = 本实例还没读过。
    ///
    /// `range` 的口径含归档前缀，而归档段一旦 `rename` 就位就不再变
    /// （只增不改；同名重写是同一份内容的幂等重复）⇒ 读一次就可以缓存，
    /// 本实例 `archive_before` 之后失效重读。**跨实例的时效性与 live 段同一口径**：
    /// 一个实例看到的是它 open 时重放到的世界（别人的 `append` 不会改变这里的 live），
    /// 别人的归档因此同样不出现在这里——这不是新增的近似，是同一个近似。
    archived: RwLock<Option<Vec<Event>>>,
    /// 装配期冻结表（DE-024 D-198 / D-199）：写闸先问它，未命中再问进程装配表与
    /// 共享表。
    ///
    /// 注入点有两个：生产路径在 [`crate::store::Store::facts`] 打开事实源后挂上
    /// `symbio_plugin_api::sealed_declarations()`；用例可以用
    /// [`WalStore::with_declarations`] 挂一张小表。`None` 不表示「没有网格」——
    /// 写闸仍会问全局装配表；三份表都不认识 ⇒ 网格之外（fail-closed）。
    declarations: Option<Arc<Frozen>>,
}

struct WalInner {
    events: Vec<Event>,
    ids: HashSet<String>,
    head: u64,
    /// 已开轮的 `turn`（[`is_turn_open`] 口径）——I1 的「每轮至多一条开轮」索引。
    ///
    /// 存档段移走的是**旧轮**，而旧轮不会再被重开；归档因此**不清空**本索引
    /// （清空会让一张老会话在压缩之后忘记自己开过哪些轮）。
    opened: HashSet<u64>,
    /// 已收束的 `turn`（[`closes_turn`] 口径）——I3 的「旧轮收没收」用这一份。
    ///
    /// 注意它**不是** I1 去重的索引：产物收束（`artifact.added`）也算「这一轮有话说过」，
    /// 但同一轮随后仍应有且只有一条 `final`。
    closed: HashSet<u64>,
    /// 已出现 `final` 的 `turn`（[`is_turn_final`] 口径）——I1 的「每轮至多一条 final」索引。
    finals: HashSet<u64>,
    /// I3 的**显式降级**计数：新开轮越过了未收束的旧轮（崩溃残轮 / 真丢收束）。
    ///
    /// 不拒绝而是计数 + 出声，理由见 [`WalInner::gate`]；读侧 `check_all` 会把同一条
    /// 缺口报成 `unresolved_turns` 违规，两处口径一致。
    degraded_i3: u64,
}

impl WalInner {
    /// 把一条事件记进两份索引（重放与入库共用）。
    fn note(&mut self, vocab: &Vocab, event: &Event) {
        if is_turn_open(event) {
            self.opened.insert(event.turn);
        }
        if closes_turn(event) {
            self.closed.insert(event.turn);
        }
        if vocab.is_final(event) {
            self.finals.insert(event.turn);
        }
    }

    /// 把一段事件（重放结果 / 归档段）整段记进索引。幂等。
    fn note_all<'e>(&mut self, vocab: &Vocab, events: impl IntoIterator<Item = &'e Event>) {
        for e in events {
            self.note(vocab, e);
        }
    }

    /// 比 `turn` 更早、却还没收束的轮里最小的那个（I3 的判据）。
    fn unresolved_before(&self, turn: u64) -> Option<u64> {
        self.opened
            .iter()
            .filter(|t| **t < turn && !self.closed.contains(t))
            .min()
            .copied()
    }

    /// 写侧闸门（+
    /// 段四）：
    /// **唯一写入口内**的拒绝判定。
    ///
    /// 返回 `Err` = 这条事件不得入库；`Ok` = 允许（可能的「显式降级」已在内部出声）。
    ///
    /// ## 拒 / 降级的分界
    ///
    /// 分界不是「哪条更严重」，而是**拒绝会不会把会话钉死**：
    ///
    /// | 判据 | 处置 | 理由 |
    /// |---|---|---|
    /// | 名字与格位对不上（不在表上 / 自述挪了一格） | **拒绝** | 投影按格子取事实，错位是**静默少一条事实**；名字与坐标必须同源（段四） |
    /// | I2 声明缺 `produced_by` | **拒绝** | 没有上游的声明无法审计也无法撤销；生产者补上即可，重试是安全的 |
    /// | I1 同轮第二条 `final` | **拒绝** | 这一轮已经有收束，第二条会让「每轮至多一条 final」失效 |
    /// | I1 同轮第二条开轮（不同 id） | **拒绝** | 同 id 的重放更早就会被 `Duplicate` 拦下，能走到这里是两条不同事件抢同一轮 |
    /// | I3 新轮越过未收束的旧轮 | **降级**（出声 + 计数） | 崩溃恢复**恰好长这样**：上一轮停在半路，重启后开下一轮。拒绝会让会话再也写不进任何事实 |
    ///
    /// I3 的降级不是「静默放过」：它在写入口当场 `plugin_warn!` 出声，读侧
    /// `check_all` 的 `unresolved_turns` 会把同一处
    /// 缺口报成违规——两条信号指向同一个事实，不存在「日志里有、清单里没有」。
    fn gate(&mut self, event: &Event, vocab: &Vocab) -> Result<(), AppendError> {
        // **先判名字与格位是否同源，再判事实之间的关系**：I1 / I2 / I3 的判据全部按
        // `(entity, verb, kind)` 取事实（`is_turn_open` 认的就是这一格）。坐标本身错位时，
        // 后面每条判定的输入都是错位的——所以这一臂排在最前，且它不需要任何索引。
        // 三份词汇表的取舍顺序见 [`Vocab`]（DE-024 D-199 / D-203）。
        let given = (event.entity, event.verb);
        let declared = vocab.cell(&event.kind);
        if declared != Some(given) {
            return Err(AppendError::OffGrid {
                event_id: event.event_id.clone(),
                kind: event.kind.clone(),
                declared,
                given,
            });
        }
        if let Some(v) = provenance_violation(event) {
            return Err(AppendError::Invariant {
                invariant: StoreInvariant::I2,
                event_id: v.event_id,
                why: v.why,
            });
        }
        if vocab.is_final(event) && self.finals.contains(&event.turn) {
            return Err(AppendError::Invariant {
                invariant: StoreInvariant::I1,
                event_id: event.event_id.clone(),
                why: format!(
                    "turn {} 已有收束格，又出现一条 final——每轮至多 1 条（I1 单通道）",
                    event.turn
                ),
            });
        }
        if is_turn_open(event) {
            if self.opened.contains(&event.turn) {
                return Err(AppendError::Invariant {
                    invariant: StoreInvariant::I1,
                    event_id: event.event_id.clone(),
                    why: format!(
                        "turn {} 已有开轮事件，又出现一条（I1 单通道：每轮至多 1 条开轮）",
                        event.turn
                    ),
                });
            }
            if let Some(missing) = self.unresolved_before(event.turn) {
                self.degraded_i3 += 1;
                symbio_plugin_api::plugin_warn!(
                    "session",
                    "[invariant-I3] 新开轮 turn {} 越过了未收束的 turn {}（本轮照常入格；\
                     读侧 session/stats 的 unresolved_turns 会报同一条）",
                    event.turn,
                    missing
                );
            }
        }
        Ok(())
    }
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

/// 归档段的存放目录（相对 WAL 所在目录）：`<会话目录>/transcripts/wal-archive`。
///
/// `transcripts` 这一段取自 [`paths::TRANSCRIPTS_SUBDIR`]——那是会话布局的公共词汇，
/// 只定义一次；`wal-archive` 这一段是本模块自己的布局，读侧（`range`）与写侧
/// （`archive_before`）共用同一个拼法。
///
/// 放在 `transcripts/` 之下而不是与会话文件同级：VDFS 会话树是**显式构造**的
/// （不走 `read_dir`），故这个子目录不会漏进前端的文件列表。
fn archive_dir_of(wal_dir: &Path) -> PathBuf {
    wal_dir.join(paths::TRANSCRIPTS_SUBDIR).join("wal-archive")
}

/// 归档段的路径：`<wal 所在目录>/transcripts/wal-archive/<wal 文件名>.<first>-<last>`。
///
/// 段名带上首末号：①「这段覆盖哪一号区间」在文件名上直接可读；② 同一次归档
/// 重复执行会落到同一个名字（幂等，`File::create` 覆盖），不同次归档天然不同名，
/// 不会互相覆盖。
fn archive_path(wal: &Path, first: u64, last: u64) -> PathBuf {
    let mut name = wal.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{first}-{last}"));
    archive_dir_of(wal.parent().unwrap_or_else(|| Path::new("."))).join(name)
}

impl WalStore {
    /// 打开（或创建）一个 WAL 文件，并**尝试取得写者令牌**。
    ///
    /// 拿不到令牌（另一个写者持有）⇒ 返回一个可读、但 `append` 恒
    /// `NotTheWriter` 的实例——降级不是失败，
    /// 是「这次你不是写者」这个事实本身。
    ///
    /// **恢复在这里发生**：持令牌者重放所有完整行，撕裂的尾行被丢弃并截断
    /// （提交边界，DE-035 §6.1）；非写者只重放，不动文件。
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

    /// 事实源所在目录（= 会话目录）。
    ///
    /// 只给 session 域内用：寄存区要落在**它等着落进的那条 WAL 旁边**，路径由句柄
    /// 自己给出，调用方不必重新派生一遍布局（重新派生 = 第二份布局知识，
    /// [DE-014 §2.2 推论 1](../../../../docs/003-design/014-单环一表.md "DE-014-单环一表")
    /// 要消掉的正是这个）。
    pub(crate) fn dir(&self) -> &Path {
        self.path.parent().unwrap_or_else(|| Path::new("."))
    }

    /// **只读**打开：只解析，不动文件、不取令牌。
    ///
    /// 与 `open` 的差别只在「谁有资格写」：恢复（截断撕裂尾行）
    /// 是**写方**的语义，读方不能顺手做——读数口在会话进行中随时可能被调用，
    /// 而写点恒在当前 EOF：读方若在写方落一行的中途截断了它，剩下的字节会接在
    /// 被截掉的位置之后 ⇒ 那一行静默损坏。解析器仍是同一个（`replay`），
    /// 撕裂尾行同样丢弃，只是**不改文件**。
    ///
    /// 文件不存在 ⇒ 空仓（「还没有事实源」是常态，读方不该为了读而创建文件）。
    pub fn open_readonly(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let (events, _) = Self::replay(&path)?;
        Ok(Self::build(path, events, None))
    }

    /// 重放到最后一行完整记录（撕裂尾行丢弃——**提交边界**，DE-035 §6.1）。
    ///
    /// 返回 `(事件, 最后一条完整记录的结束偏移)`；文件不存在 ⇒ 空。
    /// 不写文件：写不写由调用方决定（`open` 截断 /
    /// `open_readonly` 不动）。
    fn replay(path: &Path) -> std::io::Result<(Vec<Event>, u64)> {
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
            match serde_json::from_slice::<Event>(&bytes) {
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
    fn build(path: PathBuf, events: Vec<Event>, writer: Option<File>) -> Self {
        let ids = events.iter().map(|e| e.event_id.clone()).collect();
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
        // 闸门索引**从重放结果重建**：重启不是「闸门失忆」的理由——同一份 WAL
        // 重开后，同轮第二条 final 仍要被拒（见 `gate_index_is_rebuilt_from_the_wal_on_reopen`）。
        // 此刻还没挂实例表（`with_declarations` 在 `build` 之后），词汇面走进程装配表 →
        // 共享表这两份表；角色与格位在每份表内同源，重建口径与后续写闸因此一致。
        let vocab = Vocab::of(None::<&Frozen>);
        let opened = events
            .iter()
            .filter(|e| is_turn_open(e))
            .map(|e| e.turn)
            .collect();
        let closed = events
            .iter()
            .filter(|e| closes_turn(e))
            .map(|e| e.turn)
            .collect();
        let finals = events
            .iter()
            .filter(|e| vocab.is_final(e))
            .map(|e| e.turn)
            .collect();
        WalStore {
            path,
            writer,
            inner: RwLock::new(WalInner {
                events,
                ids,
                head,
                opened,
                closed,
                finals,
                degraded_i3: 0,
            }),
            archived: RwLock::new(None),
            declarations: None,
        }
    }

    /// 挂上装配期冻结表（DE-024 D-198）：写闸先问它，未命中再回退。
    ///
    /// 生产入口在 [`crate::store::Store::facts`]；用例用一张小表验证「写闸读的是
    /// 声明，不是内核常量」——本方法因此是 `pub`（`EventWalStore` 本身仍只
    /// `pub(crate)` 可见）。
    pub fn with_declarations(mut self, frozen: Arc<Frozen>) -> Self {
        self.declarations = Some(frozen);
        self
    }

    /// **段归档**：把 `seq < cut` 的事件移入归档段，live 文件从 `cut` 起。
    ///
    /// 由 `EVENT_SESSION_CONSOLIDATED`
    /// 的写方调用（压缩成功之后）：快照**取代**了它之前的一切，那些事件对 live 读侧
    /// 不再有意义（读侧要的是「当前有效的历史」），而它们仍然**不可删**（审计 / 回溯）。
    /// 于是「上下文压缩」与「WAL 无界增长」由同一个机制解决（D-053 决策 6）。
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
    /// live」恰好等于完整事件史，**逐条不重不漏**——这是 D-053「零回退损失」
    /// 的可执行依据：回退时把各段与 live 依次拼接即得原 WAL。
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
    /// 归档段是「回退零损失」的依据（D-053）：它们不参与常规读，
    /// 但必须还在。删除策略要单独定——那是在决定「哪些事实可以不再可回溯」，
    /// 与 D-049「事实是发生过什么」直接相关，不该顺手做。
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
        // 归档段并集的缓存作废：本实例刚往归档目录里放了一段新的。
        *self.archived.write().expect("WalStore 锁中毒") = None;
        Ok(Some(split))
    }

    /// **归档段的并集**（全部归档段按段名首号升序拼接，**不含 live**）。
    ///
    /// 这是 `range` 的另一半：`range` 的口径是「从 `from` 起的全部
    /// 事实，含被 `archive_before` 移出 live 的前缀」，而那段
    /// 前缀不在内存里。归档段一旦 `rename` 就位就不再变（只增不改；同名重写是同一份
    /// 内容的幂等重复），故**读一次缓存一次**，本实例归档之后失效重读——每次读都重扫
    /// 目录不是「读法归一」的成本，而是把一次目录扫描搬进了热路径（每轮 prompt
    /// 构建都要过一遍）。
    ///
    /// ## 读法
    ///
    /// 段名 `<wal 文件名>.<first>-<last>`（见 [`archive_path`]）：`first` 既是段内
    /// 首号、也是排序键。逐段 `replay` 后**按段名首号升序拼接**，live 段由 `range`
    /// 接在后面——不重排、不排序。归档段的号恒**严格小于** live 的号（`archive_before`
    /// 只搬 `seq < cut` 的前缀），故拼接结果对正常事件就是 `seq` 升序；而对旧版本落下的
    /// `seq: null` 事件，保持它与 live 的**物理相对位置**——若按 `seq` 排序，`None` 会被
    /// 挤到末尾，把「退化到切片末尾」这类兜底判据搞反。
    /// 目录不存在 / 某段读不开 ⇒ 跳过（与「读不开就是没有」同一口径）。
    fn archive_events(&self) -> Vec<Event> {
        if let Some(cached) = self.archived.read().expect("WalStore 锁中毒").as_ref() {
            return cached.clone();
        }
        let mut all: Vec<Event> = Vec::new();
        // ── 归档段（按段名首号升序读）──────────────────────────────────────
        if let Some(parent) = self.path.parent() {
            let dir = archive_dir_of(parent);
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
        *self.archived.write().expect("WalStore 锁中毒") = Some(all.clone());
        // 归档段也是历史：索引必须认它（否则压缩之后「这个轮开过没有」会失忆）。
        // 归档**只增不改**，吸收是幂等的；且本函数不持 `inner` 锁，不会自锁。
        let vocab = Vocab::of(self.declarations.as_deref());
        self.inner
            .write()
            .expect("WalStore 锁中毒")
            .note_all(&vocab, &all);
        all
    }

    /// 已入库事件数（== `head().value()`；测试与断言用）。
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner.read().expect("WalStore 锁中毒").events.len()
    }

    /// I3 显式降级的次数（测试与断言用；生产侧由 `plugin_warn!` 与读侧 `check_all` 出声）。
    #[cfg(test)]
    pub fn degraded_i3(&self) -> u64 {
        self.inner.read().expect("WalStore 锁中毒").degraded_i3
    }
}

impl Store for WalStore {
    type Event = symbio_core::Event;

    fn append(&self, mut event: Self::Event) -> Result<Seq, AppendError> {
        let vocab = Vocab::of(self.declarations.as_deref());
        let mut inner = self.inner.write().expect("WalStore 锁中毒");
        // 单写者闸门（跨进程）：非写者一律拒写。这是 I1 在并发下的硬边界——
        // 在此之前这里只有进程内 RwLock，第二个进程会静默地写出重复 seq。
        if self.writer.is_none() {
            return Err(AppendError::NotTheWriter);
        }
        if inner.ids.contains(event.event_id()) {
            return Err(AppendError::Duplicate);
        }
        // **写侧不变量闸门**（I1 / I2 / I3）：挂在唯一写入口内，三条真实写路径
        // （用户输入 / 工具结果 / 收尾落格）没有一条能绕过它。位置在幂等去重之后
        // （同 id 重放仍按 `Duplicate` 回答，不被闸门改判）、赋号与落盘之前
        // （被拒的事件不会消耗 seq，也不会在盘上留下半条）。
        inner.gate(&event, &vocab)?;
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
        event.fallback_timestamp(symbio_core::clock_now_ms());
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
        inner.note(&vocab, &event);
        inner.events.push(event);
        inner.head += 1;
        Ok(seq)
    }

    fn range(&self, from: Seq) -> Vec<Self::Event> {
        // **一个读口，两个来源**：归档段（被取代的前缀）+ live 段。号空间全局单调、
        // 归档段恒在前 ⇒ 拼接即 `seq` 升序（缺号事件保持物理位置，见 `archive_events`）。
        //
        // **`from` 是号，不是下标**：`archive_before` 之后 live 段的首号不再是 0，
        // 按下标切会把「从 seq k 起」读成「从 live 第 k 条起」⇒ 少读甚至读空，而空集
        // 看起来像「没有这条事实」。
        //
        // 缺号的事件（旧版本落下的行）**没有号**，因而不属于任何 `[from, head)` 区间：
        // 只在读全史（`from = 0`）时出现。它与 `archive_before` 的「缺号按最大处理
        // ⇒ 不归档它」是同一取向的两半。
        let from = from.value();
        let mut all = self.archive_events();
        all.extend(
            self.inner
                .read()
                .expect("WalStore 锁中毒")
                .events
                .iter()
                .cloned(),
        );
        if from == 0 {
            return all;
        }
        all.into_iter()
            .filter(|e| e.seq().is_some_and(|s| s.value() >= from))
            .collect()
    }

    fn head(&self) -> Seq {
        Seq::new(self.inner.read().expect("WalStore 锁中毒").head)
    }
}

/// 便捷别名：持久化事实源（信封恒为 [`symbio_core::event::Event`]）。
pub type EventWalStore = WalStore;

#[cfg(test)]
#[path = "wal.test.rs"]
mod tests;

// ── panic 面登记（PN-001…003）─────────────────────────────────────────
// 本文件每一处 `unwrap` / `expect` / `panic!` / `unreachable!` 的理由。登记放在
// 文件内而不是集中一张表：理由与它解释的那段代码会一起被 review、一起被删。
// 判据见 `scripts/panic-audit.mjs`。**加一处 panic 必须同时加一行登记，理由非空。**
// panic-allow crates/symbio-plugin-session/src/store/wal.rs::range: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow crates/symbio-plugin-session/src/store/wal.rs::head: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow crates/symbio-plugin-session/src/store/wal.rs::append: 存储层灾难不可静默：WAL 打不开 / 写不进时继续跑等于事实源与读侧悄悄分叉。序列化自有类型失败同理。
// panic-allow crates/symbio-plugin-session/src/store/wal.rs::archive_before: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow crates/symbio-plugin-session/src/store/wal.rs::archive_events: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
