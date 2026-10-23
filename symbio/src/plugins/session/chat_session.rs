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
//! | 本文件 | 引擎本体：句柄与 Key、结构与构造、配置读取 |
//! | [`read`]  | 读路径：存储视图与上下文候选集（滑动窗口 / 孤儿剔除 / content 归一） |
//! | [`write`] | 写路径：两条写入路径 + 其策略纯函数（序号分配 / 时间戳回填 / 写入不变量 / 存储期裁剪） |
//!
//! 读写各自的纯函数与自己的路径同处一文件，便于对照。
//!
//! ## 为何归属 session 插件
//!
//! `PersistentChatSession` / `ChatSessionHandle` / `SESSION_HANDLE` 的读写方全部
//! 在 session 插件内（编排器交付句柄 → chat_loop / resume / handlers 消费），
//! 不存在跨插件使用，故从 `symbio_core` 下沉到本插件——核心架构只保留跨插件
//! 共享的抽象，不承载单一模块的内部定义。
//!
//! 不设 `ChatSession` trait（唯一实现即本结构体）：单实现多态只会
//! 多一层 `dyn` 分发与一份契约文档的两处维护，已去除——方法直接内聚在实现上。

use super::config::{default_context_messages, SessionConfig};
use super::store::SessionStore;
use crate::plugin_info;
use crate::symbio_core::chat_node as cm;
use crate::symbio_core::chat_node::{ChatNode, MessageContent, NodeRole};
use crate::symbio_core::clock_now_ms;
use crate::symbio_core::{PluginError, SymbioKey};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

mod read;
mod write;

// 变更原语是**唯一写入口** `write::apply` 的入参类型——它们的消费方在 `session`
// 域的各个写点，所以从这里重导出（`mod write` 本身保持私有：两个存储原语
// `append_messages` / `replace_all` 不得被 `write.rs` 之外直接调用，可见性即守卫）。
pub(crate) use write::{Carrier, MessageChange};

/// 会话引擎句柄（`SESSION_HANDLE` 键的值类型）。
pub struct ChatSessionHandle(pub Arc<PersistentChatSession>);

impl ChatSessionHandle {
    pub fn new(session: Arc<PersistentChatSession>) -> Self {
        Self(session)
    }
}

// 会话句柄 Key（Value）：session 编排器交付给 chat_loop 的会话引擎实例
pub struct SessionHandleKey;
impl SymbioKey for SessionHandleKey {
    type Value = Arc<ChatSessionHandle>;
    fn name(&self) -> &'static str {
        "session_handle"
    }
    fn parse(&self, _s: &str) -> Option<Self::Value> {
        None
    }
    fn format(&self, _v: &Self::Value) -> String {
        "chat_session_handle".to_string()
    }
}
pub const SESSION_HANDLE: SessionHandleKey = SessionHandleKey;

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

    /// 对话窗口（最近轮数，**含当前轮**）——v2 执行器的 prompt 窗口
    /// （锁被占回落默认，与 [`Self::cfg_or_default`] 同一读取纪律）。
    pub(crate) fn context_window(&self) -> u64 {
        self.cfg_or_default().context_messages as u64
    }

    async fn load_session(&self) -> Result<super::types::Session, PluginError> {
        self.store.load_session(&self.session_id).await
    }

    /// 本会话所属**父会话的主体**——委派链上的父主体（[]）。
    ///
    /// 口径三段，每一段都只有一个 owner：子会话 `metadata.parent_session_id`
    /// → 父会话 `metadata.agent_id` → `authz::principal_of`（部署事实的唯一派生点）。
    ///
    /// 任一环落空 ⇒ `None`：顶层会话的正常形态，或父会话已删 / 读失败。调用方
    /// （`orchestrator::consume`）按 `Scope::Root` 处置——与接线前逐字一致。
    /// **这不是 fail-open**：作用域是**权威的上限**，不是许可本身；
    /// 上限取不到时每一格仍按子主体自己那一行判（`turn_runner::closure_gate`）。
    ///
    pub(crate) async fn delegating_principal(&self) -> Option<String> {
        let session = self.load_session().await.ok()?;
        delegating_principal_in(&self.store, &session).await
    }

    /// 子会话清单（Q3 worker 进展的读侧入口）：委托 store 枚举
    /// `<根>/<safe(本会话)>/sessions/*/session.json`——**按路径天然只含本父之子**，
    /// 归属复核另走 `Session::parent_session_id`。
    ///
    /// 只读 `session.json`（「清单不碰消息」，与会话量脱钩）；`messages.json` 由
    /// [`Self::load_sub_session`] 按需单独取。
    pub(crate) async fn list_sub_sessions(
        &self,
    ) -> Result<Vec<super::types::SessionSummary>, PluginError> {
        self.store.list_sub_sessions(&self.session_id).await
    }

    /// 载入**子**会话的完整转写（含 `messages.json`）——Q3 投影要 `Session.messages`。
    /// store 的 `load_session` 走嵌套查找，子会话可凭自身 id 直接寻址。
    pub(crate) async fn load_sub_session(
        &self,
        id: &str,
    ) -> Result<super::types::Session, PluginError> {
        self.store.load_session(id).await
    }

    async fn save_session(&self, session: &super::types::Session) -> Result<(), PluginError> {
        self.store.save_session(session).await
    }

    /// 本会话的**事实源句柄**（D-084 §1-B）：会话内写方/读方拿它落格、取切片，
    /// 不再自己拼目录 `open`。临时会话 ⇒ `Ok(None)`；事实源打不开 ⇒ `Err`（真失败）。
    pub(in crate::plugins::session) fn facts(
        &self,
    ) -> Result<Option<Arc<super::store::EventWalStore>>, PluginError> {
        self.store
            .facts(&self.session_id)
            .map_err(PluginError::InternalError)
    }

    /// 存储句柄（D-084 §2 的引擎侧端口）：只许 session 插件内的引擎面持有，
    /// 能力面不得见到 `SessionStore` 类型。
    pub(in crate::plugins::session) fn store_handle(&self) -> Arc<SessionStore> {
        self.store.clone()
    }

    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
    }
}

/// 委派链上的**父主体**——三段取值（子 `metadata.parent_session_id` → 父
/// `metadata.agent_id` → [`crate::symbio_core::principal_of`]）的**唯一实现**。
///
/// [`PersistentChatSession::delegating_principal`] 是运行期读面；会话**创建**时
/// （`commands::session_upsert`）也要用同一份判据登记主体行（[]），两处
/// 各写一遍迟早分叉成「登记挂 A、运行期判 B」。
pub(super) async fn delegating_principal_in(
    store: &SessionStore,
    session: &super::types::Session,
) -> Option<String> {
    let parent_id = session.parent_session_id()?;
    let parent = store.load_session(parent_id).await.ok()?;
    let agent = parent.metadata.get("agent_id").and_then(|v| v.as_str());
    Some(crate::symbio_core::principal_of(agent))
}

#[cfg(test)]
#[path = "chat_session.test.rs"]
mod tests;
