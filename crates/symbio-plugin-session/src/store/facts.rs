//! 事实源端口与它的**投影值**（会话存储的实现段之二）。
//!
//! 分在这里的判据是「这一段错了谁会红」：本文件出错，红的是**读到的事实**——
//! 句柄拿不到（写点当场报真失败）、运行态投影读到旧值（上一轮结局说错话）、
//! 召回漏掉会话（记忆视图少一块）。这与「目录长什么样」（`layout`：错了红的是
//! 会话读不出来 / 整份写坏 / 清单少项）和「条目怎么读写删」（`super`）是三批
//! 不同的观众；混在一处时，一次布局改动会顺手碰坏投影，反之亦然。
//!
//! 三件事的形状（[D-084](../../../../docs/003-design/001-核心架构.md#d-084-能力模块公用基础协议上下文只递值事实只经引擎abcde-五类需要的唯一取得方式 "DE-001-核心架构")、
//! [D-083 §5.2](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）：
//!
//! - **句柄只发给引擎写点**，且进程内按会话构造一次（「一写者」，见
//!   `SessionStore::facts` 的文档）；
//! - **读方拿值面**：`fact_source` 交的是契约事实源；跨会话的读经
//!   `recall_snapshots` / `deep_snapshots` 交 `Vec<Event>`，不交路径、不交句柄；
//! - **跨会话的读要预算**：`recall_snapshots` 的名额与字节上限是本文件末尾那几个常量。

use std::collections::BTreeMap;

use serde_json::Value;
use symbio_kernel::event::{Entity, Verb};
use symbio_kernel::store::AppendError;

use super::*;

impl SessionStore {
    /// 开面播种（D-340）：把部署级**已裁决**的 `verdict.asserted` 复制进本会话事实流。
    ///
    /// 只在会话尚无事实（`head() == 0`）时执行一次——这是「开面时一致」的全部含义；
    /// 开面之后才被撤销的主体在本会话内仍能动，运行期**不追发**（已知缺口，见 D-340）。
    ///
    /// 跨会话索引只出现在这里（裁决四）：`deep_snapshots` 是既有的部署级读，交的
    /// 是值不交路径；`DriveTarget` / `PortExecContext` / 执行体一概看不到它。
    ///
    /// 复制走 `Store::append` 直落 WAL，不经 `FactSource::append`：播种**不踢驱动**，
    /// 也不参与义务环的 owes / settled（本会话的第一条业务事实落地时才开轮）。
    pub(crate) fn seed_open_verdicts(&self, session_id: &str) -> Result<(), String> {
        let Some(wal) = self.facts(session_id)? else {
            return Ok(());
        };
        if Store::head(&*wal).value() != 0 {
            return Ok(());
        }
        for source in deployment_verdicts(&self.deep_snapshots()) {
            let source_seq = source.seq.map(|seq| seq.value()).unwrap_or(0);
            let mut payload = source.payload.clone();
            if let Value::Object(map) = &mut payload {
                // 审计对账点：`produced_by` 保留原引用，载荷里的 `source_seq` 指向
                // 被复制事实的号——不新增信封字段（裁决三）。
                map.insert("source_seq".to_string(), Value::from(source_seq));
            }
            let mut seeded = Event::pending(
                source.event_id.clone(),
                source.kind.clone(),
                source.entity,
                source.verb,
                source.actor.clone(),
            );
            seeded.payload = payload;
            seeded.produced_by = source.produced_by;
            seeded.cost_ms = source.cost_ms;
            match Store::append(&*wal, seeded) {
                Ok(_) | Err(AppendError::Duplicate) => {}
                Err(err) => {
                    return Err(format!("开面播种失败（{}）：{err:?}", source.event_id));
                }
            }
        }
        Ok(())
    }

    /// 该会话的**唯一事实源句柄**；会话目录不存在 ⇒ `None`。
    ///
    /// 进程内按会话构造一次并缓存：所有引擎写方共享同一实例，`head` 只有一份、
    /// 内部写锁也只有一把。第二个 `WalStore::open` 不存在——这正是
    /// [DE-001 §4.3](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")
    /// 的「单写者」。跨进程写者令牌仍由 `WalStore` 自己守（旁挂锁文件）。
    ///
    /// **只发给引擎写点**：能力面拿到的是事实的值（读）或待落的值（写），
    /// 不是这个句柄（[D-084](../../../../docs/003-design/001-核心架构.md#d-084-能力模块公用基础协议上下文只递值事实只经引擎abcde-五类需要的唯一取得方式 "DE-001-核心架构") §1）。
    ///
    /// `Ok(None)` = 没有事实源（会话目录不存在）；`Err` = 目录在、事实源
    /// **打不开**——后者是真正的失败，调用方不得当成「没有事实源」静默跳过。
    pub(crate) fn facts(&self, session_id: &str) -> Result<Option<Arc<EventWalStore>>, String> {
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
        let mut store = EventWalStore::open(&wal)
            .map_err(|e| format!("事实源打不开（{}）：{e}", wal.display()))?;
        // 写闸先问装配期冻结表（DE-024 D-198）。装载失败不阻断事实源打开——
        // 写闸仍会退到进程装配表与共享表（已知名字照常），而义务环的装载口会带着同一份
        // 缺口拒绝开幕（`crates/symbio-plugin-loop`）；这里再抛一次只会让会话连读都打不开。
        if let Ok(frozen) = symbio_plugin_api::sealed_declarations() {
            store = store.with_declarations(frozen);
        }
        let store = Arc::new(store);
        cache.insert(session_id.to_string(), store.clone());
        Ok(Some(store))
    }

