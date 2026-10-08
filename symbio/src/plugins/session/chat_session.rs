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
//! 曾经这里还有一个 `ChatSession` trait（唯一实现即本结构体）：单实现多态只会
//! 多一层 `dyn` 分发与一份契约文档的两处维护，已去除——方法直接内聚在实现上。

use super::config::{default_context_messages, SessionConfig};
use super::store::SessionStore;
use crate::plugin_info;
use crate::symbio_core::chat_message as cm;
use crate::symbio_core::chat_message::{ChatMessage, MessageContent, MessageRole, MessageType};
use crate::symbio_core::clock_now_ms;
use crate::symbio_core::{PluginError, SymbioKey};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

mod read;
mod write;

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

    /// 同 [`ephemeral`](Self::ephemeral)，但配置以**值快照**传入（调用方无需自行构造
    /// `Arc<RwLock<_>>`）：内存会话不接 `session/config` 热更新，快照即其终生命周期的配置。
    pub fn detached(session_id: impl Into<String>, config: SessionConfig) -> Self {
        Self::ephemeral(session_id, Arc::new(RwLock::new(config)))
    }

    /// 读当前配置；锁被占时回落到 `SessionConfig::default()`。
    ///
    /// 不再手写魔数兜底（历史上是 `unwrap_or(200)` / `unwrap_or(3)`，与默认值三源）：
    /// 默认值只存在于 `SessionConfig` 一处。
    fn cfg_or_default(&self) -> SessionConfig {
        self.config
            .try_read()
            .map(|c| c.clone())
            .unwrap_or_default()
    }

    /// 当前 v2 切换档位（`v2_bridge` 转写与否的唯一判据；锁被占时回落默认档）。
    pub(crate) fn v2_mode(&self) -> super::config::V2Mode {
        self.cfg_or_default().v2_mode
    }

    /// 对话窗口（最近轮数，**含当前轮**）——v2 执行器的 prompt 窗口
    /// （锁被占回落默认；与 [`Self::v2_mode`] 同一读取纪律）。
    pub(crate) fn context_window(&self) -> u64 {
        self.cfg_or_default().context_messages as u64
    }

    async fn load_session(&self) -> Result<super::types::Session, PluginError> {
        self.store.load_session(&self.session_id).await
    }

    /// 子会话清单（Q3 worker 进展的读侧入口）：委托 store 枚举
    /// `<根>/<safe(本会话)>/sessions/*/session.json`——**按路径天然只含本父之子**，
    /// 归属复核另走 [`Session::parent_session_id`](super::types::Session::parent_session_id)。
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

    /// 会话目录（v2 事实桥的 WAL 安放处）；临时会话 ⇒ `None`。
    pub(crate) fn session_dir(&self) -> Option<std::path::PathBuf> {
        self.store.session_dir(&self.session_id)
    }

    pub(crate) fn session_id(&self) -> &str {
        &self.session_id
    }

    pub(crate) fn line_threshold(&self) -> usize {
        self.cfg_or_default().compress_line_threshold
    }

    /// 内容节点淡化保护窗口：请求视图中最近 N 条内容节点（Text/Reasoning）保留原文。
    ///
    /// 对话末端锚点——保持模型对"最近在做什么/刚想了什么"的连续记忆。
    pub(crate) fn compress_keep_recent(&self) -> usize {
        self.cfg_or_default().compress_keep_recent
    }

    /// 老旧工具结果淡化（fade）的激活阈值：单轮请求的工具迭代轮数**超过**此值后，
    /// 请求视图才把较早轮次的工具结果压成头尾摘要。
    pub(crate) fn fade_activate_rounds(&self) -> usize {
        self.cfg_or_default().fade_activate_rounds
    }

    /// 技能编译开关（步 22 · [roadmap/S11](../../../../docs/plan/roadmap/S11-技能编译与自我改进.md)；
    /// 锁被占回落默认 off——与 [`Self::v2_mode`] 同一读取纪律）。
    pub(crate) fn skill_compile_enabled(&self) -> bool {
        self.cfg_or_default().skill_compile_enabled
    }

    /// 技能快路开关（步 22 的**执行半边**，S11 §4 的 `actor.pattern`；
    /// 锁被占回落默认 off——与 [`Self::v2_mode`] 同一读取纪律）。
    pub(crate) fn skill_fast_path(&self) -> bool {
        self.cfg_or_default().skill_fast_path
    }

    /// fade 的保留窗口：最近 N 个 user turn 的工具结果保持原文，更早的才淡化。
    pub(crate) fn fade_keep_recent_turns(&self) -> usize {
        self.cfg_or_default().fade_keep_recent_turns
    }
}

#[cfg(test)]
#[path = "chat_session.test.rs"]
mod tests;
