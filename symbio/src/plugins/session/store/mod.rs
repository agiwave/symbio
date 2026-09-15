//! Session 存储 —— **一种磁盘布局，两种驻留方式**
//!
//! ## 它不是什么
//!
//! 这里过去是「一个 trait + 三个后端」：`FileSessionStore` / `SqliteSessionStore`
//! / `InMemorySessionStore`，由配置项 `store_kind` 经 `create_store` 工厂选型。
//! 那个形状有两个已证伪的前提（见 docs/architecture/session-*-audit.md）：
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
//! | 持久会话 | [`SessionStore::new`]（根 = `<homedir>/plugins/session`） | 磁盘 `<根>/<id>/session.json` |
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
//! - 条目也不是文件字节：消息**内联**在 `session.json` 里，`<id>/消息/<mid>`
//!   是从整份 `Session` 派生的视图，`append` / `replace` / `update` 的 seq 分配
//!   与剔孤儿是会话专有的写入语义（在 `super::chat_session`）。
//!
//! 真正共用的是**寻址**：类别根取宿主层的 [`category_dir`]、id→段名取
//! [`safe_segment`]（经 [`super::paths::safe_id`]），所以会话目录名与资源条目目录名
//! 永远是同一份规则。这与规范 §13.4 里「目录自管的类型自己落盘，不经 vdfs_service」
//! 是同一条判据——agent bundle 是先例。
//!
//! ## 磁盘布局
//!
//! 顶层会话：`<根>/<safe_id>/session.json`
//! 消息压缩存档：`<根>/<safe_id>/messages/msg_*.txt`
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

use super::types::Session;
use crate::plugins::session::paths;
use crate::symbio_core::PluginError;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use tokio::sync::RwLock;

/// 父会话目录内存放子会话的固定子目录名
const SUB_SESSIONS_DIR: &str = "sessions";

/// 会话主文件名（一个会话目录内的唯一真源）
const SESSION_FILE: &str = "session.json";

/// 会话主文件的原子写临时文件名
const SESSION_FILE_TMP: &str = "session.json.tmp";

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
        let Some(base) = self.disk() else {
            return match self.mem.read().await.get(session_id) {
                Some(session) => Ok(session.clone()),
                None => Ok(Session::new(session_id)),
            };
        };

        let path = file_for(base, session_id);
        if path.exists() {
            let content = tokio::fs::read_to_string(&path)
                .await
                .map_err(|e| PluginError::InternalError(format!("读取会话文件失败: {e}")))?;
            return match parse_session_content(&content) {
                Some(s) => Ok(s),
                None => Err(PluginError::ParseError(
                    "解析会话失败: 内容无法恢复".to_string(),
                )),
            };
        }
        // 顶层未命中：嵌套查找（子会话凭自身 id 直接寻址）
        if let Some(dir) = find_nested_dir(base, session_id) {
            if let Some(s) = read_session_file(&dir.join(SESSION_FILE)) {
                return Ok(s);
            }
        }
        Ok(Session::new(session_id))
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

        let path = dir.join(SESSION_FILE);
        let content = serde_json::to_string_pretty(session)
            .map_err(|e| PluginError::InternalError(format!("序列化会话失败: {e}")))?;

        // 原子写：先写临时文件再 rename 覆盖。直接 fs::write（O_TRUNC + write）
        // 在多个并发保存交错时会留下"短 JSON + 长旧内容残留"，产生 trailing
        // characters 损坏；rename 覆盖保证磁盘上永远是某一刻的完整版本。
        let tmp = dir.join(SESSION_FILE_TMP);
        tokio::fs::write(&tmp, &content)
            .await
            .map_err(|e| PluginError::InternalError(format!("写入会话临时文件失败: {e}")))?;
        tokio::fs::rename(&tmp, &path)
            .await
            .map_err(|e| PluginError::InternalError(format!("落盘会话文件失败: {e}")))
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

    /// 列出所有**顶层**会话（按 `updated_at` 降序；子会话不在其中）
    pub async fn list_sessions(&self) -> Result<Vec<Session>, PluginError> {
        let Some(base) = self.disk() else {
            let mut all: Vec<Session> = self.mem.read().await.values().cloned().collect();
            sort_by_updated_desc(&mut all);
            return Ok(all);
        };

        if !base.exists() {
            return Ok(Vec::new());
        }

        let mut sessions = Vec::new();
        let mut entries = tokio::fs::read_dir(base)
            .await
            .map_err(|e| PluginError::InternalError(format!("读取存储目录失败: {e}")))?;

        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| PluginError::InternalError(e.to_string()))?
        {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let session_file = path.join(SESSION_FILE);
            if !session_file.exists() {
                continue;
            }
            if let Ok(content) = tokio::fs::read_to_string(&session_file).await {
                if let Some(session) = parse_session_content(&content) {
                    sessions.push(session);
                }
            }
        }

        sort_by_updated_desc(&mut sessions);
        Ok(sessions)
    }

    /// 子会话清单：`<根>/<safe(父)>/sessions/*/session.json`，`updated_at` 降序。
    /// 不落盘的临时会话恒空（无目录概念）。
    pub async fn list_sub_sessions(&self, parent_id: &str) -> Result<Vec<Session>, PluginError> {
        let Some(base) = self.disk() else {
            return Ok(Vec::new());
        };
        let nested = dir_for(base, parent_id).join(SUB_SESSIONS_DIR);
        if !nested.exists() {
            return Ok(Vec::new());
        }
        let mut sessions = tokio::task::spawn_blocking(move || read_sessions_in(&nested))
            .await
            .map_err(|e| PluginError::InternalError(e.to_string()))?;
        sort_by_updated_desc(&mut sessions);
        Ok(sessions)
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

/// 解析 session.json 内容；存在尾部残留时截取首个完整 JSON 自愈
/// （流式反序列化取首个完整对象），完全无法解析时返回 None。
fn parse_session_content(content: &str) -> Option<Session> {
    match serde_json::from_str(content) {
        Ok(s) => Some(s),
        Err(_) => serde_json::Deserializer::from_str(content)
            .into_iter::<Session>()
            .next()
            .and_then(Result::ok),
    }
}

/// 读取单个 session.json（存在且可解析时返回 Session）
fn read_session_file(path: &Path) -> Option<Session> {
    let content = std::fs::read_to_string(path).ok()?;
    parse_session_content(&content)
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

/// 列出某目录下所有 `<dir>/session.json` 会话（跳过不可解析项）
fn read_sessions_in(dir: &Path) -> Vec<Session> {
    let mut sessions = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return sessions;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(s) = read_session_file(&path.join(SESSION_FILE)) {
                sessions.push(s);
            }
        }
    }
    sessions
}

/// 清单排序：`updated_at` 降序（两种驻留方式共用同一份，清单顺序不分叉）
fn sort_by_updated_desc(sessions: &mut Vec<Session>) {
    sessions.sort_by_key(|b| std::cmp::Reverse(b.updated_at));
}

#[cfg(test)]
mod tests;
