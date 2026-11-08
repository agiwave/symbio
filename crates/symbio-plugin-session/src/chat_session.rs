//! 会话引擎（[`PersistentChatSession`]，唯一实现）。
//!
//! 会话只有磁盘一种驻留（D-329）：无事实源的临时会话壳已退场——它既读不到历史，
//! 也落不进格。存储的构造见 `super::store::SessionStore::new`。
//!
//! ## 模块分工
//!
//! | 文件 | 职责 |
//! |---|---|
//! | 本文件 | 引擎本体：结构与构造、配置读取 |
//! | [`read`]  | 读路径：存储视图 |
//! | [`write`] | 写路径：两条写入路径 + 其策略纯函数（序号分配 / 时间戳回填 / 写入不变量 / 存储期裁剪） |
//!
//! 读写各自的纯函数与自己的路径同处一文件，便于对照。
//!
//! ## 为何归属 session 插件
//!
//! `PersistentChatSession` 的读写方全部在 session 插件内，不存在跨插件使用，
//! 故从旧 core 下沉到本插件——核心架构只保留跨插件共享的抽象，
//! 不承载单一模块的内部定义。
//!
//! 不设 `ChatSession` trait（唯一实现即本结构体）：单实现多态只会
//! 多一层 `dyn` 分发与一份契约文档的两处维护，已去除——方法直接内聚在实现上。

use super::config::SessionConfig;
use super::store::SessionStore;
use std::collections::HashSet;
use std::sync::Arc;
use symbio_kernel::plugin::PluginError;
use symbio_sdk::chat::chat_node as cm;
use symbio_sdk::chat::chat_node::{ChatNode, NodeRole};
use symbio_sdk::clock::clock_now_ms;
use tokio::sync::RwLock;

mod read;
mod write;

// 变更原语是**唯一写入口** `write::apply` 的入参类型——它们的消费方在 `session`
// 域的各个写点，所以从这里重导出（`mod write` 本身保持私有：两个存储原语
// `append_messages` / `replace_all` 不得被 `write.rs` 之外直接调用，可见性即守卫）。
pub(crate) use write::{Carrier, MessageChange};

// PersistentChatSession

pub struct PersistentChatSession {
    session_id: String,
    config: Arc<RwLock<SessionConfig>>,
    store: Arc<SessionStore>,
}

impl PersistentChatSession {
    pub fn new(
        session_id: String,
        config: Arc<RwLock<SessionConfig>>,
        store: Arc<SessionStore>,
    ) -> Self {
        Self {
            session_id,
            config,
            store,
        }
    }

    /// 读当前配置；锁被占时回落到 `SessionConfig::default()`。
    ///
    /// 不再手写魔数兜底（历史上是 `unwrap_or(200)` / `unwrap_or(3)`，与默认值三源）：
    /// 默认值只存在于 `SessionConfig` 一处。
    pub(crate) fn cfg_or_default(&self) -> SessionConfig {
        self.config
            .try_read()
            .map(|c| c.clone())
            .unwrap_or_default()
    }

    async fn load_session(&self) -> Result<super::types::Session, PluginError> {
        self.store.load_session(&self.session_id).await
    }

    async fn save_session(&self, session: &super::types::Session) -> Result<(), PluginError> {
        self.store.save_session(session).await
    }
}

/// 委派链上的**父主体**——三段取值（子 `metadata.parent_session_id` → 父
/// `metadata.agent_id` → [`symbio_sdk::authz::principal_of`]）的**唯一实现**。
///
/// [`PersistentChatSession::delegating_principal`] 是运行期读面；会话**创建**时
/// （`commands::session_upsert`）也要用同一份判据登记主体行，两处
/// 各写一遍迟早分叉成「登记挂 A、运行期判 B」。
pub(super) async fn delegating_principal_in(
    store: &SessionStore,
    session: &super::types::Session,
) -> Option<String> {
    let parent_id = session.parent_session_id()?;
    let parent = store.load_session(parent_id).await.ok()?;
    let agent = parent.metadata.get("agent_id").and_then(|v| v.as_str());
    Some(symbio_sdk::authz::principal_of(agent))
}

#[cfg(test)]
#[path = "chat_session.test.rs"]
mod tests;
