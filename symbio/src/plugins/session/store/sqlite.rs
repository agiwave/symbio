//! SQLite 存储后端 (基于 tokio-rusqlite)
//!
//! 所有 session 存储在 `<base_dir>/sessions.db` 的 SQLite 数据库中。
//! 表结构：
//!   sessions(id TEXT PRIMARY KEY, data TEXT NOT NULL, updated_at INTEGER NOT NULL)

use super::SessionStore;
use crate::plugins::session::types::Session;
use crate::symbio_core::PluginError;
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use tokio_rusqlite::Connection;

// SqliteSessionStore

pub struct SqliteSessionStore {
    conn: Connection,
    /// 数据库所在的基础目录（用于消息压缩存档路径）
    base_dir: PathBuf,
}

impl SqliteSessionStore {
    /// 打开（或创建）SQLite 数据库并初始化表结构。
    pub async fn open(base_dir: PathBuf) -> Result<Self, PluginError> {
        tokio::fs::create_dir_all(&base_dir)
            .await
            .map_err(|e| PluginError::InternalError(format!("创建数据库目录失败: {e}")))?;

        let db_path = base_dir.join("sessions.db");
        let conn = Connection::open(&db_path)
            .await
            .map_err(|e| PluginError::InternalError(format!("连接 SQLite 数据库失败: {e}")))?;

        // 初始化表结构
        conn.call(|c| {
            c.execute(
                "CREATE TABLE IF NOT EXISTS sessions (
                    id         TEXT    NOT NULL PRIMARY KEY,
                    data       TEXT    NOT NULL,
                    updated_at INTEGER NOT NULL DEFAULT 0
                )",
                [],
            )
        })
        .await
        .map_err(|e| PluginError::InternalError(format!("初始化数据库表失败: {e}")))?;

        Ok(Self { conn, base_dir })
    }

    /// 消息压缩存档目录（与文件后端保持一致的路径规范）
    fn archive_dir_for(base_dir: &Path, session_id: &str) -> PathBuf {
        base_dir.join(super::super::paths::safe_id(session_id))
    }
}

#[async_trait]
impl SessionStore for SqliteSessionStore {
    async fn load_session(&self, session_id: &str) -> Result<Session, PluginError> {
        let id_for_query = session_id.to_string();
        let id_for_fallback = session_id.to_string();

        let data_str: Option<String> = self
            .conn
            .call::<_, _, rusqlite::Error>(move |c| {
                let mut stmt = c.prepare("SELECT data FROM sessions WHERE id = ?")?;
                let res = stmt.query_row([id_for_query], |row| row.get(0)).ok();
                Ok(res)
            })
            .await
            .map_err(|e| PluginError::InternalError(format!("查询会话失败: {e}")))?;

        match data_str {
            Some(data) => serde_json::from_str(&data)
                .map_err(|e| PluginError::ParseError(format!("解析会话数据失败: {e}"))),
            None => Ok(Session::new(&id_for_fallback)),
        }
    }

    async fn save_session(&self, session: &Session) -> Result<(), PluginError> {
        let id = session.id.clone();
        let updated_at = session.updated_at;
        let data = serde_json::to_string(session)
            .map_err(|e| PluginError::InternalError(format!("序列化会话失败: {e}")))?;

        self.conn
            .call(move |c| {
                c.execute(
                    "INSERT INTO sessions (id, data, updated_at)
                 VALUES (?, ?, ?)
                 ON CONFLICT(id) DO UPDATE SET
                     data       = excluded.data,
                     updated_at = excluded.updated_at",
                    rusqlite::params![id, data, updated_at],
                )
            })
            .await
            .map_err(|e| PluginError::InternalError(format!("保存会话失败: {e}")))?;

        Ok(())
    }

    async fn delete_session(&self, session_id: &str) -> Result<(), PluginError> {
        let id = session_id.to_string();
        self.conn
            .call(move |c| c.execute("DELETE FROM sessions WHERE id = ?", [id]))
            .await
            .map_err(|e| PluginError::InternalError(format!("删除会话失败: {e}")))?;

        // 同时删除压缩存档目录（若存在）
        let archive_dir = Self::archive_dir_for(&self.base_dir, session_id);
        if archive_dir.exists() {
            let _ = tokio::fs::remove_dir_all(&archive_dir).await;
        }

        Ok(())
    }

