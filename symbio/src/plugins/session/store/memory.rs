//! 内存存储后端 —— [`SessionStore`] 契约的零持久化实现。
//!
//! 存在意义（审计 B2）：**会话引擎只保留 [`PersistentChatSession`] 一份实现**，
//! "要不要持久化"这个差异下沉到存储后端。原先的 `EphemeralChatSession` 是同一
//! 契约的第三份手写实现，其 `append_messages`（seq 分配 + 轮次 FIFO）与
//! `get_context_messages`（剔孤儿 + content 归一 + 滑动窗口）与持久化版本各写
//! 一遍，任何语义调整都要改两处——本后端让两者共享同一份逻辑。
//!
//! 消费方：
//! - `_t_` 前缀 / 空 `session_id` 的临时会话（`handlers::open_session_handle`）；
//! - 编排器未交付会话句柄时的兜底会话（`chat_loop::open_chat_session`）。
//!
//! 语义要点：
//! - [`load_session`](SessionStore::load_session) 命中返回**克隆**，调用方在副本上
//!   改写后经 [`save_session`](SessionStore::save_session) 整体替换——与文件后端
//!   "读文件→改→写回"的语义逐字对齐，因此上层逻辑无需为本后端特判。
//! - 无目录概念，`session_dir` 返回 `None`（与 SQLite 后端一致）：存档路径解析
//!   退化为"无存档"，符合临时会话不回看历史的定位。
//! - 子会话归属无意义：沿用 trait 默认空实现。

use super::SessionStore;
use crate::plugins::session::types::Session;
use crate::symbio_core::PluginError;
use async_trait::async_trait;
use std::collections::BTreeMap;
use std::path::PathBuf;
use tokio::sync::RwLock;

/// 内存会话存储：`session_id -> Session` 的进程内映射，进程退出即丢失。
#[derive(Default)]
pub struct InMemorySessionStore {
    sessions: RwLock<BTreeMap<String, Session>>,
}

impl InMemorySessionStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl SessionStore for InMemorySessionStore {
    async fn load_session(&self, session_id: &str) -> Result<Session, PluginError> {
        match self.sessions.read().await.get(session_id) {
            Some(session) => Ok(session.clone()),
            // 与文件后端一致：不存在时返回新建的空 Session（不落库，
            // 由调用方后续 save_session 决定是否创建）。
            None => Ok(Session::new(session_id)),
        }
    }

    async fn save_session(&self, session: &Session) -> Result<(), PluginError> {
        self.sessions
            .write()
            .await
            .insert(session.id.clone(), session.clone());
        Ok(())
    }

    async fn delete_session(&self, session_id: &str) -> Result<(), PluginError> {
        self.sessions.write().await.remove(session_id);
        Ok(())
    }

    async fn list_sessions(&self) -> Result<Vec<Session>, PluginError> {
        let mut all: Vec<Session> = self.sessions.read().await.values().cloned().collect();
        // 与文件 / SQLite 后端一致：按 updated_at 降序
        all.sort_by_key(|s| std::cmp::Reverse(s.updated_at));
        Ok(all)
    }

    fn session_dir(&self, _session_id: &str) -> Option<PathBuf> {
        None
    }
}

#[cfg(test)]
mod tests;
