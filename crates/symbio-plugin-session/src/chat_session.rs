//! 会话引擎（[`PersistentChatSession`]，唯一实现）。
//!
//! "要不要持久化"的差异下沉到存储（`store::SessionStore` 的
//! `new` / `ephemeral` 两种构造）：内存态会话经 [`PersistentChatSession::detached`]
//! 构造，因此与持久会话共享同一套孤儿清理、轮次窗口与配置读取语义（审计 B1）。
//!
//! - 持久化实现委托 `super::store::SessionStore` 落库；不落盘面向 `_t_` 临时会话。
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
//! 故从 `symbio_core` 下沉到本插件——核心架构只保留跨插件共享的抽象，
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
use symbio_sdk::plugin_info;
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
    /// 写入期是否执行工具链物理裁剪（[`write::prune_historical_tool_calls`]）。
    ///
    /// 持久会话为 `true`（控制磁盘与节点树体积）；内存临时会话为 `false`——
    /// 临时会话本就不回看历史，物理裁剪只会让同一轮内可复用的工具链凭空消失。
    prune_on_write: bool,
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
            prune_on_write: true,
        }
    }

    /// 不落盘的临时会话：同一份引擎逻辑 + [`SessionStore::ephemeral`]，零持久化。
    ///
    /// 审计 B1 的收敛点——原先这里存在第二份手写实现 `EphemeralChatSession`
    /// （116 行），其 seq 分配、轮次 FIFO、剔孤儿、content 归一与持久版各写
    /// 一遍，任何语义调整都要改两处且已经出现行为漂移（持久版有 prune、内存
    /// 版没有；持久版配置动态读、内存版构造时快照）。"要不要持久化"是**存储**
    /// 的差异，不是会话引擎的差异，故下沉到 store 层。
    ///
    /// `session_id` 由调用方决定：`_t_` 临时会话用真实传入 id，兜底会话用固定
    /// `"ephemeral"`（随机 id 会让压缩前的 transcript 转存落到永不复现的目录名
    /// 下，成为无法关联的孤儿存档）。
    pub fn ephemeral(session_id: impl Into<String>, config: Arc<RwLock<SessionConfig>>) -> Self {
        Self {
            session_id: session_id.into(),
            config,
            store: Arc::new(SessionStore::ephemeral()),
            prune_on_write: false,
        }
    }

    /// 同 `ephemeral`，但配置以**值快照**传入（调用方无需自行构造
    /// `Arc<RwLock<_>>`）：内存会话不接 `session/config` 热更新，快照即其终生命周期的配置。
    pub fn detached(session_id: impl Into<String>, config: SessionConfig) -> Self {
        Self::ephemeral(session_id, Arc::new(RwLock::new(config)))
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
