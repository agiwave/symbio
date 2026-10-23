//! 事实源端口与它的**投影值**（会话存储的实现段之二）。
//!
//! 分在这里的判据是「这一段错了谁会红」：本文件出错，红的是**读到的事实**——
//! 句柄拿不到（写点当场报真失败）、运行态投影读到旧值（上一轮结局说错话）、
//! 召回漏掉会话（记忆视图少一块）。这与「目录长什么样」（`layout`：错了红的是
//! 会话读不出来 / 整份写坏 / 清单少项）和「条目怎么读写删」（`super`）是三批
//! 不同的观众；混在一处时，一次布局改动会顺手碰坏投影，反之亦然。
//!
//! 三件事的形状（[D-084](../../../../../docs/003-design/001-核心架构.md#d-084-能力模块公用基础协议上下文只递值事实只经引擎abcde-五类需要的唯一取得方式 "DE-001-核心架构")、
//! [D-083 §5.2](../../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）：
//!
//! - **句柄只发给引擎写点**，且进程内按会话构造一次（「一写者」，见
//!   `SessionStore::facts` 的文档）；
//! - **读方拿值**：`snapshot` / `runtime_facts` 交的是 `Vec<Event>` 与投影结构，
//!   不交路径、不交句柄；
//! - **跨会话的读要预算**：`recall_snapshots` 的名额与字节上限是本文件末尾那几个常量。

use super::*;

/// 运行态投影缓存项：值与它对应的**事实版本戳**。
#[derive(Debug, Clone)]
pub(super) struct RuntimeFactsCache {
    stamp: FactsStamp,
    facts: RuntimeFacts,
}

/// 事实版本戳——两种驻留方式各一个判据，互不比较（变体不同即失效）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FactsStamp {
    /// 本进程持有写句柄：`head`（下一个待分配序号）即版本。
    Writer(u64),
    /// 只读打开：文件长度 + mtime（追加必然改长度）。
    File {
        len: u64,
        modified: Option<SystemTime>,
    },
}

impl SessionStore {
    /// 该会话的**唯一事实源句柄**（持久会话）；临时会话 / 目录不存在 ⇒ `None`。
    ///
    /// 进程内按会话构造一次并缓存：所有引擎写方共享同一实例，`head` 只有一份、
    /// 内部写锁也只有一把。第二个 `WalStore::open` 不存在——这正是
    /// [plan/14 §2.2 推论 3](../../../../../docs/003-design/014-单环一表.md "DE-014-单环一表")
    /// 的「一写者」。跨进程写者令牌仍由 `WalStore` 自己守（旁挂锁文件）。
    ///
    /// **只发给引擎写点**：能力面拿到的是事实的值（读）或待落的值（写），
    /// 不是这个句柄（[D-084](../../../../../docs/003-design/001-核心架构.md#d-084-能力模块公用基础协议上下文只递值事实只经引擎abcde-五类需要的唯一取得方式 "DE-001-核心架构") §1）。
    ///
    /// `Ok(None)` = 没有事实源（临时会话 / 目录不存在）；`Err` = 目录在、事实源
    /// **打不开**——后者是 的真失败，调用方不得当成「没有事实源」静默跳过。
    pub(in crate::plugins::session) fn facts(
        &self,
        session_id: &str,
    ) -> Result<Option<Arc<EventWalStore>>, String> {
        let mut cache = self.facts.lock().expect("会话事实源表锁中毒");
        if let Some(store) = cache.get(session_id) {
            return Ok(Some(store.clone()));
        }
        let Some(dir) = self.session_dir(session_id) else {
            return Ok(None);
        };
        let wal = dir.join(paths::V2_WAL_FILE);
        // 持锁 open：两个并发 `facts` 若各自 open，后插入者可能是不持令牌的
        // 降级实例，把先前的写者顶掉（跨进程锁是进程级独占，不是句柄级）。
        let store = Arc::new(
            EventWalStore::open(&wal)
                .map_err(|e| format!("事实源打不开（{}）：{e}", wal.display()))?,
        );
        cache.insert(session_id.to_string(), store.clone());
        Ok(Some(store))
    }

