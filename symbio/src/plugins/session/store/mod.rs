//! Session 存储 —— **一种磁盘布局，两种驻留方式**
//!
//! ## 它不是什么
//!
//! 这里过去是「一个 trait + 三个后端」：`FileSessionStore` / `SqliteSessionStore`
//! / `InMemorySessionStore`，由配置项 `store_kind` 经 `create_store` 工厂选型。
//! 那个形状有两个已证伪的前提（见 docs/*-audit.md）：
//!
//! - **sqlite 是「可配置但没人能配置」**：前端全量搜索 `store_kind` 零命中，
//!   默认值恒为 `file`；它还不支持子会话清单（有测试专门锁死这条限制），
//!   并且为了放压缩存档仍然要在磁盘上开一个 `<root>/<safe_id>/` 目录——
//!   所谓「第二种后端」既没换来性能，也没换来独立。
//! - **memory 不是后端，是「要不要持久化」**：它的每一处语义都是逐条对齐文件
//!   后端写出来的（load 缺省新建、save upsert、list 按 updated_at 降序），
//!   存在的唯一理由是让临时会话复用同一份会话引擎（审计 B1/B2）。
//!
//! 两者都不是「同一件事的第二种实现」，`dyn` 因此只是把一次构造换成一次查表。
//! 现在只剩一个具体类型，差异收成构造时的一次选型：
//!
//! | 驻留方式 | 构造 | 条目住在 |
//!|---|---|---|
//! | 持久会话 | [`SessionStore::new`]（根 = `<homedir>/plugins/session`） | 磁盘 `<根>/<id>/{session.json, messages.json}` |
//! | 临时会话 | [`SessionStore::ephemeral`] | 进程内，退出即丢 |
//!
//! ## 与 VDFS 的关系
//!
//! **对外**，会话早已只有 VDFS 一个入口（`.vdfs/session` 的清单 / `消息` /
//! `子会话` / `工作目录`，见 `super::plugin` 的 `impl VdfsProvider`）；本模块是
//! 那个 provider 下面的真相源。**对内**，它刻意**不**改用
//! [`vdfs_service`](crate::providers::vdfs_service) 的三种集中实现，理由是拓扑相反：
//!
//! - `DirVdfs` 的定义是「条目内部可下钻浏览」，会话一旦套上，`session.json`、
//!   `messages/`、`tool_archives/`、`transcripts/`、`sessions/` 会原样成为对外
//!   地址——把物理布局当公共契约。而会话要求 `<id>` 是叶子、内部只以人读语义段
//!   呈现（与 agent bundle 的 `提示词` / `技能` / `MCP` 同一口径，规范 §13.4）。
//! - 条目也不是文件字节：`<id>/消息/<mid>` 是从整份 `Session` 派生的视图，
//!   `append` / `replace` / `update` 的 seq 分配与剔孤儿是会话专有的写入语义
//!   （在 `super::chat_session`）。
//!
//! 真正共用的是**寻址**：类别根取宿主层的 [`category_dir`]、id→段名取
//! [`safe_segment`]（经 [`super::paths::safe_id`]），所以会话目录名与资源条目目录名
//! 永远是同一份规则。这与规范 §13.4 里「目录自管的类型自己落盘，不经 vdfs_service」
//! 是同一条判据——agent bundle 是先例。
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
//! 曾经消息**内联**在 `session.json` 里，于是「列一次清单」= 读出并解析
//! **所有会话的全部历史**——会话越多、聊得越久越慢，且与有界窗口无关
//! （窗口只减少读几个文件，不减少每个文件的大小）。
//!
//! 分开之后清单只读 `session.json`，其大小与会话聊了多久**无关**；`messages.json`
//! 只有真的要取转写时才读。代价是清单需要的字段（标题 / 条数 / 摘要 / 标签）
//! 不能再从消息现算，必须在 `save` 时算好落盘——它们是**投影**
//! （[`super::types::SessionSummary`]），可重算，不是第二份真相。
//!
//! 旧布局（内联）仍**可读**：`messages.json` 缺失时回落到 `session.json` 里的
//! `messages` 字段；投影缺失时同样用内联消息**就地补算**
//! （[`SessionMetaFile::into_summary`]）——这是「存量文件没有投影字段」的兜底，
//! 缺了它清单会静默退化成显示 id。
//!
//! ### 子会话嵌套存储（机制约定，见 docs/design/vdfs.md）
//!
//! 会话可派生子会话：子会话不是顶层平级条目，而是存放在父会话目录内的
//! `sessions/` 子目录——`<根>/<safe(父)>/sessions/<safe(子)>/`。
//!
//! 归属声明与路由规则（调用方无感知）：
//! - **归属声明**：`metadata.parent_session_id`（父会话 id）。save 时据此
//!   路由到嵌套目录（无该元数据 = 顶层会话）；
//! - **load / delete / session_dir**：先查顶层，未命中则扫描各会话目录的
//!   `sessions/<id>/`（子会话可凭自身 id 直接寻址）；
//! - **list_sessions**：只列顶层（子会话不出现在顶层清单）；
//!   子会话清单走 [`list_sub_sessions`](SessionStore::list_sub_sessions)；
//! - **级联删除**：删除父会话 = `remove_dir_all(父目录)`，子会话随之删除。
//!
//! 子会话 id 由系统生成，与顶层 id 同一格式约束。临时会话无目录概念，
//! 因此不参与嵌套（`list_sub_sessions` 恒空、`session_dir` 恒 `None`）。

