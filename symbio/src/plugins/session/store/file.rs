//! 文件系统存储后端
//!
//! 每个 session 对应一个子目录：`<base_dir>/<safe_id>/session.json`
//! 消息压缩存档写入：`<base_dir>/<safe_id>/messages/msg_*.txt`

use super::SessionStore;
use crate::plugins::session::types::Session;
use crate::symbio_core::PluginError;
use async_trait::async_trait;
use std::path::{Path, PathBuf};

// FileSessionStore

pub struct FileSessionStore {
    base_dir: PathBuf,
}

impl FileSessionStore {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    /// 将 session_id 转换为安全的目录名
    fn safe_id(session_id: &str) -> String {
        session_id.replace(['/', '\\', ':'], "_")
    }

    /// `<base_dir>/<safe_id>/`
    pub fn dir_for(base_dir: &Path, session_id: &str) -> PathBuf {
        base_dir.join(Self::safe_id(session_id))
    }

    /// `<base_dir>/<safe_id>/session.json`
    fn file_for(base_dir: &Path, session_id: &str) -> PathBuf {
        Self::dir_for(base_dir, session_id).join("session.json")
    }

    /// 解析 session.json 内容；存在尾部残留时截取首个完整 JSON 自愈。
    ///
    /// 历史缺陷：旧的 `fs::write` 直写方式在并发保存交错时会留下
    /// "短 JSON + 长旧内容残留"（trailing characters）。这里用流式反序列化
    /// 取首个完整对象，尽量恢复旧损坏文件；完全无法解析时返回 None。
    fn parse_session_content(content: &str) -> Option<Session> {
        match serde_json::from_str(content) {
            Ok(s) => Some(s),
            Err(_) => serde_json::Deserializer::from_str(content)
                .into_iter::<Session>()
                .next()
                .and_then(Result::ok),
        }
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
            match Self::parse_session_content(&content) {
                Some(s) => Ok(s),
                None => Err(PluginError::ParseError(
                    "解析会话失败: 内容无法恢复".to_string(),
                )),
            }
        } else {
            Ok(Session::new(session_id))
        }
    }

    async fn save_session(&self, session: &Session) -> Result<(), PluginError> {
        let dir = Self::dir_for(&self.base_dir, &session.id);
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|e| PluginError::InternalError(format!("创建会话目录失败: {e}")))?;

        let path = dir.join("session.json");
        let content = serde_json::to_string_pretty(session)
            .map_err(|e| PluginError::InternalError(format!("序列化会话失败: {e}")))?;

        // 原子写：先写临时文件再 rename 覆盖。直接 fs::write（O_TRUNC + write）
        // 在多个并发保存交错时会留下"短 JSON + 长旧内容残留"，产生 trailing
        // characters 损坏；rename 覆盖保证磁盘上永远是某一刻的完整版本。
        let tmp = dir.join("session.json.tmp");
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
            let session_file = path.join("session.json");
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

    fn session_dir(&self, session_id: &str) -> Option<PathBuf> {
        Some(Self::dir_for(&self.base_dir, session_id))
    }
}