    /// 该会话的**完整史快照**（含归档前缀）；没有事实源 ⇒ `None`。
    ///
    /// 这是读方的唯一形态：值是 `Vec<Event>`，不是路径、不是句柄。已有写句柄时
    /// 走它（就是本进程刚落的那些格），否则只读打开一次——读方不创建文件、
    /// 不取令牌、不截尾。
    pub(in crate::plugins::session) fn snapshot(&self, session_id: &str) -> Option<Vec<Event>> {
        if let Some(store) = self
            .facts
            .lock()
            .expect("会话事实源表锁中毒")
            .get(session_id)
            .cloned()
        {
            return Some(store.range(Seq::new(0)));
        }
        let dir = self.session_dir(session_id)?;
        let wal = dir.join(paths::V2_WAL_FILE);
        if !wal.is_file() {
            return None;
        }
        let store = EventWalStore::open_readonly(&wal).ok()?;
        Some(store.range(Seq::new(0)))
    }

    /// 该会话的**运行态事实投影**（上一轮结局）；临时会话 / 没有 WAL ⇒ `None`。
    ///
    /// [D-083 §5.2](../../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")：结局在读侧是投影值，
    /// 不是第二份状态。这里只交**值**，不交句柄、不交路径；投影函数是纯的
    /// （`runtime::project`），缓存只是「事实没变就不重折」。
    ///
    /// 缓存失效判据按驻留方式分两种：
    /// - 本进程有写句柄：`head`（下一个待分配序号）——任何追加都使它变化；
    /// - 只读：WAL 长度 + mtime——别的进程追加必然改动文件。
    ///
    /// 归档不改变 `head` 也不改变完整史（归档段 + live 段仍拼成同一份事实），
    /// 缓存因此不会因归档而失效，也不需要失效。
    pub(in crate::plugins::session) fn runtime_facts(
        &self,
        session_id: &str,
    ) -> Option<RuntimeFacts> {
        let writer = self
            .facts
            .lock()
            .expect("会话事实源表锁中毒")
            .get(session_id)
            .cloned();
        let cached = self.runtime_cache().get(session_id).cloned();

        if let Some(store) = writer {
            let stamp = FactsStamp::Writer(store.head().value());
            if let Some(hit) = cached.as_ref().filter(|c| c.stamp == stamp) {
                return Some(hit.facts.clone());
            }
            let facts = runtime::project(&store.range(Seq::new(0)));
            self.remember_runtime(session_id, stamp, &facts);
            return Some(facts);
        }

        let dir = self.session_dir(session_id)?;
        let wal = dir.join(paths::V2_WAL_FILE);
        let md = std::fs::metadata(&wal).ok()?;
        if !md.is_file() {
            return None;
        }
        let stamp = FactsStamp::File {
            len: md.len(),
            modified: md.modified().ok(),
        };
        if let Some(hit) = cached.as_ref().filter(|c| c.stamp == stamp) {
            return Some(hit.facts.clone());
        }
        let store = EventWalStore::open_readonly(&wal).ok()?;
        let facts = runtime::project(&store.range(Seq::new(0)));
        self.remember_runtime(session_id, stamp, &facts);
        Some(facts)
    }

