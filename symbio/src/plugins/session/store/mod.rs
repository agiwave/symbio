//! Session 存储 —— **一种磁盘布局，两种驻留方式**
//!
//! ## 它不是什么
//!
//! **没有可切换的存储后端**：会话只有一种落盘布局，临时会话只是「不持久化」。
//! 为什么不做多后端选型（以及当初那个形状被推翻的两个前提），见
//! ADR-008——此处不复述。
//!
//! 差异收成构造时的一次选型：
//!
//! | 驻留方式 | 构造 | 条目住在 |
//!|---|---|---|
//! | 持久会话 | [`SessionStore::new`]（根 = `<本插件目录>`） | 磁盘 `<根>/<id>/{session.json, messages.json}` |
//! | 临时会话 | [`SessionStore::ephemeral`] | 进程内，退出即丢 |
//!
//! ## 事件事实源（子模块 `wal`）
//!
//! 会话的**权威事件流**也住本模块：JSONL 追加、跨进程写者令牌、被压缩取代的前缀
//! 归档。它实现 core 的 [`Store`](crate::symbio_core::Store) 端口，而类型本身
//! **只对本插件可见**——布局 / 锁文件 / 归档段是「会话怎么落事实」的实现细节，
//! 不属于机制层（[plan/14 §2.2 推论 2](../../../../../docs/004-plan/014-单环一表收敛.md "PL-014-单环一表收敛")）。
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
//! [`crate::providers::vdfs_safe_segment`]
//! （经 [`super::paths::safe_id`]），与资源条目同一份规则。判据见
//! `docs/005-arch/013-vdfs.md` §13.4（agent 目录是先例）。
//!
//! ## 磁盘布局
//!
//! ```text
//! 顶层会话：`<根>/<safe_id>/session.json`     ← 元数据 + 清单投影（小）
//!           `<根>/<safe_id>/messages.json`    ← 消息（大）
//! 消息压缩存档：`<根>/<safe_id>/messages/msg_*.txt`
//! ```
//!
//! ### 为什么元数据与消息要分两个文件
//!
//! 清单只读 `session.json`，其大小与会话聊了多久**无关**；`messages.json` 只有真的
//! 要取转写时才读。代价是清单字段（标题 / 条数 / 摘要 / 标签）不能现算，必须在 `save`
//! 时算好落盘——它们是**投影**（[`super::types::SessionSummary`]），可重算，不是第二份
//! 真相。性能取舍见 `symbio/src/plugins/session/docs/019-perf.md`。
//!
//! 旧布局（消息内联在 `session.json`）仍**可读**：`messages.json` 缺失时回落到内联
//! `messages` 字段；投影缺失时用内联消息**就地补算**（[`SessionMetaFile::into_summary`]）
//! ——缺了这条兜底，清单会静默退化成显示 id。
//!
//! ### 子会话嵌套存储（机制约定，见 docs/005-arch/013-vdfs.md）
//!
//! 会话可派生子会话：子会话不是顶层平级条目，而是存放在父会话目录内的
//! `sessions/` 子目录——`<根>/<safe(父)>/sessions/<safe(子)>/`。
//!
//! 归属声明与路由规则（调用方无感知）：
//! - **归属声明**：`metadata.parent_session_id`（父会话 id）。save 时据此
//!   路由到嵌套目录（无该元数据 = 顶层会话）；
//! - **load / delete / session_dir**：先查顶层，未命中则扫描各会话目录的
//!   `sessions/<id>/`（子会话可凭自身 id 直接寻址）；
//! - **list_sessions_window**：只列顶层（子会话不出现在顶层清单）；
//!   子会话清单走 [`list_sub_sessions`](SessionStore::list_sub_sessions)；
//! - **级联删除**：删除父会话 = `remove_dir_all(父目录)`，子会话随之删除。
//!
//! 子会话 id 由系统生成，与顶层 id 同一格式约束。临时会话无目录概念，
//! 因此不参与嵌套（`list_sub_sessions` 恒空、`session_dir` 恒 `None`）。

use super::types::{ChatNode, Session, SessionSummary};
use crate::plugins::session::paths;
use crate::symbio_core::PluginError;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, OwnedMutexGuard, RwLock};

mod wal;

