//! Session 存储抽象层
//!
//! 定义 [`SessionStore`] trait，并提供工厂函数 [`create_store`]。
//! 调用方只需持有 `Arc<dyn SessionStore>`，无需感知底层存储策略。

mod file;
mod sqlite;

use super::types::Session;
pub use crate::symbio_core::schemas::session::session_config::StoreKind;
use crate::symbio_core::PluginError;
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;

// SessionStore Trait

/// 会话存储后端统一接口
#[async_trait]
pub trait SessionStore: Send + Sync {
    /// 加载指定 session；不存在时返回新建的空 Session
    async fn load_session(&self, session_id: &str) -> Result<Session, PluginError>;

    /// 持久化保存 session
    async fn save_session(&self, session: &Session) -> Result<(), PluginError>;

    /// 删除指定 session（含所有关联存档）
    async fn delete_session(&self, session_id: &str) -> Result<(), PluginError>;

    /// 列出所有 session（按 updated_at 降序）
    async fn list_sessions(&self) -> Result<Vec<Session>, PluginError>;

    /// 返回该 session 的本地目录（文件后端有效；SQLite 后端返回 None）。
    /// 用于消息压缩存档路径解析。
    fn session_dir(&self, session_id: &str) -> Option<PathBuf>;
}

// 工厂

/// 根据 `kind` 和基础目录创建对应的存储后端实例。
///
/// - `base_dir`: 存储根目录（文件后端用目录树；SQLite 后端在此目录下创建 `sessions.db`）
/// - `kind`: 后端类型
pub async fn create_store(
    base_dir: PathBuf,
    kind: StoreKind,
) -> Result<Arc<dyn SessionStore>, PluginError> {
    match kind {
        StoreKind::File => {
            let store = file::FileSessionStore::new(base_dir);
            Ok(Arc::new(store))
        },
        StoreKind::Sqlite => {
            let store = sqlite::SqliteSessionStore::open(base_dir).await?;
            Ok(Arc::new(store))
        },
    }
}