    /// 运行态投影缓存表的唯一取锁口（std 锁当无中毒用，同本文件其它表）。
    pub(super) fn runtime_cache(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<String, RuntimeFactsCache>> {
        self.runtime_cache.lock().expect("会话运行态投影缓存锁中毒")
    }

    fn remember_runtime(&self, session_id: &str, stamp: FactsStamp, facts: &RuntimeFacts) {
        self.runtime_cache().insert(
            session_id.to_string(),
            RuntimeFactsCache {
                stamp,
                facts: facts.clone(),
            },
        );
    }

    /// 顶级会话事实源的召回视图（**新近优先**、名额与字节双预算）。
    ///
    /// 召回只扫平级会话（不含嵌套子会话）——范围裁决在
    /// `memory_events::recall_view` 的文档里；
    /// 本函数只负责「枚举 + 按预算开档 + 交值」，可见域过滤仍在读方。
    ///
    /// `own` = 这份事实源是否调用方自己的会话（按目录名比，同一个根下的两层
    /// 派生可能不同形）——`thread_private` 的收放靠它。
    pub(in crate::plugins::session) fn recall_snapshots(
        &self,
        self_id: &str,
    ) -> Vec<FleetSnapshot> {
        let Some(base) = self.disk() else {
            return Vec::new();
        };
        let own_name = paths::safe_id(self_id);
        let mut files: Vec<(std::time::SystemTime, PathBuf, u64)> = Vec::new();
        if let Ok(dir) = std::fs::read_dir(base) {
            for entry in dir.flatten() {
                let dir = entry.path();
                let wal = dir.join(paths::V2_WAL_FILE);
                let Ok(md) = std::fs::metadata(&wal) else {
                    continue;
                };
                if !md.is_file() || md.len() == 0 {
                    continue;
                }
                if md.len() > RECALL_MAX_FILE_BYTES {
                    crate::plugin_warn!(
                        "session",
                        "[memory] 召回跳过超大事实源（{} B > 上限 {} B）：{}",
                        md.len(),
                        RECALL_MAX_FILE_BYTES,
                        wal.display()
                    );
                    continue;
                }
                let mtime = md.modified().unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                files.push((mtime, wal, md.len()));
            }
        }
        files.sort_by(|a, b| b.0.cmp(&a.0));

        let mut out = Vec::new();
        let mut bytes = 0u64;
        for (i, (_, wal, len)) in files.into_iter().enumerate() {
            if i >= RECALL_MAX_FILES {
                break;
            }
            if bytes > 0 && bytes + len > RECALL_MAX_TOTAL_BYTES {
                break;
            }
            let own = wal
                .parent()
                .and_then(|d| d.file_name())
                .is_some_and(|n| n.to_string_lossy() == own_name);
            match EventWalStore::open_readonly(&wal) {
                Ok(store) => out.push(FleetSnapshot {
                    own,
                    events: store.range(Seq::new(0)),
                }),
                Err(e) => crate::plugin_warn!(
                    "session",
                    "[memory] 召回读取失败（{}）：{e}",
                    wal.display()
                ),
            }
            bytes += len;
        }
        out
    }

    /// 部署级全量快照（含嵌套子会话，任意深度）。
    ///
    /// 统计聚合与淘汰覆盖层共用：两者都要求「所有会话」完整，单个事实源打不开
    /// ⇒ 跳过并出声，不因一个坏会话让整列消失。
    pub(in crate::plugins::session) fn deep_snapshots(&self) -> Vec<Vec<Event>> {
        let Some(base) = self.disk() else {
            return Vec::new();
        };
        paths::session_wals_deep(base)
            .into_iter()
            .filter_map(|wal| match EventWalStore::open_readonly(&wal) {
                Ok(store) => Some(store.range(Seq::new(0))),
                Err(e) => {
                    crate::plugin_warn!(
                        "session",
                        "[facts] 部署级扫描跳过打不开的事实源（{}）：{e}",
                        wal.display()
                    );
                    None
                }
            })
            .collect()
    }
}

/// 召回扫描预算：最多看几个会话的事实源 / 单文件字节上限 / 总字节上限。
///
/// 超限 ⇒ 不扫并 `plugin_warn` 记一笔——看不见的文件不假装看得见。
/// 预算属于**读成本策略**，枚举属于存储布局，故两者都住在存储层。
pub(in crate::plugins::session) const RECALL_MAX_FILES: usize = 16;
pub(in crate::plugins::session) const RECALL_MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
pub(in crate::plugins::session) const RECALL_MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
// ── panic 面登记（PN-001…003）─────────────────────────────────────────
// 判据与「登记写在代码旁边」的理由见 `store/mod.rs` 末尾同名区块。
// panic-allow symbio/src/plugins/session/store/facts.rs::facts: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/plugins/session/store/facts.rs::snapshot: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/plugins/session/store/facts.rs::runtime_facts: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
// panic-allow symbio/src/plugins/session/store/facts.rs::runtime_cache: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