use super::types::{ChatMessage, Session, SessionSummary};
use crate::plugins::session::paths;
use crate::symbio_core::PluginError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tokio::sync::RwLock;

/// 父会话目录内存放子会话的固定子目录名
const SUB_SESSIONS_DIR: &str = "sessions";

/// 会话主文件名（一个会话目录内的唯一真源）
const SESSION_FILE: &str = "session.json";

/// 会话主文件的原子写临时文件名
const SESSION_FILE_TMP: &str = "session.json.tmp";

/// 消息文件名 —— **与元数据分开存放**（见模块头「元数据与消息分文件」）。
/// 清单只读 `session.json`，**不读**本文件；只有真的要取转写时才读。
const MESSAGES_FILE: &str = "messages.json";

/// 消息文件的原子写临时文件名
const MESSAGES_FILE_TMP: &str = "messages.json.tmp";

/// 会话存储：一个具体类型，落盘与不落盘由构造时选定（见模块头）
pub struct SessionStore {
    /// 落盘根；`None` = 不落盘（临时会话），条目只活在 `mem` 里
    base_dir: Option<PathBuf>,
    /// 不落盘时的进程内条目表（落盘型恒空）
    mem: RwLock<BTreeMap<String, Session>>,
}

impl SessionStore {
    /// 持久会话存储；`base_dir` 通常是
    /// [`SessionPlugin::session_storage_dir`](super::plugin::SessionPlugin::session_storage_dir)
    /// （即宿主层的 `<homedir>/plugins/session` 类别根），测试注入临时目录。
    pub fn new(base_dir: PathBuf) -> Self {
        Self {
            base_dir: Some(base_dir),
            mem: RwLock::new(BTreeMap::new()),
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
        }
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

        // 元数据与消息**分两个文件**落盘：清单只读 `session.json`（恒定小成本），
        // `messages.json` 只有真的要取转写时才读——这是清单与消息量脱钩的关键。
        //
        // 顺序是**先消息、后元数据**（先数据后索引）：中途崩溃最坏情况是元数据
        // 落后一版，不会出现「索引指向没有的消息」。
        //
        // 原子写（两个文件各一份 tmp + rename）：直接 fs::write（O_TRUNC + write）
        // 在多个并发保存交错时会留下"短 JSON + 长旧内容残留"，产生 trailing
        // characters 损坏；rename 覆盖保证磁盘上永远是某一刻的完整版本。
        let summary = SessionSummary::of(session);
        atomic_write(
            &dir.join(MESSAGES_FILE),
            &dir.join(MESSAGES_FILE_TMP),
            &MessagesFile {
                messages: session.messages.clone(),
            },
        )
        .await?;
        atomic_write(
            &dir.join(SESSION_FILE),
            &dir.join(SESSION_FILE_TMP),
            &SessionMetaFile::of(&summary),
        )
        .await
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

    /// 列出所有**顶层**会话的摘要（按 `updated_at` 降序；子会话不在其中）。
    ///
    /// 返回值**不含消息**——类型上就带不走（见 `SessionSummary`）。这是清单
    /// 与消息量脱钩的关键：读一次清单的成本与「会话聊了多久」无关。
    pub async fn list_sessions(&self) -> Result<Vec<SessionSummary>, PluginError> {
        self.list_sessions_window(None, None).await
    }

    /// 有界清单：最多 `limit` 条，从游标 `before`（某条会话 id）之后继续。
    ///
    /// `limit` 计量单位是**清单项**（一个会话 = 一项），与转写窗口的「根节点」
    /// 口径不同——这里没有父子结构，一项就是一项。`None` = 不截断。
    ///
    /// 排序与截断都在内存里做：窗口约束的是**响应大小**（少传多少节点给前端），
    /// 而 IO 成本已经由「清单只读 `session.json`」压平了——再引入一份索引文件
    /// 换来的收益不值一个新文件（见 docs/perf.md §3）。
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
    pub async fn list_sub_sessions(&self, parent_id: &str) -> Result<Vec<SessionSummary>, PluginError> {
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
    /// 少了它清单会静默退化成显示 id（见 docs/perf.md §11.3）。
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
    messages: Option<Vec<ChatMessage>>,
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
    fn into_session(self, messages: Vec<ChatMessage>) -> Session {
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
    messages: Vec<ChatMessage>,
}

/// 原子写：先落 tmp 再 rename。
///
/// 直接 `fs::write`（O_TRUNC + write）在并发保存交错时会留下「短 JSON + 长旧内容
/// 残留」，产生 trailing characters 损坏；rename 保证磁盘上永远是某一刻的完整版本。
async fn atomic_write<T: Serialize>(
    target: &Path,
    tmp: &Path,
    value: &T,
) -> Result<(), PluginError> {
    let json = serde_json::to_string(value)
        .map_err(|e| PluginError::InternalError(format!("序列化会话文件失败: {e}")))?;
    tokio::fs::write(tmp, json)
        .await
        .map_err(|e| PluginError::InternalError(format!("写入会话临时文件失败: {e}")))?;
    tokio::fs::rename(tmp, target)
        .await
        .map_err(|e| PluginError::InternalError(format!("替换会话文件失败: {e}")))
}

/// 解析 session.json 内容；存在尾部残留时截取首个完整 JSON 自愈
/// （流式反序列化取首个完整对象），完全无法解析时返回 None。
fn parse_meta_content(content: &str) -> Option<SessionMetaFile> {
    parse_or_self_heal::<SessionMetaFile>(content)
}

fn parse_messages_content(content: &str) -> Option<Vec<ChatMessage>> {
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

/// 读 `messages.json`；缺失或不可解析时回落到元数据里的内联消息
/// （旧布局），两者都没有则空。
async fn load_messages(dir: &Path, meta: &SessionMetaFile) -> Vec<ChatMessage> {
    let path = dir.join(MESSAGES_FILE);
    if let Ok(content) = tokio::fs::read_to_string(&path).await {
        if let Some(msgs) = parse_messages_content(&content) {
            return msgs;
        }
    }
    meta.messages.clone().unwrap_or_default()
}

/// 同步版读 `messages.json`
fn read_messages_sync(dir: &Path) -> Option<Vec<ChatMessage>> {
    let content = std::fs::read_to_string(dir.join(MESSAGES_FILE)).ok()?;
    parse_messages_content(&content)
}

/// 读 `session.json`（同步；用于 spawn_blocking 里的批量读取）
fn read_meta(dir: &Path) -> Option<SessionMetaFile> {
    let content = std::fs::read_to_string(dir.join(SESSION_FILE)).ok()?;
    parse_meta_content(&content)
}

/// 读整个会话目录 = 元数据 + 消息（同步；嵌套查找的 miss 路径用）
fn read_session_dir(dir: &Path) -> Option<Session> {
    let meta = read_meta(dir)?;
    // 两个来源都没有消息 = 空会话（不是「不存在」），别把「没有消息」当成「不存在」
    let messages = read_messages_sync(dir)
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
async fn split_inline_messages(dir: &Path) -> Result<(), PluginError> {
    let Some(meta) = read_meta(dir) else {
        return Ok(());
    };
    let has_split = dir.join(MESSAGES_FILE).is_file();
    let inline = meta.messages.clone();
    // 已是新布局、投影齐全、也没有残留的内联消息 → 无需改动
    if has_split && inline.is_none() && !meta.title.is_empty() {
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

    if !has_split {
        atomic_write(
            &dir.join(MESSAGES_FILE),
            &dir.join(MESSAGES_FILE_TMP),
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
    atomic_write(
        &dir.join(SESSION_FILE),
        &dir.join(SESSION_FILE_TMP),
        &SessionMetaFile::of(&summary),
    )
    .await
}

/// 清单排序：`updated_at` 降序（两种驻留方式共用同一份，清单顺序不分叉）
fn sort_summaries_desc(summaries: &mut [SessionSummary]) {
    summaries.sort_by_key(|b| std::cmp::Reverse(b.updated_at));
}

#[cfg(test)]
mod tests;
