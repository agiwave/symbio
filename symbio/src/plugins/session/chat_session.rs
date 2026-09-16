//! 会话引擎：契约（`ChatSession`）+ 唯一实现（`PersistentChatSession`）。
//!
//! 实现只有一个，"要不要持久化"的差异下沉到存储（`store::SessionStore` 的
//! `new` / `ephemeral` 两种构造）：内存态会话经 [`PersistentChatSession::detached`]
//! 构造，因此与持久会话共享同一套孤儿清理、轮次窗口与配置读取语义（审计 B1）。
//!
//! - 持久化实现委托 `super::store::SessionStore` 落库；不落盘面向 `_t_` 临时会话。
//! - 滑动窗口、孤儿剔除、时间戳回填等纯函数均在本文件。
//! - `prune_historical_tool_calls`（存储期工具链物理裁剪）由原 `context.rs`
//!   并入——其唯一消费者就是本模块（体检备注 audit-5）。
//!
//! ## 契约为何归属 session 插件
//!
//! `ChatSession` / `ChatSessionHandle` / `SESSION_HANDLE` 的读写方全部在 session
//! 插件内（编排器交付句柄 → chat_loop / resume / handlers 消费），不存在跨插件使用，
//! 故从 `symbio_core` 下沉到本插件——核心架构只保留跨插件共享的抽象，不承载单一
//! 模块的内部定义。

use super::store::SessionStore;
use crate::plugin_info;
use crate::symbio_core::now_ms;
use crate::symbio_core::schemas::session::chat_message as cm;
use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageType,
};
use crate::symbio_core::schemas::session::session_config::{
    default_context_messages, SessionConfig,
};
use crate::symbio_core::{PluginError, SymbioKey};
use async_trait::async_trait;
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::RwLock;

/// 会话引擎契约：消息的读写与轮次窗口视图。
#[async_trait]
pub trait ChatSession: Send + Sync + 'static {
    async fn get_messages(&self) -> Result<Vec<ChatMessage>, PluginError>;

    /// 获取进入 LLM 上下文的候选消息（存储视图：过滤 + 滑动轮次窗口）。
    ///
    /// 注意：工具级骨架化（fade / 保留策略）**不在此处**——那是请求视图层的职责，
    /// 由模型插件 run_chat_loop 在构建每次请求时统一执行（build_request_view）。
    async fn get_context_messages(
        &self,
        max_turns: Option<usize>,
    ) -> Result<Vec<ChatMessage>, PluginError>;

    async fn append_messages(&self, messages: Vec<ChatMessage>) -> Result<usize, PluginError>;

    async fn replace_messages(&self, messages: Vec<ChatMessage>) -> Result<(), PluginError>;

    /// 按 id 就地更新已存在的消息（**增量合并**，调用 [`ChatMessage::apply_patch`]）。
    /// 不存在的 id 静默跳过。用于工具恢复时更新 ToolCall 父节点状态。
    ///
    /// 注意：绝不可实现为"整条覆盖"。调用方普遍只传局部补丁（如仅 `id` + `meta`），
    /// 整条覆盖会把 `role` / `msg_type` / `content` / `timestamp` 抹成 `None`，
    /// 进而让下一轮请求体出现 `"content": null` 被 Provider 拒绝。
    async fn update_messages(&self, messages: Vec<ChatMessage>) -> Result<(), PluginError>;

    fn session_id(&self) -> &str;

    fn line_threshold(&self) -> usize;

    /// 内容节点淡化保护窗口：请求视图中最近 N 条内容节点（Text/Reasoning）保留原文。
    ///
    /// 对话末端锚点——保持模型对"最近在做什么/刚想了什么"的连续记忆。
    /// 默认取 `SessionConfig` 的默认值，持久会话从配置读取（真源唯一）。
    fn compress_keep_recent(&self) -> usize {
        SessionConfig::default().compress_keep_recent
    }

    /// 老旧工具结果淡化（fade）的激活阈值：单轮请求的工具迭代轮数**超过**此值后，
    /// 请求视图才把较早轮次的工具结果压成头尾摘要。
    ///
    /// 取代 `chat_loop` 中原有的硬编码常量 `FADE_ACTIVATE_ROUNDS`（审计 R3：fade
    /// 参数与 `SessionConfig` 双源），真源统一为配置字段 `fade_activate_rounds`。
    fn fade_activate_rounds(&self) -> usize {
        SessionConfig::default().fade_activate_rounds
    }

    /// fade 的保留窗口：最近 N 个 user turn 的工具结果保持原文，更早的才淡化。
    ///
    /// 取代原硬编码常量 `FADE_KEEP_RECENT_TURNS`。
    fn fade_keep_recent_turns(&self) -> usize {
        SessionConfig::default().fade_keep_recent_turns
    }
}