/// 持久化事实源（`Store` 端口的一种实现）。**可见性只到会话插件**：WAL 的布局 /
/// 锁文件 / 归档段都是「会话怎么落事实」的私有细节，别的插件要拿事实走 core 的
/// `Store` 端口或会话的地址面，而不是自己开一个 WAL。
pub(in crate::plugins::session) use wal::EventWalStore;
#[cfg(test)]
pub(in crate::plugins::session) use wal::WalStore;

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

/// 会话存储：一个具体类型，落盘与不落盘由构造时选定（见模块头）
pub struct SessionStore {
    /// 落盘根；`None` = 不落盘（临时会话），条目只活在 `mem` 里
    base_dir: Option<PathBuf>,
    /// 不落盘时的进程内条目表（落盘型恒空）
    mem: RwLock<BTreeMap<String, Session>>,
    /// 每会话一把写入锁（见 [`lock_writes`](Self::lock_writes)）
    write_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl SessionStore {
    /// 持久会话存储；`base_dir` 通常是
    /// [`SessionPlugin::storage_dir`](super::plugin::SessionPlugin::storage_dir)
    /// （= 本插件自己的目录），测试注入临时目录。
    pub fn new(base_dir: PathBuf) -> Self {
        Self {
            base_dir: Some(base_dir),
            mem: RwLock::new(BTreeMap::new()),
            write_locks: Mutex::new(HashMap::new()),
        }
    }

    /// 不落盘的临时会话（`_t_` 前缀 / 空 `session_id`，以及编排器未交付
    /// 会话句柄时的兜底会话）。
    ///
    /// 每次构造一张新表——临时会话的可见范围就是这一个会话句柄的生命周期。
    pub fn ephemeral() -> Self {
        Self {
            base_dir: None,
            mem: RwLock::new(BTreeMap::new()),
            write_locks: Mutex::new(HashMap::new()),
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
            let mut map = self.write_locks.lock().await;
            map.entry(session_id.to_string())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        lock.lock_owned().await
    }

    /// 落盘根；`None` = 本实例不落盘
    fn disk(&self) -> Option<&Path> {
        self.base_dir.as_deref()
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
        let Some(base) = self.disk() else {
            return Ok(self.mem.read().await.get(session_id).cloned());
        };

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
        let Some(base) = self.disk() else {
            self.mem
                .write()
                .await
                .insert(session.id.clone(), session.clone());
            return Ok(());
        };

        // 归属路由：声明了 parent_session_id 的会话存入父目录的 sessions/ 子目录
        let dir = match session.parent_session_id() {
            Some(parent) => sub_dir_for(base, parent, &session.id),
            None => dir_for(base, &session.id),
        };
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| PluginError::InternalError(format!("创建会话目录失败: {e}")))?;

        // 元数据落盘（[ADR-049]：**消息正文不再由 store 写**）：`v2-events.wal` 是
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
        let Some(base) = self.disk() else {
            self.mem.write().await.remove(session_id);
            return Ok(());
        };

        let dir = dir_for(base, session_id);
        if dir.exists() {
            tokio::fs::remove_dir_all(&dir)
                .await
                .map_err(|e| PluginError::InternalError(format!("删除会话目录失败: {e}")))?;
            return Ok(());
        }
        // 顶层未命中：嵌套删除（子会话级联于父目录之外的单点删除）
        if let Some(nested) = find_nested_dir(base, session_id) {
            tokio::fs::remove_dir_all(&nested)
                .await
                .map_err(|e| PluginError::InternalError(format!("删除会话目录失败: {e}")))?;
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
    /// 不落盘的临时会话恒空（无目录概念）。
    pub async fn list_sub_sessions(
        &self,
        parent_id: &str,
    ) -> Result<Vec<SessionSummary>, PluginError> {
        let Some(base) = self.disk() else {
            return Ok(Vec::new());
        };
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

    /// 未经排序 / 未开窗的全量摘要（两种驻留方式共用）
    async fn read_all_summaries(&self) -> Result<Vec<SessionSummary>, PluginError> {
        let Some(base) = self.disk() else {
            let mem = self.mem.read().await;
            return Ok(mem.values().map(SessionSummary::of).collect());
        };
        if !base.exists() {
            return Ok(Vec::new());
        }
        let base = base.to_path_buf();
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
        let Some(base) = self.disk() else {
            return Ok(());
        };
        if !base.exists() {
            return Ok(());
        }
        for dir in session_dirs_deep(base) {
            if let Err(e) = split_inline_messages(&dir).await {
                // 单个会话迁移失败不该拖垮整个会话插件——读路径有兜底（内联消息）
                crate::plugin_warn!("session", "迁移会话目录失败 {:?}: {}", dir, e);
            }
        }
        Ok(())
    }

    // ==================== 目录寻址 ====================

    /// 该会话的本地目录（压缩存档、transcript 转存的落点）。
    ///
    /// 临时会话返回 `None`：存档路径解析因此退化为"无存档"，符合临时会话
    /// 不回看历史的定位。
    pub fn session_dir(&self, session_id: &str) -> Option<PathBuf> {
        let base = self.disk()?;
        let top = dir_for(base, session_id);
        if top.exists() {
            return Some(top);
        }
        find_nested_dir(base, session_id)
    }
}

// ==================== 落盘细节（自由函数） ====================
//
// 都是纯映射 / 纯 IO，不需要 `&self`——挂成自由函数而非私有方法，是为了让
// 「哪一段依赖实例状态」在名字上一眼可辨（`session_dir` 需要，`dir_for` 不需要）。

/// `<base>/<safe_id>/`
///
/// id→段名的唯一实现在宿主层，经 [`paths::safe_id`] 进入（见模块头）。
fn dir_for(base: &Path, session_id: &str) -> PathBuf {
    base.join(paths::safe_id(session_id))
}

/// `<base>/<safe_id>/session.json`
fn file_for(base: &Path, session_id: &str) -> PathBuf {
    dir_for(base, session_id).join(SESSION_FILE)
}

/// 子会话目录：`<base>/<safe(父)>/sessions/<safe(子)>/`
fn sub_dir_for(base: &Path, parent_id: &str, session_id: &str) -> PathBuf {
    base.join(paths::safe_id(parent_id))
        .join(SUB_SESSIONS_DIR)
        .join(paths::safe_id(session_id))
}

/// `session.json` 的落盘形状 —— **元数据 + 清单投影**（小，与消息量无关）
///
/// `title` / `message_count` / `summary` / `meta_tags` 是**投影**：由
/// [`SessionSummary::of`] 在保存时算好，清单只读它们、绝不读消息。
/// 三者都带 `#[serde(default)]`，因为它们是后加的——存量文件里没有，
/// 此时由 [`SessionMetaFile::into_summary`] 用内联消息就地补算。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionMetaFile {
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    created_at: i64,
    #[serde(default)]
    updated_at: i64,
    #[serde(default)]
    metadata: serde_json::Value,
    #[serde(default)]
    message_count: usize,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    meta_tags: Vec<String>,
    /// 旧布局把消息内联在这里。新布局**不写**该字段；读的时候仍要认，
    /// 否则存量会话会读成空。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    messages: Option<Vec<ChatNode>>,
}

impl SessionMetaFile {
    fn of(s: &SessionSummary) -> Self {
        Self {
            id: s.id.clone(),
            title: s.title.clone(),
            created_at: s.created_at,
            updated_at: s.updated_at,
            metadata: s.metadata.clone(),
            message_count: s.message_count,
            summary: s.summary.clone(),
            meta_tags: s.meta_tags.clone(),
            messages: None,
        }
    }

    /// 补上消息，得到完整 `Session`
    fn into_session(self, messages: Vec<ChatNode>) -> Session {
        Session {
            id: self.id,
            messages,
            created_at: self.created_at,
            updated_at: self.updated_at,
            metadata: self.metadata,
        }
    }

    /// 清单投影。
    ///
    /// ⚠️ 存量文件没有投影字段（`title` 空 / `message_count` 0 / `summary` None），
    /// 此时用**内联消息就地补算**——少了这段，清单会静默退化成显示会话 id
    /// （一串短 guid），且不报任何错。
    fn into_summary(self) -> SessionSummary {
        let projection_missing = self.title.is_empty();
        if projection_missing {
            if let Some(messages) = self.messages.as_ref() {
                let s = Session {
                    id: self.id.clone(),
                    messages: messages.clone(),
                    created_at: self.created_at,
                    updated_at: self.updated_at,
                    metadata: self.metadata.clone(),
                };
                return SessionSummary::of(&s);
            }
        }
        SessionSummary {
            id: self.id,
            title: self.title,
            created_at: self.created_at,
            updated_at: self.updated_at,
            metadata: self.metadata,
            message_count: self.message_count,
            summary: self.summary,
            meta_tags: self.meta_tags,
        }
    }
}

/// `messages.json` 的落盘形状 —— 只有消息（大，只有真要取转写时才读）
#[derive(Debug, Clone, Serialize, Deserialize)]
struct MessagesFile {
    #[serde(default)]
    messages: Vec<ChatNode>,
}

/// 原子写：先落 tmp 再 rename。
///
/// 直接 `fs::write`（O_TRUNC + write）在并发保存交错时会留下「短 JSON + 长旧内容
/// 残留」，产生 trailing characters 损坏；rename 保证磁盘上永远是某一刻的完整版本。
async fn atomic_write<T: Serialize>(target: &Path, value: &T) -> Result<(), PluginError> {
    let json = serde_json::to_string(value)
        .map_err(|e| PluginError::InternalError(format!("序列化会话文件失败: {e}")))?;
    let tmp = tmp_path_for(target);
    tokio::fs::write(&tmp, json)
        .await
        .map_err(|e| PluginError::InternalError(format!("写入会话临时文件失败: {e}")))?;
    tokio::fs::rename(&tmp, target)
        .await
        .map_err(|e| PluginError::InternalError(format!("替换会话文件失败: {e}")))
}

/// 解析 session.json 内容；存在尾部残留时截取首个完整 JSON 自愈
/// （流式反序列化取首个完整对象），完全无法解析时返回 None。
fn parse_meta_content(content: &str) -> Option<SessionMetaFile> {
    parse_or_self_heal::<SessionMetaFile>(content)
}

fn parse_messages_content(content: &str) -> Option<Vec<ChatNode>> {
    parse_or_self_heal::<MessagesFile>(content).map(|f| f.messages)
}

/// 先整体解析，失败则退回「取首个完整对象」自愈
fn parse_or_self_heal<T: serde::de::DeserializeOwned>(content: &str) -> Option<T> {
    match serde_json::from_str(content) {
        Ok(v) => Some(v),
        Err(_) => serde_json::Deserializer::from_str(content)
            .into_iter::<T>()
            .next()
            .and_then(Result::ok),
    }
}

/// 读消息——**真源是 `v2-events.wal`**（ADR-049）：
/// 有 WAL 就从它重建；WAL 不在才回落 `messages.json` / 元数据内联（v1 时代
/// 的遗留布局——那批会话的事实从未入格，文件是它们的唯一记录）。
async fn load_messages(dir: &Path, meta: &SessionMetaFile) -> Vec<ChatNode> {
    if let Some(msgs) = rebuild_from_wal_dir(dir) {
        return msgs;
    }
    let path = dir.join(MESSAGES_FILE_NAME);
    if let Ok(content) = tokio::fs::read_to_string(&path).await {
        if let Some(msgs) = parse_messages_content(&content) {
            return msgs;
        }
    }
    meta.messages.clone().unwrap_or_default()
}

/// 从 `<会话目录>/v2-events.wal` 重建消息列表（真源读取）。
///
/// WAL 不在 / 打不开 ⇒ `None`（调用方回落遗留读法——**没有 WAL 不等于没有历史**，
/// v1 时代落的 `messages.json` 仍是那批会话的唯一记录）。只读打开：读路径无写副作用。
fn rebuild_from_wal_dir(dir: &Path) -> Option<Vec<ChatNode>> {
    let path = dir.join(crate::plugins::session::paths::V2_WAL_FILE);
    if !path.exists() {
        return None;
    }
    let store = crate::plugins::session::store::EventWalStore::open_readonly(&path).ok()?;
    // `range` 是 `Store` trait 的方法：trait 必须在作用域里（局部引入，
    // 不为一次读取把这个 trait 抬进本文件的顶层导入）。
    use crate::symbio_core::Store as _;
    let snapshot = store.range(crate::symbio_core::Seq::new(0));
    Some(super::rebuild::rebuild(&snapshot))
}

/// **现行窗口**：从最近一份快照事件起的那段（含快照本身；没有快照 ⇒ 整份都是现行）。
///
/// [`Store::range`](crate::symbio_core::Store::range) 归一后给的是**完整史**（含被
/// [`archive_before`](crate::plugins::session::store::WalStore::archive_before) 移出的前缀，
/// [plan/14 §2.2](../../../../../docs/004-plan/014-单环一表收敛.md "PL-014-单环一表收敛") 推论 1），
/// 而「**这一轮**还开着没收束」判的是现行历史：被快照取代的那段已不参与上下文，
/// 拿它找未收束轮次会把陈锚当现行（压缩期间等审批的那一轮正是这种形状）。
///
/// 窗口由**消费方按事实切**，不是存储层的第二种读法——这正是归一的前提：读法只有一种，
/// 想要窗口就从「最近一份快照落在第几号」自己切。折叠侧的同类判据（消息列表 / 轮次计数）
/// 已经内建在 `mutations_latest_snapshot` 的用法里，不需要本函数。
pub(crate) fn current_window(events: &[crate::symbio_core::Event]) -> &[crate::symbio_core::Event] {
    match crate::symbio_core::mutations_latest_snapshot(events) {
        Some((i, _)) => &events[i..],
        None => events,
    }
}

/// 同步版读 `messages.json`
fn read_messages_sync(dir: &Path) -> Option<Vec<ChatNode>> {
    let content = std::fs::read_to_string(dir.join(MESSAGES_FILE_NAME)).ok()?;
    parse_messages_content(&content)
}

/// 读 `session.json`（同步；用于 spawn_blocking 里的批量读取）
fn read_meta(dir: &Path) -> Option<SessionMetaFile> {
    let content = std::fs::read_to_string(dir.join(SESSION_FILE)).ok()?;
    parse_meta_content(&content)
}

/// 读整个会话目录 = 元数据 + 消息（同步；嵌套查找的 miss 路径用）。
///
/// 消息同样以 WAL 为准（与 [`load_messages`] 同一条真源纪律），遗留读法是回落。
fn read_session_dir(dir: &Path) -> Option<Session> {
    let meta = read_meta(dir)?;
    // 「两个来源都没有消息」= 空会话（不是「不存在」），别把「没有消息」当成「不存在」
    let messages = rebuild_from_wal_dir(dir)
        .or_else(|| read_messages_sync(dir))
        .or_else(|| meta.messages.clone())
        .unwrap_or_default();
    Some(meta.into_session(messages))
}

/// 嵌套查找：在所有顶层会话目录的 `sessions/<safe_id>/` 中定位子会话，
/// 返回其目录。仅在顶层未命中时调用（miss 路径，开销可接受）。
fn find_nested_dir(base: &Path, session_id: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(base).ok()?;
    for entry in entries.flatten() {
        let dir = entry
            .path()
            .join(SUB_SESSIONS_DIR)
            .join(paths::safe_id(session_id));
        if dir.join(SESSION_FILE).is_file() {
            return Some(dir);
        }
    }
    None
}

/// 列出某目录下所有 `<dir>/<子>/session.json` 的**摘要**（跳过不可解析项）
///
/// 只读 `session.json`——这里正是「清单不碰消息」的落点。
fn read_summaries_in(dir: &Path) -> Vec<SessionSummary> {
    let mut out = Vec::new();
    for child in session_dirs_under(dir) {
        if let Some(meta) = read_meta(&child) {
            out.push(meta.into_summary());
        }
    }
    out
}

/// 某目录下所有「含 session.json 的子目录」
fn session_dirs_under(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join(SESSION_FILE).is_file())
        .collect()
}

/// 递归收集所有会话目录（含嵌套子会话；迁移用——子会话也可能还是旧布局）
fn session_dirs_deep(base: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for child in session_dirs_under(dir) {
            out.push(child.clone());
            walk(&child.join(SUB_SESSIONS_DIR), out);
        }
    }
    let mut out = Vec::new();
    walk(base, &mut out);
    out
}

/// 把一个旧布局目录（消息内联）拆成两个文件，并补写缺失的投影。幂等。
///
/// ## 迁移边界（[ADR-049]）
///
/// `messages.json` 已**不再是**布局的一部分：真源是 `v2-events.wal`，本函数只为
/// **v1 遗留目录**服务——那些把消息内联在 `session.json` 里、事实从未入格的会话。
/// 因此判据是「**有内联消息吗**」：没有内联消息的目录（新布局会话 / 已迁过的）
/// 一律跳过。少了这条，新会话会在首次迁移时被写出一份**空 `messages.json`**
/// ——把我们刚停写的缓存又凭空造出来（读侧虽已不读它，但凭空多一份空文件是布局污染）。
/// 存量补投影那半仍保留（`title` 空的旧文件要补）。
async fn split_inline_messages(dir: &Path) -> Result<(), PluginError> {
    let Some(meta) = read_meta(dir) else {
        return Ok(());
    };
    let has_split = dir.join(MESSAGES_FILE_NAME).is_file();
    let inline = meta.messages.clone();
    // 已是新布局（消息文件在 / 内联已空）、且投影齐全 → 无需改动。
    //
    // 「有内联消息」是唯一的迁移触发条件：`messages.json` 停写后，一份**既无**
    // 消息文件、**又无**内联消息的目录就是新布局的正常态，绝不能因此补写空缓存。
    let needs_split = inline.as_ref().is_some_and(|m| !m.is_empty());
    let projection_missing = meta.title.is_empty();
    if !needs_split && !projection_missing {
        return Ok(());
    }

    let messages = if has_split {
        // 消息文件读不出来时**不猜**：宁可留着旧投影，也不要把空写进元数据
        match read_messages_sync(dir) {
            Some(m) => m,
            None => return Ok(()),
        }
    } else {
        inline.unwrap_or_default()
    };

    // 只在**确有内联消息要拆**时才落 `messages.json`（停写后不再凭空补一份空缓存）。
    if !has_split && needs_split {
        atomic_write(
            &dir.join(MESSAGES_FILE_NAME),
            &MessagesFile {
                messages: messages.clone(),
            },
        )
        .await?;
    }

    // 投影**必须**按现有消息重算：这才是「存量文件补投影」的落点
    let summary = SessionSummary::of(&Session {
        id: meta.id.clone(),
        messages,
        created_at: meta.created_at,
        updated_at: meta.updated_at,
        metadata: meta.metadata.clone(),
    });
    atomic_write(&dir.join(SESSION_FILE), &SessionMetaFile::of(&summary)).await
}

/// 清单排序：`updated_at` 降序（两种驻留方式共用同一份，清单顺序不分叉）
fn sort_summaries_desc(summaries: &mut [SessionSummary]) {
    summaries.sort_by_key(|b| std::cmp::Reverse(b.updated_at));
}

/// 原子写的临时文件路径：`<目标名>.<pid>.<单调序号>.tmp`
///
/// tmp 名**每次唯一**。早先它由调用方传固定名（`messages.json.tmp`），两次并发
/// 保存会交错写同一个 tmp：A 写完、B 覆盖同一 tmp，A 的 rename 便把 B 的内容放进
/// A 的目标文件——**rename 的原子性只保证「不写坏」，保证不了「写的是自己那份」**。
///
/// 序号取进程内原子计数（同进程两次写必然不同名），`pid` 兼顾多进程
/// （同一目录被两个实例打开时）。
fn tmp_path_for(target: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let mut name = target
        .file_name()
        .map(|s| s.to_os_string())
        .unwrap_or_default();
    name.push(format!(".{}.{n}.tmp", std::process::id()));
    target.with_file_name(name)
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

    let store = Arc::new(SessionStore::new(dir.to_path_buf()));
    if let Err(e) = store.migrate_split_messages().await {
        crate::plugin_warn!("session", "会话存储迁移失败（读路径有兜底）: {}", e);
    }

    // OnceCell::set 在多 writer 竞争时可能失败，但失败时另一线程已成功，直接拿
    let _ = slot.set(Arc::clone(&store));
    Ok(slot.get().cloned().unwrap_or_else(|| Arc::clone(&store)))
}

#[cfg(test)]
mod tests;