    async fn list_sessions(&self) -> Result<Vec<Session>, PluginError> {
        let rows: Vec<String> = self
            .conn
            .call::<_, _, rusqlite::Error>(move |c| {
                let mut stmt = c.prepare("SELECT data FROM sessions ORDER BY updated_at DESC")?;
                let iter = stmt.query_map([], |row| row.get(0))?;
                let mut results = Vec::new();
                for s in iter.flatten() {
                    results.push(s);
                }
                Ok(results)
            })
            .await
            .map_err(|e| PluginError::InternalError(format!("列出会话失败: {e}")))?;

        let mut sessions = Vec::with_capacity(rows.len());
        for data in rows {
            match serde_json::from_str::<Session>(&data) {
                Ok(s) => sessions.push(s),
                Err(e) => {
                    tracing::warn!("跳过无效会话记录: {e}");
                }
            }
        }
        Ok(sessions)
    }

    fn session_dir(&self, session_id: &str) -> Option<PathBuf> {
        Some(Self::archive_dir_for(&self.base_dir, session_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session_with(id: &str) -> Session {
        let mut s = Session::new(id);
        s.updated_at = 1000;
        s
    }

    /// SQLite 后端契约测试：与文件后端共享 `SessionStore` 语义（load 缺省新建、
    /// save 幂等 upsert、delete 连带存档目录、list 按 updated_at 降序）。
    #[tokio::test]
    async fn sqlite_store_roundtrip_contract() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = SqliteSessionStore::open(tmp.path().to_path_buf())
            .await
            .unwrap();

        // load 不存在的会话 → 新建空 Session（与文件后端一致，不报错）
        let fresh = store.load_session("v2_sess_missing").await.unwrap();
        assert_eq!(fresh.id, "v2_sess_missing");

        // save → load 往返
        let mut s = session_with("v2_sess_a");
        s.metadata["k"] = serde_json::json!("v");
        store.save_session(&s).await.unwrap();
        let loaded = store.load_session("v2_sess_a").await.unwrap();
        assert_eq!(loaded.id, s.id);
        assert_eq!(loaded.metadata["k"], "v");

        // 同 id 再存 → upsert 而非报错/重复
        s.updated_at = 2000;
        store.save_session(&s).await.unwrap();
        assert_eq!(store.list_sessions().await.unwrap().len(), 1);

        // list 按 updated_at 降序
        store
            .save_session(&session_with("v2_sess_b"))
            .await
            .unwrap(); // 1000
        let listed = store.list_sessions().await.unwrap();
        assert_eq!(listed[0].id, "v2_sess_a"); // 2000 在前
        assert_eq!(listed[1].id, "v2_sess_b");

        // delete：DB 行与压缩存档目录一并清除
        let archive = store.session_dir("v2_sess_a").unwrap();
        tokio::fs::create_dir_all(&archive).await.unwrap();
        store.delete_session("v2_sess_a").await.unwrap();
        assert!(store
            .load_session("v2_sess_a")
            .await
            .unwrap()
            .messages
            .is_empty());
        assert!(!archive.exists());
    }

    /// 已知限制（有意为之）：子会话嵌套存储仅文件后端承载，sqlite 后端
    /// `list_sub_sessions` 恒为空——`store_kind=sqlite` 配置项因此不承诺
    /// 子会话清单能力。该测试锁死这一行为，防止误以为已支持。
    #[tokio::test]
    async fn sqlite_backend_has_no_sub_session_listing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = SqliteSessionStore::open(tmp.path().to_path_buf())
            .await
            .unwrap();
        store
            .save_session(&session_with("v2_sess_p"))
            .await
            .unwrap();
        assert!(store
            .list_sub_sessions("v2_sess_p")
            .await
            .unwrap()
            .is_empty());
    }
}