/// 会话引擎句柄（`session/open` 的返回载荷、`SESSION_HANDLE` 键的值类型）。
pub struct ChatSessionHandle(pub Arc<dyn ChatSession>);

impl ChatSessionHandle {
    pub fn new(session: Arc<dyn ChatSession>) -> Self {
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

fn sliding_window(messages: &[ChatMessage], max_turns: usize) -> Vec<ChatMessage> {
    if max_turns == 0 {
        return messages.to_vec();
    }

    let mut user_indices = Vec::new();
    for (idx, msg) in messages.iter().enumerate() {
        if msg.role == Some(MessageRole::User) {
            user_indices.push(idx);
        }
    }

    if user_indices.len() <= max_turns {
        return messages.to_vec();
    }

    let start_idx = user_indices[user_indices.len() - max_turns];
    messages[start_idx..].to_vec()
}

/// 返回给 LLM 前对单条消息做 content 归一兜底：
/// - 角色为 None 的占位消息不入上下文，直接跳过（避免 role 空污染 LLM）；
/// - content 为合法 `Text`/`Parts` 的保留原样；
/// - 缺失 content 的一律补成空串（**不再原样放行 `None`**）。
///
/// ## 为什么不能再放行 `content: None`
///
/// `NativeMessage::to_api_value` 对 `None` 输出 JSON `null`，而 Provider 侧
/// `MessageContent` 是 `String | ContentBlock[]` 的 untagged enum，`null`
/// 两个变体都不匹配，于是整包被 400 拒绝：
/// `messages[1]: data did not match any variant of untagged enum MessageContent`。
/// 一条脏消息就能让整个会话彻底卡死在 400，且用户无法自行恢复。
///
/// 仅作用于返回的构造结果，不修改存储。
fn normalize_message_content(msg: ChatMessage) -> Option<ChatMessage> {
    // 占位消息（无角色）直接丢弃；as_ref 避免部分移动（后续 ..msg 复用其余字段）
    msg.role.as_ref()?;
    let content_ok = matches!(
        &msg.content,
        Some(MessageContent::Parts(_)) | Some(MessageContent::Text(_))
    );
    if content_ok {
        return Some(msg);
    }
    let raw = msg
        .content
        .as_ref()
        .map(|c| c.to_text())
        .unwrap_or_default();
    let text = if raw.is_empty() {
        // 组合节点（Turn/ToolCall）与无内容消息统一落空串，保证 JSON 里是合法字符串
        String::new()
    } else {
        raw
    };
    Some(ChatMessage {
        content: Some(MessageContent::Text(text)),
        ..msg
    })
}

/// 丢弃"孤儿"消息：`parent_id` 指向本批次里不存在的节点。
///
/// 孤儿来源：
/// - Failed Turn 被 `resume::process_retry_turn` 删除时若子节点未一并清理；
/// - `persist_failure` 把仅存在于内存的流式子节点直接追加进存储；
/// - 兜底压缩（[`crate::symbio_core`] 之外的 `compression::emergency_tail_compression`）
///   按 token 预算切中段时，保留区首条可能是父节点已被截断的 Tool 结果。
///
/// 这些节点会被 `flatten_chat_messages` 当成根节点单独发一条 native message，
/// 其中 tool 结果还会携带一个请求里根本不存在的 `tool_call_id`，Provider 直接报错。
/// 因此进上下文前统一剔除（根级的 Turn / User 无 parent，天然不受影响）。
///
/// 采用**不动点迭代**：一次截断可能同时打断多层父子链（Turn → ToolCall → Tool），
/// 单趟过滤只解一层，剩下的会以"父在集合内"的假象漏出。
pub(crate) fn drop_orphan_messages(mut messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    // 不动点迭代：父节点被剔除后，其子节点也随之成为孤儿（截断/清理场景下
    // 可能出现多层链，如 Turn → ToolCall → Tool）。单趟过滤只解一层。
    loop {
        let ids: std::collections::HashSet<String> =
            messages.iter().map(|m| m.id.clone()).collect();
        let before = messages.len();
        messages.retain(|m| match m.parent_id.as_deref() {
            None => true,
            Some(pid) => ids.contains(pid),
        });
        if messages.len() == before {
            return messages;
        }
    }
}

/// 为缺失 `timestamp` 的消息回填 `now`，保持数组内的相对顺序不被排序打乱。
fn backfill_timestamps(messages: Vec<ChatMessage>, now: i64) -> Vec<ChatMessage> {
    messages
        .into_iter()
        .map(|mut m| {
            if m.timestamp.unwrap_or(0) == 0 {
                m.timestamp = Some(now);
            }
            m
        })
        .collect()
}

// PersistentChatSession

pub struct PersistentChatSession {
    session_id: String,
    config: Arc<RwLock<SessionConfig>>,
    store: Arc<SessionStore>,
    /// 写入期是否执行工具链物理裁剪（[`prune_historical_tool_calls`]）。
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

    async fn load_session(&self) -> Result<super::types::Session, PluginError> {
        self.store.load_session(&self.session_id).await
    }

    async fn save_session(&self, session: &super::types::Session) -> Result<(), PluginError> {
        self.store.save_session(session).await
    }
}

#[async_trait]
impl ChatSession for PersistentChatSession {
    async fn get_messages(&self) -> Result<Vec<ChatMessage>, PluginError> {
        let session = self.load_session().await?;
        let mut messages: Vec<_> = session.messages.to_vec();
        // 按**单调序号** `seq` 排序（稳定排序，缺失 seq 的旧数据排最后并保持插入顺序）。
        //
        // 这里曾经用 `sort_by_key(|m| m.timestamp)`，有两个致命问题：
        //   1. `Option` 序是 `None < Some(_)`，缺失 timestamp 的消息被顶到最前面，
        //      会话顺序被打乱，父节点可能排到子节点之后；
        //   2. timestamp 是"时刻"不是"顺序"——同一毫秒批量落库的消息会并列，
        //      排序退化为依赖数组当前顺序，而数组顺序又会被上一次错误排序打乱。
        // 改用写入时分配的单调 seq 后，两者都被消除。
        messages.sort_by_key(|m| m.seq.unwrap_or(i64::MAX));

        Ok(messages)
    }

    async fn get_context_messages(
        &self,
        max_turns: Option<usize>,
    ) -> Result<Vec<ChatMessage>, PluginError> {
        let messages = self.get_messages().await?;
        // **不过滤 Failed 消息**（"继续会话"中断可见性，docs/turn-tool-mechanisms.md 2.6）：
        // 用户选择不重试、直接继续对话时，模型必须看到上一轮的中断现场——
        // 失败 Turn 的半截输出 + 中断说明——否则思维链断裂。
        // persist_failure 只把根 Turn 标 Failed（半截子节点定稿 Completed），
        // 因此整树保留即可；中断说明与占位工具结果由请求视图层
        // （flatten_chat_messages / build_request_view）按 status 动态补齐。
        // 下方孤儿过滤退化为安全网（正常路径失败 Turn 整树保留，无孤儿产生）。
        // 剔除父节点缺失的孤儿节点（否则会带着不存在的 tool_call_id 进请求包）
        let messages: Vec<ChatMessage> = drop_orphan_messages(messages);
        // 对各消息做 content 归一兜底（并跳过 role 为 None 的占位消息），
        // 防止历史坏消息导致 provider 反序列化 MessageContent 失败。
        let messages: Vec<ChatMessage> = messages
            .into_iter()
            .filter_map(normalize_message_content)
            .collect();
        let turns = max_turns.unwrap_or_else(|| {
            self.config
                .try_read()
                .map(|c| c.context_messages)
                .unwrap_or_else(|_| default_context_messages())
        });
        let result = sliding_window(&messages, turns);
        // 会话层只做全局轮次窗口；工具级骨架化/保留策略由模型插件 run_chat_loop
        // 构建请求视图时统一解析（build_request_view），避免对压缩原料的提前污染。
        Ok(result)
    }

    async fn append_messages(&self, messages: Vec<ChatMessage>) -> Result<usize, PluginError> {
        let mut session = self.load_session().await?;
        let now = now_ms();

        let cfg = self.config.read().await;

        // 分配单调序号：起点取当前会话已有最大 seq，保证追加的消息严格排在其后。
        let mut seq_cursor = cm::max_seq(&session.messages);

        // 存储保持**完整原文**（架构原则，见 chat_loop「存储保持完整历史，视图逐轮裁剪」）：
        // 一切压缩均发生在"发给大模型之前"——L0 工具结果守卫在工具执行生产时刻、
        // L2 上下文摘要在 Turn 开始、内容节点淡化在请求视图组装（build_request_view）。
        // 落库前不做任何内容改写：压缩原料不被污染，UI（reason 面板等）读到的也是全文。
        for mut chat_msg in messages {
            if chat_msg.timestamp.unwrap_or(0) == 0 {
                chat_msg.timestamp = Some(now);
            }
            if chat_msg.seq.is_none() {
                seq_cursor += 1;
                chat_msg.seq = Some(seq_cursor);
            } else if let Some(s) = chat_msg.seq {
                if s > seq_cursor {
                    seq_cursor = s;
                }
            }

            session.messages.push(chat_msg);
        }

        let context_messages = cfg.context_messages;
        // 内存临时会话（`prune_on_write = false`）跳过物理裁剪：其历史本就不回看，
        // 裁剪只会让同一轮内可复用的工具链凭空消失。
        let prune_tool_history = cfg.prune_tool_history && self.prune_on_write;
        // `max_messages` 直接生效，0 = 不限制（与 `context_messages` / `sliding_window` 语义一致）。
        // 历史上此处曾硬编码 `.max(500)`，导致设置面板中小于 500 的值静默失效。
        let max_turns = cfg.max_messages;
        drop(cfg);

        // 写入期工具链裁剪：架构原则（存储保持完整原文）的唯一例外，默认开启以
        // 控制磁盘与节点树体积；置 `prune_tool_history: false` 后存储严格保留全文，
        // 工具链裁剪完全交给请求视图层（build_request_view 的骨架化/淡化）。
        if prune_tool_history {
            prune_historical_tool_calls(&mut session.messages, context_messages);
        }

        let mut user_indices = Vec::new();
        for (idx, msg) in session.messages.iter().enumerate() {
            if msg.role == Some(MessageRole::User) {
                user_indices.push(idx);
            }
        }

        if max_turns > 0 && user_indices.len() > max_turns {
            let start_idx = user_indices[user_indices.len() - max_turns];
            // 轮次 FIFO 淘汰只删消息节点，**不动** `tool_archives/` 归档文件（审计 A3）。
            // 理由：归档的磁盘生命周期已有专职策略——`tool_result_guard` 按修改时间
            // 保留每会话最新 [`TOOL_ARCHIVE_KEEP`] 个文件（滚动淘汰）。写入路径上再删一遍
            // 既与"淘汰旧轮"无关（旧轮引用的往往是最新的那几个存档），又让配置项
            // `max_messages` 变成静默删文件的破坏性操作。
            let dropped = start_idx;
            session.messages.drain(0..start_idx);
            if dropped > 0 {
                plugin_info!(
                    "session",
                    "append_messages: 轮次窗口淘汰 {} 条消息（归档文件保留，由 L0 滚动策略回收）",
                    dropped
                );
            }
        }

        let count = session.messages.len();
        session.updated_at = now;
        self.save_session(&session).await?;

        Ok(count)
    }

    async fn replace_messages(&self, messages: Vec<ChatMessage>) -> Result<(), PluginError> {
        let mut session = self.load_session().await?;
        let now = now_ms();
        // 回填缺失的 timestamp 与 seq：replace 会整体重写消息列表，若保留 `None`，
        // `get_messages` 只能靠"哨兵 + 稳定排序"兜底，容易打乱"父先于子"的顺序。
        // seq 按调用方给出的数组顺序递增分配，因此**数组顺序即权威顺序**。
        let mut messages = backfill_timestamps(messages, now);
        cm::assign_seq(&mut messages, cm::max_seq(&session.messages));
        // 孤儿存档：`replace_messages` 整体重写消息列表（L2 语义压缩 / 紧急截断 /
        // Streaming 清理），被丢弃消息引用的 L0 `tool_archives/` 存档随之失去引用。
        // 此处**只统计不删除**（审计 A3）：归档的磁盘生命周期由 `tool_result_guard`
        // 的每会话滚动保留（最新 `TOOL_ARCHIVE_KEEP` 个）统一负责，写入路径不再
        // 承担删文件职责——历史上这里需要在删除前做"拒绝 `..` + 限定 archives 根"
        // 的双重路径校验，正是因为把不可信输入变成了删除动作。
        if let Some(dir) = self.store.session_dir(&self.session_id) {
            let kept: HashSet<&str> = messages
                .iter()
                .filter_map(|m| m.meta.as_ref())
                .filter_map(|m| m.get("archive_path"))
                .filter_map(|v| v.as_str())
                .collect();
            let orphans = session
                .messages
                .iter()
                .filter_map(|m| {
                    m.meta
                        .as_ref()
                        .and_then(|m| m.get("archive_path"))
                        .and_then(|v| v.as_str())
                })
                .filter(|p| !kept.contains(*p))
                .count();
            if orphans > 0 {
                plugin_info!(
                    "session",
                    "replace_messages: {} 个工具存档失去引用（保留于 {}，等待 L0 滚动回收）",
                    orphans,
                    dir.display()
                );
            }
        }
        session.messages = messages;
        session.updated_at = now;
        self.save_session(&session).await
    }

    async fn update_messages(&self, messages: Vec<ChatMessage>) -> Result<(), PluginError> {
        let mut session = self.load_session().await?;
        let now = now_ms();
        for patch in messages {
            if let Some(existing) = session.messages.iter_mut().find(|m| m.id == patch.id) {
                // 增量合并（非整条覆盖）：只更新 patch 中显式携带的字段。
                // 整条覆盖会把 role/type/content/timestamp 抹成 None，导致请求体出现
                // `"content": null` 被 Provider 以 invalid_request_error 拒绝。
                existing.apply_patch(&patch);
            }
        }
        session.updated_at = now;
        self.save_session(&session).await
    }

    fn session_id(&self) -> &str {
        &self.session_id
    }

    fn line_threshold(&self) -> usize {
        self.cfg_or_default().compress_line_threshold
    }

    fn compress_keep_recent(&self) -> usize {
        self.cfg_or_default().compress_keep_recent
    }

    fn fade_activate_rounds(&self) -> usize {
        self.cfg_or_default().fade_activate_rounds
    }

    fn fade_keep_recent_turns(&self) -> usize {
        self.cfg_or_default().fade_keep_recent_turns
    }
}

/// 写入期工具链物理裁剪（原 `context.rs` 并入，体检备注 audit-5）。
///
/// 「存储保持完整原文、压缩只发生在请求视图」架构原则的**唯一例外**：在落库时
/// 物理删除 `context_messages` 轮之前的 Tool / ToolCall / Reasoning 节点。
/// 由 `SessionConfig::prune_tool_history` 控制（默认 `true` 保持历史行为）；
/// 置 `false` 后工具链裁剪完全由请求视图层（`build_request_view`）承担，
/// 存储与前端节点树可回看全部历史。
///
/// **只删节点、不删文件**（审计 A3）：`meta.archive_path` 指向的 L0 归档由
/// `tool_result_guard` 的每会话滚动保留（最新 `TOOL_ARCHIVE_KEEP` 个，按修改时间）
/// 统一回收。写入路径此前把不可信的消息 meta 当删除依据（需额外做 `..` 与根目录
/// 双重校验），且"淘汰旧轮"删掉的常是最新存档——职责错位，故收敛为纯节点裁剪。
pub fn prune_historical_tool_calls(messages: &mut Vec<ChatMessage>, keep_turns: usize) {
    // keep_turns == 0 语义为「不限制」（与 `SessionConfig::context_messages` 的 0 值语义、
    // `sliding_window` 保持一致）。此处缺失守卫时，下面 `user_indices[len - keep_turns]`
    // 会取到下标 `len`（越界）并 panic，导致整次 `append_messages` 失败。
    if keep_turns == 0 {
        return;
    }
    let mut user_indices = Vec::new();
    for (idx, msg) in messages.iter().enumerate() {
        if msg.role == Some(MessageRole::User) {
            user_indices.push(idx);
        }
    }

    if user_indices.len() <= keep_turns {
        return;
    }

    let limit_idx = user_indices[user_indices.len() - keep_turns];
    let mut to_remove = HashSet::new();

    for msg in &messages[..limit_idx] {
        if msg.role == Some(MessageRole::Tool)
            || msg.msg_type == Some(MessageType::ToolCall)
            || msg.msg_type == Some(MessageType::Reasoning)
        {
            to_remove.insert(msg.id.clone());
        }
    }

    // 同时移除被剪除 ToolCall 的直接子节点（请求 Text / 响应 Text）
    let extra: HashSet<String> = messages[..limit_idx]
        .iter()
        .filter(|m| {
            m.parent_id
                .as_ref()
                .map(|p| to_remove.contains(p))
                .unwrap_or(false)
        })
        .map(|m| m.id.clone())
        .collect();
    to_remove.extend(extra);

    for msg in &messages[..limit_idx] {
        if msg.role == Some(MessageRole::Assistant) {
            let content_text = msg
                .content
                .as_ref()
                .map(|c| c.to_text())
                .unwrap_or_default();
            let mut has_retained_children = false;
            for child in &messages[..limit_idx] {
                if child.parent_id.as_ref() == Some(&msg.id) && !to_remove.contains(&child.id) {
                    has_retained_children = true;
                    break;
                }
            }
            if content_text.trim().is_empty() && !has_retained_children {
                to_remove.insert(msg.id.clone());
            }
        }
    }

    messages.retain(|msg| !to_remove.contains(&msg.id));
}

#[cfg(test)]
#[path = "chat_session.test.rs"]
mod tests;
