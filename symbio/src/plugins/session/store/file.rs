//! 文件系统存储后端
//!
//! 顶层会话：`<base_dir>/<safe_id>/session.json`
//! 消息压缩存档写入：`<base_dir>/<safe_id>/messages/msg_*.txt`
//!
//! ## 子会话嵌套存储（机制约定，见 docs/design/vdfs.md）
//!
//! 会话可派生子会话：子会话不是顶层平级实体，而是存放在父会话目录内的
//! `sessions/` 子目录——`<base_dir>/<safe(父)>/sessions/<safe(子)>/`。
//!
//! 归属声明与路由规则（机制级，调用方无感知）：
//! - **归属声明**：`metadata.parent_session_id`（父会话 id）。save 时据此
//!   路由到嵌套目录（无该元数据 = 顶层会话）；
//! - **load / delete / session_dir**：先查顶层，未命中则扫描各会话目录的
//!   `sessions/<id>/`（子会话可凭自身 id 直接寻址）；
//! - **list_sessions**：只列顶层（子会话不出现在顶层清单）；
//!   子会话清单走 `list_sub_sessions(parent)`；
//! - **级联删除**：删除父会话 = `remove_dir_all(父目录)`，子会话随之删除。
//!
//! 目录名安全化沿用 `safe_id`（`/ \ :` → `_`）；子会话 id 由系统生成，
//! 与顶层 id 同一格式约束。

use super::super::paths::safe_id;
use super::SessionStore;
use crate::plugins::session::types::Session;
use crate::symbio_core::PluginError;
use async_trait::async_trait;
use std::path::{Path, PathBuf};

// FileSessionStore

pub struct FileSessionStore {
    base_dir: PathBuf,
}

/// 父会话目录内存放子会话的固定子目录名
const SUB_SESSIONS_DIR: &str = "sessions";

/// 会话主文件名（一个会话目录内的唯一真源）
const SESSION_FILE: &str = "session.json";

/// 会话主文件的原子写临时文件名
const SESSION_FILE_TMP: &str = "session.json.tmp";

impl FileSessionStore {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    /// `<base_dir>/<safe_id>/`
    ///
    /// safe_id 的唯一实现在 [`super::super::paths::safe_id`]（历史上有 4 处
    /// 重复实现，已收敛；此处 base_dir 由外部注入，仅复用 id→目录名映射）。
    pub fn dir_for(base_dir: &Path, session_id: &str) -> PathBuf {
        base_dir.join(safe_id(session_id))
    }

    /// `<base_dir>/<safe_id>/session.json`
    fn file_for(base_dir: &Path, session_id: &str) -> PathBuf {
        Self::dir_for(base_dir, session_id).join(SESSION_FILE)
    }

    /// 子会话目录：`<base_dir>/<safe(父)>/sessions/<safe(子)>/`
    fn sub_dir_for(base_dir: &Path, parent_id: &str, session_id: &str) -> PathBuf {
        base_dir
            .join(safe_id(parent_id))
            .join(SUB_SESSIONS_DIR)
            .join(safe_id(session_id))
    }

    /// 归属父会话 id（metadata.parent_session_id；空/自引用视为无归属）
    fn parent_of(session: &Session) -> Option<String> {
        session.parent_session_id().map(str::to_string)
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
        Self::parse_session_content(&content)
    }

    /// 嵌套查找：在所有顶层会话目录的 `sessions/<safe_id>/` 中定位子会话，
    /// 返回其目录。仅在顶层未命中时调用（miss 路径，开销可接受）。
    fn find_nested_dir(&self, session_id: &str) -> Option<PathBuf> {
        let entries = std::fs::read_dir(&self.base_dir).ok()?;
        for entry in entries.flatten() {
            let dir = entry
                .path()
                .join(SUB_SESSIONS_DIR)
                .join(safe_id(session_id));
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
                if let Some(s) = Self::read_session_file(&path.join(SESSION_FILE)) {
                    sessions.push(s);
                }
            }
        }
        sessions
    }
}

#[async_trait]
impl SessionStore for FileSessionStore {
    async fn load_session(&self, session_id: &str) -> Result<Session, PluginError> {
        let path = Self::file_for(&self.base_dir, session_id);
        if path.exists() {
            let content = tokio::fs::read_to_string(&path)
                .await
                .map_err(|e| PluginError::InternalError(format!("读取会话文件失败: {e}")))?;
            return match Self::parse_session_content(&content) {
                Some(s) => Ok(s),
                None => Err(PluginError::ParseError(
                    "解析会话失败: 内容无法恢复".to_string(),
                )),
            };
        }
        // 顶层未命中：嵌套查找（子会话凭自身 id 直接寻址）
        if let Some(dir) = self.find_nested_dir(session_id) {
            if let Some(s) = Self::read_session_file(&dir.join(SESSION_FILE)) {
                return Ok(s);
            }
        }
        Ok(Session::new(session_id))
    }

    async fn save_session(&self, session: &Session) -> Result<(), PluginError> {
        // 归属路由：声明了 parent_session_id 的会话存入父目录的 sessions/ 子目录
        let dir = match Self::parent_of(session) {
            Some(parent) => Self::sub_dir_for(&self.base_dir, &parent, &session.id),
            None => Self::dir_for(&self.base_dir, &session.id),
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

    async fn delete_session(&self, session_id: &str) -> Result<(), PluginError> {
        let dir = Self::dir_for(&self.base_dir, session_id);
        if dir.exists() {
            tokio::fs::remove_dir_all(&dir)
                .await
                .map_err(|e| PluginError::InternalError(format!("删除会话目录失败: {e}")))?;
            return Ok(());
        }
        // 顶层未命中：嵌套删除（子会话级联于父目录之外的单点删除）
        if let Some(nested) = self.find_nested_dir(session_id) {
            tokio::fs::remove_dir_all(&nested)
                .await
                .map_err(|e| PluginError::InternalError(format!("删除会话目录失败: {e}")))?;
        }
        Ok(())
    }

    async fn list_sessions(&self) -> Result<Vec<Session>, PluginError> {
        if !self.base_dir.exists() {
            return Ok(Vec::new());
        }

        let mut sessions = Vec::new();
        let mut entries = tokio::fs::read_dir(&self.base_dir)
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
                if let Some(session) = Self::parse_session_content(&content) {
                    sessions.push(session);
                }
            }
        }

        sessions.sort_by_key(|b| std::cmp::Reverse(b.updated_at));
        Ok(sessions)
    }

    /// 子会话清单：`<base>/<safe(父)>/sessions/*/session.json`，updated_at 降序
    async fn list_sub_sessions(&self, parent_id: &str) -> Result<Vec<Session>, PluginError> {
        let nested = Self::dir_for(&self.base_dir, parent_id).join(SUB_SESSIONS_DIR);
        if !nested.exists() {
            return Ok(Vec::new());
        }
        let mut sessions = tokio::task::spawn_blocking(move || Self::read_sessions_in(&nested))
            .await
            .map_err(|e| PluginError::InternalError(e.to_string()))?;
        sessions.sort_by_key(|b| std::cmp::Reverse(b.updated_at));
        Ok(sessions)
    }

    fn session_dir(&self, session_id: &str) -> Option<PathBuf> {
        let top = Self::dir_for(&self.base_dir, session_id);
        if top.exists() {
            return Some(top);
        }
        self.find_nested_dir(session_id)
    }
}

#[cfg(test)]
mod tests;