    /// 该会话的契约事实源（五个方法的中性面）；装配层与插件共用同一底层 `WalStore`。
    pub(crate) fn fact_source(
        &self,
        session_id: &str,
    ) -> Result<Option<Arc<dyn symbio_plugin_api::fact_source::FactSource>>, String> {
        self.fact_source_bound(
            session_id,
            None,
            symbio_kernel::exec::ExecEventSink::silent(),
        )
    }

    /// 按**部署根**绑定的契约事实源：驱动句柄与执行期出口随事实源一起打开。
    ///
    /// 只读调用方走 [`Self::fact_source`]（显式无驱动、静默）；开轮写口必须走本方法，
    /// 把本根的驱动实例与真实转写出口绑上——写口自己不做任何进程级反查。
    pub(crate) fn fact_source_bound(
        &self,
        session_id: &str,
        drive: Option<Arc<dyn symbio_plugin_api::DriveEntry>>,
        sink: symbio_kernel::exec::ExecEventSink,
    ) -> Result<Option<Arc<dyn symbio_plugin_api::fact_source::FactSource>>, String> {
        let dir = self.session_dir(session_id);
        let workdir = dir.as_deref().and_then(super::layout::read_workdir);
        // 会话主体的唯一来源；取不到就显式失败，不回落 `session/main`。
        let agent_id = dir
            .as_deref()
            .and_then(super::layout::read_agent_id)
            .ok_or_else(|| {
                format!("会话 {session_id} 的 metadata 取不到 agent_id（拒绝回落主体）")
            })?;
        Ok(self.facts(session_id)?.map(|wal| {
            super::fact_source::SessionFactSource::open(
                wal,
                session_id.to_string(),
                workdir,
                Some(agent_id),
                drive,
                sink,
            )
        }))
    }

    /// 顶级会话事实源的召回视图（**新近优先**、名额与字节双预算）。
    ///
    /// 召回只扫平级会话（不含嵌套子会话）——范围裁决在
    /// `memory_events::recall_view` 的文档里；
    /// 本函数只负责「枚举 + 按预算开档 + 交值」，可见域过滤仍在读方。
    ///
    /// `own` = 这份事实源是否调用方自己的会话（按目录名比，同一个根下的两层
    /// 派生可能不同形）——`thread_private` 的收放靠它。
    pub(crate) fn recall_snapshots(&self, self_id: &str) -> Vec<FleetSnapshot> {
        let base = &self.base_dir;
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
                    symbio_plugin_api::plugin_warn!(
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
                Err(e) => symbio_plugin_api::plugin_warn!(
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
    pub(crate) fn deep_snapshots(&self) -> Vec<Vec<Event>> {
        let base = &self.base_dir;
        paths::session_wals_deep(base)
            .into_iter()
            .filter_map(|wal| match EventWalStore::open_readonly(&wal) {
                Ok(store) => Some(store.range(Seq::new(0))),
                Err(e) => {
                    symbio_plugin_api::plugin_warn!(
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

/// 部署级 `verdict.asserted` 的 last-wins 视图：每对 `(subject, capability)` 只留
/// **时刻最后**的一条（`ts` 定序，同刻按 `seq`、`event_id` 定序，保证双跑一致）。
///
/// 认格子而不是名字：`verdict × asserted` 上只有裁决者说过的话；`change` 认不出
/// （含缺失）⇒ 跳过该条，不猜。默认能力与读侧覆盖层同口径（`assign.work`）。
fn deployment_verdicts(snapshots: &[Vec<Event>]) -> Vec<Event> {
    let mut last: BTreeMap<(String, String), Event> = BTreeMap::new();
    for events in snapshots {
        for event in events {
            if event.entity != Entity::Verdict || event.verb != Verb::Asserted {
                continue;
            }
            let Some(subject) = event.payload.get("subject").and_then(Value::as_str) else {
                continue;
            };
            let capability = event
                .payload
                .get("capability")
                .and_then(Value::as_str)
                .unwrap_or(symbio_sdk::authz::CAP_ASSIGN_WORK);
            match event.payload.get("change").and_then(Value::as_str) {
                Some("revoke") | Some("restore") => {}
                _ => continue,
            }
            let key = (subject.to_string(), capability.to_string());
            let order = verdict_order(&event);
            let replace = match last.get(&key) {
                None => true,
                Some(current) => verdict_order(current) < order,
            };
            if replace {
                last.insert(key, event.clone());
            }
        }
    }
    last.into_values().collect()
}

/// 跨会话的「最后」只能按时钟比：各会话 `seq` 是各自的号空间，不可互比。
fn verdict_order(event: &Event) -> (i64, u64, &str) {
    (
        event.ts,
        event.seq.map(|seq| seq.value()).unwrap_or(0),
        event.event_id.as_str(),
    )
}

/// 召回扫描预算：最多看几个会话的事实源 / 单文件字节上限 / 总字节上限。
///
/// 超限 ⇒ 不扫并 `plugin_warn` 记一笔——看不见的文件不假装看得见。
/// 预算属于**读成本策略**，枚举属于存储布局，故两者都住在存储层。
pub(crate) const RECALL_MAX_FILES: usize = 16;
pub(crate) const RECALL_MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
pub(crate) const RECALL_MAX_TOTAL_BYTES: u64 = 64 * 1024 * 1024;
// ── panic 面登记（PN-001…003）─────────────────────────────────────────
// 判据与「登记写在代码旁边」的理由见 `store/mod.rs` 末尾同名区块。
// panic-allow crates/symbio-plugin-session/src/store/facts.rs::facts: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
