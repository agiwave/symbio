//! Session 存储 —— **一种磁盘布局，一种驻留方式**
//!
//! ## 它不是什么
//!
//! **没有可切换的存储后端**：会话只有一种落盘布局。为什么不做多后端选型
//! （以及当初那个形状被推翻的两个前提），见 D-008——此处不复述。
//!
//! 无事实源的驻留方式已退场（D-329）：会话只有磁盘一种驻留，因为事实源是
//! 唯一真相——没有事实源的会话壳既读不到历史，也落不进格。
//!
//! | 驻留方式 | 构造 | 条目住在 |
//!|---|---|---|
//! | 持久会话 | [`SessionStore::new`]（根 = `<本插件目录>`） | 磁盘 `<根>/<id>/{session.json, messages.json}` |
//!
//! ## 事件事实源（子模块 `wal`）
//!
//! 会话的**权威事件流**也住本模块：JSONL 追加、跨进程写者令牌、被压缩取代的前缀
//! 归档。它实现 core 的 `Store` 端口，而类型本身
//! **只对本插件可见**——布局 / 锁文件 / 归档段是「会话怎么落事实」的实现细节，
//! 不属于机制层（[D-083](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
//!
//! ## 三段分工（判据：这一段错了谁会红）
//!
//! | 段 | 住在 | 错了红的是 |
//! |---|---|---|
//! | 条目的读 / 写 / 删 / 清单 | 本文件 | 会话打不开、并发丢更新 |
//! | 事实源句柄与投影值 | `facts` | 落不进格、读到旧结局、召回漏会话 |
//! | 磁盘布局 | `layout` | 目录寻错址、整份写坏 |
//!
//! ## 与 VDFS 的关系
//!
//! **对外**，会话只有 VDFS 一个入口（`<根>/session` 的清单 / `消息` / `子会话` /
//! `工作目录`，见 `super::plugin` 的 `impl VdfsProvider`）；本模块是那个 provider 下面
//! 的真相源。**对内**，它刻意**不**改用
//! `providers::vdfs_service` 的三种集中实现：`DirVdfs` 会把条目
//! 内部原样变成对外地址（`session.json` / `messages/` / `tool_archives/` …），而会话
//! 要求 `<id>` 是叶子、内部只以人读语义段呈现；且条目不是文件字节——`<id>/message/<mid>`
//! 由整份 `Session` 派生，seq 分配与剔孤儿是会话专有的写入语义（在 `super::chat_session`）。
//! 共用的只是**寻址**：根由调用方传入，id→段名取
//! [`symbio_providers::vdfs_safe_segment`]
//! （经 [`super::paths::safe_id`]），与资源条目同一份规则。判据见
//! `docs/003-design/009-VDFS规范.md` §13.4（agent 目录是先例）。
//!
//! ## 磁盘布局
//!
//! 形状与它的两条理由（为什么元数据与消息分两个文件、子会话为什么嵌套存放）
//! 写在 `layout` 的模块头——那个文件就是这段布局的实现，判据同表。

use super::types::{ChatNode, Session, SessionSummary};
use crate::paths;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use symbio_kernel::event::{Event, Seq};
use symbio_kernel::plugin::PluginError;
use symbio_kernel::store::Store;
use symbio_sdk::turn::facts::FleetSnapshot;
use tokio::sync::{Mutex, OwnedMutexGuard};

mod fact_source;
mod facts;
mod layout;
mod wal;

pub(crate) use fact_source::remember_storage_dir;

/// 持久化事实源（`Store` 端口的一种实现）。**可见性只到会话插件**：WAL 的布局 /
/// 锁文件 / 归档段都是「会话怎么落事实」的私有细节，别的插件要拿事实走 core 的
/// `Store` 端口或会话的地址面，而不是自己开一个 WAL。
pub(crate) use wal::EventWalStore;
// 布局段的自由函数与两种落盘形状（`layout` 是本模块的孩子，`pub(super)` 到这里可见）。
use layout::*;

/// 父会话目录内存放子会话的固定子目录名——**唯一定义在 `paths`**（路径词汇
/// 单点，部署级读侧的下探遍历按同一个名字走，见 `paths::session_wals_deep`）。
const SUB_SESSIONS_DIR: &str = paths::SUB_SESSIONS_SUBDIR;

/// 消息文件名 —— **与元数据分开存放**（见模块头「元数据与消息分文件」）。
/// 清单只读 `session.json`，**不读**本文件；只有真的要取转写时才读。
///
/// `pub(crate)`：`load_messages` / 迁移路径按同一常量寻址 legacy 缓存，
/// 「文件名只有一份口径」不得各自硬编码字符串。
pub(crate) const MESSAGES_FILE_NAME: &str = "messages.json";

/// 会话主文件名（一个会话目录内的唯一真源）
const SESSION_FILE: &str = "session.json";

/// 会话存储：磁盘是唯一驻留（D-329）
pub struct SessionStore {
    /// 落盘根
    base_dir: PathBuf,
    /// 每会话一把写入锁（见 `lock_writes`）
    write_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    /// 每会话**一个**事实源句柄（见 `facts`）。
    ///
    /// 这是进程内唯一持有写者令牌的实例：同一会话的第二个 `WalStore::open`
    /// 会让 `head` 各自重放、写者令牌互斥 ⇒ 要么重复 seq，要么每次写都被拒。
    /// 缓存一次之后，所有引擎写方共享同一把内部锁，`head` 只有一份。
    facts: std::sync::Mutex<HashMap<String, Arc<EventWalStore>>>,
    /// 每会话的 Agent 权限作用域槽（D-153）：轮装配在 `collect_capabilities`
    /// 之后发布，事实源在 `append` 时取走并随驱动目标交给驱动。
    ///
    /// 键的**存在**不代表已发布（值为 `None` = 本会话没选中 Agent ⇒ 宿主态）；
    /// 只有 agent 插件能把 `agent_id` 解析成作用域，本模块不重读 manifest。
    agent_scopes: std::sync::Mutex<
        HashMap<String, Arc<std::sync::RwLock<Option<symbio_sdk::agent::AgentScope>>>>,
    >,
    /// 每会话的**入站来源主体**槽（D-067）：开轮写方从收件箱条目的 `params` 发布，
    /// 事实源在 `append` 时取走并随驱动目标交给驱动（工具侧据此填 `EffectEnv.source`）。
    ///
    /// 值为 `None` = 本条消息没有外部来源（本机用户发起）⇒ 工具侧回落会话身份。
    source_principals: std::sync::Mutex<HashMap<String, Arc<std::sync::RwLock<Option<String>>>>>,
}

impl SessionStore {
    /// 持久会话存储；`base_dir` 通常是
    /// `SessionPlugin::storage_dir`
    /// （= 本插件自己的目录），测试注入临时目录。
    pub fn new(base_dir: PathBuf) -> Self {
        Self {
            base_dir,
            write_locks: Mutex::new(HashMap::new()),
            facts: std::sync::Mutex::new(HashMap::new()),
            agent_scopes: std::sync::Mutex::new(HashMap::new()),
            source_principals: std::sync::Mutex::new(HashMap::new()),
        }
    }

    /// 该会话的权限作用域槽（共享句柄）：事实源在打开时取走同一个 `Arc`。
    pub(crate) fn agent_scope_cell(
        &self,
        session_id: &str,
    ) -> Arc<std::sync::RwLock<Option<symbio_sdk::agent::AgentScope>>> {
        let mut map = self
            .agent_scopes
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Arc::clone(map.entry(session_id.to_string()).or_default())
    }

    /// 发布本会话的权限作用域（`None` = 明确「本会话没选中 Agent」）。
    ///
    /// 唯一的发布方是会话轮装配（`collect_capabilities` 之后）；事实源只读。
    pub(crate) fn set_agent_scope(
        &self,
        session_id: &str,
        scope: Option<symbio_sdk::agent::AgentScope>,
    ) {
        if let Ok(mut cell) = self.agent_scope_cell(session_id).write() {
            *cell = scope;
        }
    }

    /// 该会话的入站来源主体槽（共享句柄）：事实源在打开时取走同一个 `Arc`。
    pub(crate) fn source_principal_cell(
        &self,
        session_id: &str,
    ) -> Arc<std::sync::RwLock<Option<String>>> {
        let mut map = self
            .source_principals
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        Arc::clone(map.entry(session_id.to_string()).or_default())
    }

    /// 发布本会话这一轮的来源主体（`None` = 本机用户发起）。
    ///
    /// 唯一的发布方是开轮写口（`consume_item_turn`，值来自收件箱条目的 `params`）；
    /// 事实源只读。**每轮都发布**（含 `None`）：上一轮的来源不能漏进下一轮。
    pub(crate) fn set_source_principal(&self, session_id: &str, source: Option<String>) {
        if let Ok(mut cell) = self.source_principal_cell(session_id).write() {
            *cell = source;
        }
    }

    /// 取得该会话的**写入锁**，串行化「整份 load → 改 → 整份重写」这整段临界区。
    ///
    /// 为什么必须包住**整段**而不是只包 `save_session`：会话写入是「读出来、
    /// 改内存、整份写回去」，两次并发调用会各自读到同一份旧数据、各自写回，
    /// 后写的把先写的**整个覆盖**。只锁 save 能防文件写坏，但防不住丢更新——
    /// 而丢更新的后果更隐蔽：消息凭空消失，且磁盘上永远是一个"合法"的文件。
    ///
    /// 锁按 id 惰性创建、进程生命周期内不清理：会话数有限，guard 释放后表里
    /// 留一个空 `Arc` 的成本可忽略。
    pub async fn lock_writes(&self, session_id: &str) -> OwnedMutexGuard<()> {
        let lock = {
            let mut map = self.write_locks.lock().await; // grep-audit-allow S-002: write_locks 是 tokio::sync::Mutex（异步锁，取锁即 await）；S-002 按文件命中 std::sync::Mutex（facts 表）后误扫本行
            map.entry(session_id.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        lock.lock_owned().await
    }

    // ==================== 读 ====================

    /// 加载指定会话；**不存在时返回新建的空 Session**（不报错——"没有"与
    /// "读坏了"是两件事，后者才该失败）
    pub async fn load_session(&self, session_id: &str) -> Result<Session, PluginError> {
        Ok(self
            .load_session_checked(session_id)
            .await?
            .unwrap_or_else(|| Session::new(session_id)))
    }

    /// 加载指定会话；**不存在时返回 `None`**。
    ///
    /// 与 [`Self::load_session`] 的差别只在未命中：那个版本把「没有」也归成空会话，
    /// 于是调用方拿不到存在性判据。需要区分两者时（VDFS 的 `stat` / `read` 必须对
    /// 不存在的会话报 `NotFound`）用本方法——**按 id 直取**，不必先列全量清单再 `find`。
    pub async fn load_session_checked(
        &self,
        session_id: &str,
    ) -> Result<Option<Session>, PluginError> {
        let base = &self.base_dir;

        let path = file_for(base, session_id);
        if path.exists() {
            let content = tokio::fs::read_to_string(&path)
                .await
                .map_err(|e| PluginError::InternalError(format!("读取会话文件失败: {e}")))?;
            // 元数据与消息**分两个文件**：先取元数据，再按需取消息
            let meta = match parse_meta_content(&content) {
                Some(m) => m,
                None => {
                    return Err(PluginError::ParseError(
                        "解析会话失败: 内容无法恢复".to_string(),
                    ))
                }
            };
            let dir = dir_for(base, session_id);
            let messages = load_messages(&dir, &meta).await;
            return Ok(Some(meta.into_session(messages)));
        }
        // 顶层未命中：嵌套查找（子会话凭自身 id 直接寻址）
        if let Some(dir) = find_nested_dir(base, session_id) {
            return Ok(read_session_dir(&dir));
        }
        Ok(None)
    }

    // ==================== 写 / 删 ====================

    /// 持久化保存会话（整份覆盖）
    pub async fn save_session(&self, session: &Session) -> Result<(), PluginError> {
        let base = &self.base_dir;

        // 归属路由：声明了 parent_session_id 的会话存入父目录的 sessions/ 子目录
        let dir = match session.parent_session_id() {
            Some(parent) => sub_dir_for(base, parent, &session.id),
            None => dir_for(base, &session.id),
        };
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| PluginError::InternalError(format!("创建会话目录失败: {e}")))?;

        // 元数据落盘（[D-049]：**消息正文不再由 store 写**）：`v2-events.wal` 是
        // 会话消息的唯一事实源（由事实路径 `chat_session/write.rs` 入格），`messages.json`
        // 停写——它是 v1 遗留会话的唯一记录，只在「WAL 不在」时作回落读法被读。
        //
        // 只写 `session.json`（标题 / 条数 / 摘要 / 标签等清单投影）：列一次清单的
        // 成本与会话聊了多久无关。原子写（tmp + rename）：直接 fs::write 在多个并发
        // 保存交错时会留下"短 JSON + 长旧内容残留"，rename 覆盖保证磁盘上永远是
        // 某一刻的完整版本。
        let summary = SessionSummary::of(session);
        atomic_write(&dir.join(SESSION_FILE), &SessionMetaFile::of(&summary)).await
    }

    /// 删除指定会话（含所有关联存档与——顶层会话的——全部子会话）
    pub async fn delete_session(&self, session_id: &str) -> Result<(), PluginError> {
        let base = &self.base_dir;

        let dir = dir_for(base, session_id);
        if dir.exists() {
            tokio::fs::remove_dir_all(&dir)
                .await
                .map_err(|e| PluginError::InternalError(format!("删除会话目录失败: {e}")))?;
            self.facts
                .lock()
                .expect("会话事实源表锁中毒")
                .remove(session_id);
            return Ok(());
        }
        // 顶层未命中：嵌套删除（子会话级联于父目录之外的单点删除）
        if let Some(nested) = find_nested_dir(base, session_id) {
            tokio::fs::remove_dir_all(&nested)
                .await
                .map_err(|e| PluginError::InternalError(format!("删除会话目录失败: {e}")))?;
            self.facts
                .lock()
                .expect("会话事实源表锁中毒")
                .remove(session_id);
        }
        Ok(())
    }

    // ==================== 清单 ====================

    /// 有界清单：最多 `limit` 条，从游标 `before`（某条会话 id）之后继续；
    /// 两者都 `None` = 全量（按 `updated_at` 降序，只列**顶层**会话，子会话不在其中）。
    ///
    /// 返回值**不含消息**——类型上就带不走（见 `SessionSummary`）。这是清单
    /// 与消息量脱钩的关键：读一次清单的成本与「会话聊了多久」无关。
    ///
    /// `limit` 计量单位是**清单项**（一个会话 = 一项），与转写窗口的「根节点」
    /// 口径不同——这里没有父子结构，一项就是一项。`None` = 不截断。
    ///
    /// 排序与截断都在内存里做：窗口约束的是**响应大小**（少传多少节点给前端），
    /// 而 IO 成本已经由「清单只读 `session.json`」压平了——再引入一份索引文件
    /// 换来的收益不值一个新文件（见 docs/019-perf.md §3）。
    pub async fn list_sessions_window(
        &self,
        limit: Option<u32>,
        before: Option<&str>,
    ) -> Result<Vec<SessionSummary>, PluginError> {
        let mut all = self.read_all_summaries().await?;
        sort_summaries_desc(&mut all);

        // 游标：找到它之后从下一条开始；找不到（已被删 / 已到末尾）就整段返回，
        // 让调用方自然收敛，不报错。
        let start = match before {
            Some(b) => all
                .iter()
                .position(|s| s.id == b)
                .map(|i| i + 1)
                .unwrap_or(all.len()),
            None => 0,
        };
        let mut out: Vec<SessionSummary> = all.into_iter().skip(start).collect();
        if let Some(limit) = limit {
            out.truncate(limit as usize);
        }
        Ok(out)
    }

    /// 子会话清单：`<根>/<safe(父)>/sessions/*/session.json`，`updated_at` 降序。
    pub async fn list_sub_sessions(
        &self,
        parent_id: &str,
    ) -> Result<Vec<SessionSummary>, PluginError> {
        let base = &self.base_dir;
        let nested = dir_for(base, parent_id).join(SUB_SESSIONS_DIR);
        if !nested.exists() {
            return Ok(Vec::new());
        }
        let mut summaries = tokio::task::spawn_blocking(move || read_summaries_in(&nested))
            .await
            .map_err(|e| PluginError::InternalError(e.to_string()))?;
        sort_summaries_desc(&mut summaries);
        Ok(summaries)
    }

    /// 未经排序 / 未开窗的全量摘要
    async fn read_all_summaries(&self) -> Result<Vec<SessionSummary>, PluginError> {
        let base = &self.base_dir;
        if !base.exists() {
            return Ok(Vec::new());
        }
        let base = self.base_dir.clone();
        tokio::task::spawn_blocking(move || read_summaries_in(&base))
            .await
            .map_err(|e| PluginError::InternalError(e.to_string()))
    }

    /// 一次性迁移：把**旧布局**（消息内联在 `session.json`）拆成两个文件，
    /// 并顺带补写缺失的清单投影。
    ///
    /// 幂等：新布局且投影齐全的目录直接跳过（`get_store` 每次构造只跑一次）。
    /// 为什么要补投影：投影字段是后来加的 `#[serde(default)]`，存量文件里没有，
    /// 少了它清单会静默退化成显示 id（见 docs/019-perf.md §11.3）。
    pub(crate) async fn migrate_split_messages(&self) -> Result<(), PluginError> {
        let base = &self.base_dir;
        if !base.exists() {
            return Ok(());
        }
        for dir in session_dirs_deep(base) {
            if let Err(e) = split_inline_messages(&dir).await {
                // 单个会话迁移失败不该拖垮整个会话插件——读路径有兜底（内联消息）
                symbio_plugin_api::plugin_warn!("session", "迁移会话目录失败 {:?}: {}", dir, e);
            }
        }
        Ok(())
    }

    // ==================== 目录寻址 ====================

    /// 该会话的本地目录（压缩存档、transcript 转存的落点）；目录不存在 ⇒ `None`。
    pub fn session_dir(&self, session_id: &str) -> Option<PathBuf> {
        let base = &self.base_dir;
        let top = dir_for(base, session_id);
        if top.exists() {
            return Some(top);
        }
        find_nested_dir(base, session_id)
    }
}

/// 取（或首次建）存储单例——**slot 上的懒初始化，owner 是这个自由函数**。
///
/// `slot` 是共享的 `OnceCell`，`dir` 是本插件目录（存储根）。住在插件上的
/// `SessionPlugin::get_store` 与住在收敛手柄上的那份（`orchestrator::Convergence`）
/// 都必须走这里：初始化只有一条路，迁移只跑一次，两个持有者不会各建一个 store。
///
/// 一次性迁移（旧布局 → 元数据 / 消息两个文件）在首次建时跑；读路径本来就有兜底，
/// 迁移失败不影响可用性，因此只告警不上抛。
pub(crate) async fn resolve_store(
    slot: &tokio::sync::OnceCell<Arc<SessionStore>>,
    dir: &Path,
) -> Result<Arc<SessionStore>, PluginError> {
    if let Some(store) = slot.get() {
        return Ok(Arc::clone(store));
    }

    let store = ready_shared_store(dir).await;

    // OnceCell::set 在多 writer 竞争时可能失败，但失败时另一线程已成功，直接拿
    let _ = slot.set(Arc::clone(&store));
    Ok(slot.get().cloned().unwrap_or_else(|| Arc::clone(&store)))
}

/// 按插件目录共享的存储实例：**进程内一个目录只有一个**。
///
/// 为什么需要它：生产驱动入口要读**同一个**事实源来发现新事实，而 `WalStore`
/// 在进程内按会话只许有**一个**写句柄（后开的那个不持跨进程令牌）。插件与装配
/// 各自 `SessionStore::new` 会各开一份，后开的把先开的顶掉——事实源随即分叉。
/// 因此目录 → 实例的映射放在进程级，两个取用方拿到同一个 `Arc`。
struct SharedEntry {
    store: Arc<SessionStore>,
    migrated: tokio::sync::OnceCell<()>,
}

fn shared_entries() -> &'static std::sync::Mutex<HashMap<PathBuf, Arc<SharedEntry>>> {
    static ENTRIES: std::sync::OnceLock<std::sync::Mutex<HashMap<PathBuf, Arc<SharedEntry>>>> =
        std::sync::OnceLock::new();
    ENTRIES.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn shared_entry(dir: &Path) -> Arc<SharedEntry> {
    let mut entries = shared_entries()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    Arc::clone(entries.entry(dir.to_path_buf()).or_insert_with(|| {
        Arc::new(SharedEntry {
            store: Arc::new(SessionStore::new(dir.to_path_buf())),
            migrated: tokio::sync::OnceCell::new(),
        })
    }))
}

async fn ready_shared_store(dir: &Path) -> Arc<SessionStore> {
    let entry = shared_entry(dir);
    let store = Arc::clone(&entry.store);
    entry
        .migrated
        .get_or_init(|| async move {
            if let Err(e) = store.migrate_split_messages().await {
                symbio_plugin_api::plugin_warn!(
                    "session",
                    "会话存储迁移失败（读路径有兜底）: {}",
                    e
                );
            }
        })
        .await;
    Arc::clone(&entry.store)
}

#[cfg(test)]
mod tests;

// ── panic 面登记（PN-001…003）─────────────────────────────────────────
// 本文件每一处 `unwrap` / `expect` / `panic!` / `unreachable!` 的理由。登记放在
// 文件内而不是集中一张表：理由与它解释的那段代码会一起被 review、一起被删。
// 判据见 `scripts/panic-audit.mjs`。**加一处 panic 必须同时加一行登记，理由非空。**
// panic-allow crates/symbio-plugin-session/src/store/mod.rs::delete_session: std 锁中毒只在持锁期间 panic 时传播，本仓把这些锁当无中毒用（同一条约定）。改成毒后恢复是行为变更，需单独 ADR。
