//! 会话引擎：契约（`ChatSession`）+ 持久化（`PersistentChatSession`）与内存
//! （`EphemeralChatSession`）两种实现。
//!
//! - 持久化实现委托 `super::store::SessionStore` 落库；内存实现面向 `_t_` 临时会话。
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
use crate::symbio_core::schemas::session::chat_message as cm;
use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageType,
};
use crate::symbio_core::schemas::session::session_config::SessionConfig;
use crate::symbio_core::{PluginError, SymbioKey};
use async_trait::async_trait;
use std::collections::HashSet;
use std::path::Path;
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
    /// 默认 3，与压缩配置 `compress_keep_recent` 对齐；持久会话从配置读取。
    fn compress_keep_recent(&self) -> usize {
        3
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
/// - `persist_failure` 把仅存在于内存的流式子节点直接追加进存储。
///
/// 这些节点会被 `flatten_chat_messages` 当成根节点单独发一条 native message，
/// 其中 tool 结果还会携带一个请求里根本不存在的 `tool_call_id`，Provider 直接报错。
/// 因此进上下文前统一剔除（根级的 Turn / User 无 parent，天然不受影响）。
fn drop_orphan_messages(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    let ids: std::collections::HashSet<String> = messages.iter().map(|m| m.id.clone()).collect();
    messages
        .into_iter()
        .filter(|m| match m.parent_id.as_deref() {
            None => true,
            Some(pid) => ids.contains(pid),
        })
        .collect()
}

/// 当前毫秒时间戳（与 `append_messages` / `replace_messages` 里的取值方式保持一致）。
fn now_millis() -> i64 {
    (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64
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
    store: Arc<dyn SessionStore>,
}

impl PersistentChatSession {
    pub fn new(
        session_id: String,
        config: Arc<RwLock<SessionConfig>>,
        store: Arc<dyn SessionStore>,
    ) -> Self {
        Self {
            session_id,
            config,
            store,
        }
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
                .unwrap_or(6)
        });
        let result = sliding_window(&messages, turns);
        // 会话层只做全局轮次窗口；工具级骨架化/保留策略由模型插件 run_chat_loop
        // 构建请求视图时统一解析（build_request_view），避免对压缩原料的提前污染。
        Ok(result)
    }

    async fn append_messages(&self, messages: Vec<ChatMessage>) -> Result<usize, PluginError> {
        let mut session = self.load_session().await?;
        let now = (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64;

        let session_dir = self.store.session_dir(&self.session_id);

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
        drop(cfg);

        prune_historical_tool_calls(
            &mut session.messages,
            session_dir.as_deref(),
            context_messages,
        )
        .await;

        let max_turns = self.config.read().await.max_messages.max(500);
        let mut user_indices = Vec::new();
        for (idx, msg) in session.messages.iter().enumerate() {
            if msg.role == Some(MessageRole::User) {
                user_indices.push(idx);
            }
        }

        if user_indices.len() > max_turns {
            let start_idx = user_indices[user_indices.len() - max_turns];
            if let Some(ref s_dir) = session_dir {
                for msg in &session.messages[0..start_idx] {
                    if let Some(ref meta) = msg.meta {
                        if let Some(archive_path_val) =
                            meta.get("archive_path").and_then(|v| v.as_str())
                        {
                            let path = s_dir.join(archive_path_val);
                            if path.exists() {
                                let _ = tokio::fs::remove_file(path).await;
                            }
                        }
                    }
                }
            }
            session.messages.drain(0..start_idx);
        }

        let count = session.messages.len();
        session.updated_at = now;
        self.save_session(&session).await?;

        Ok(count)
    }

    async fn replace_messages(&self, messages: Vec<ChatMessage>) -> Result<(), PluginError> {
        let mut session = self.load_session().await?;
        let now = (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64;
        // 回填缺失的 timestamp 与 seq：replace 会整体重写消息列表，若保留 `None`，
        // `get_messages` 只能靠"哨兵 + 稳定排序"兜底，容易打乱"父先于子"的顺序。
        // seq 按调用方给出的数组顺序递增分配，因此**数组顺序即权威顺序**。
        let mut messages = backfill_timestamps(messages, now);
        cm::assign_seq(&mut messages, cm::max_seq(&session.messages));
        // 孤儿存档配对清理：replace 整体重写消息列表（L2 语义压缩 / 紧急截断 /
        // Streaming 清理），被丢弃消息引用的 L0 `tool_archives/` 存档随之失去引用。
        // 与 `append_messages` 的 FIFO 淘汰、`prune_historical_tool_calls` 的物理
        // 裁剪对称，在此配对删除：仅删"旧列表引用且新列表不再引用"的存档文件，
        // 保留（keep_recent 留下的）新列表仍引用的存档。路径不可信（来自消息
        // meta），删除前做双重校验（拒绝 `..` + 限定 tool_archives/ 根，见下）。
        if let Some(dir) = self.store.session_dir(&self.session_id) {
            let archives_root = dir.join(super::paths::TOOL_ARCHIVES_SUBDIR);
            let kept: HashSet<&str> = messages
                .iter()
                .filter_map(|m| m.meta.as_ref())
                .filter_map(|m| m.get("archive_path"))
                .filter_map(|v| v.as_str())
                .collect();
            for old in &session.messages {
                let Some(rel_path) = old
                    .meta
                    .as_ref()
                    .and_then(|m| m.get("archive_path"))
                    .and_then(|v| v.as_str())
                else {
                    continue;
                };
                if kept.contains(rel_path) {
                    continue;
                }
                // 路径来自消息 meta（不可信输入）：拒绝 `..` 组件，且拼接后必须
                // 落在本会话 `tool_archives/` 内（`starts_with` 是词法前缀匹配，
                // 只挡前缀不挡 `..` 逃逸，故两道校验缺一不可），防越界删除任意文件。
                if Path::new(rel_path)
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
                {
                    continue;
                }
                let full = dir.join(rel_path);
                if full.starts_with(&archives_root) && tokio::fs::remove_file(&full).await.is_ok() {
                    plugin_info!(
                        "session",
                        "replace_messages: 已清理孤儿工具存档 {}",
                        full.display()
                    );
                }
            }
        }
        session.messages = messages;
        session.updated_at = now;
        self.save_session(&session).await
    }

    async fn update_messages(&self, messages: Vec<ChatMessage>) -> Result<(), PluginError> {
        let mut session = self.load_session().await?;
        let now = (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64;
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
        self.config
            .try_read()
            .map(|c| c.compress_line_threshold)
            .unwrap_or(200)
    }

    fn compress_keep_recent(&self) -> usize {
        self.config
            .try_read()
            .map(|c| c.compress_keep_recent)
            .unwrap_or(3)
    }
}

// EphemeralChatSession

pub struct EphemeralChatSession {
    session_id: String,
    messages: RwLock<Vec<ChatMessage>>,
    context_messages: usize,
    max_messages: usize,
    line_threshold: usize,
    keep_recent: usize,
}

impl EphemeralChatSession {
    pub fn new(config: &SessionConfig) -> Self {
        Self {
            session_id: uuid::Uuid::new_v4().to_string(),
            messages: RwLock::new(Vec::new()),
            context_messages: config.context_messages,
            max_messages: config.max_messages.max(500),
            line_threshold: config.compress_line_threshold,
            keep_recent: config.compress_keep_recent,
        }
    }
}

#[async_trait]
impl ChatSession for EphemeralChatSession {
    async fn get_messages(&self) -> Result<Vec<ChatMessage>, PluginError> {
        let messages = self.messages.read().await;
        let mut messages = messages.clone();
        // 与 PersistentChatSession 一致：按单调序号排序，缺失 seq 的排最后并保持插入顺序
        messages.sort_by_key(|m| m.seq.unwrap_or(i64::MAX));
        Ok(messages)
    }

    async fn get_context_messages(
        &self,
        max_turns: Option<usize>,
    ) -> Result<Vec<ChatMessage>, PluginError> {
        let messages = self.messages.read().await;
        // **不过滤 Failed 消息**（与 PersistentChatSession 一致，2.6 节）：失败 Turn
        // 整树保留进上下文，中断说明由请求视图层补齐。
        let messages: Vec<ChatMessage> = messages.clone();
        // 与 PersistentChatSession 保持一致：剔孤儿 + content 归一，
        // 否则 content: None 会在请求体里序列化成 null 被 Provider 拒绝。
        let messages: Vec<ChatMessage> = drop_orphan_messages(messages);
        let messages: Vec<ChatMessage> = messages
            .into_iter()
            .filter_map(normalize_message_content)
            .collect();
        let turns = max_turns.unwrap_or(self.context_messages);
        let result = sliding_window(&messages, turns);
        // 与 PersistentChatSession 保持一致：工具级骨架化上移至模型插件请求视图。
        Ok(result)
    }

    async fn append_messages(&self, messages: Vec<ChatMessage>) -> Result<usize, PluginError> {
        let mut store = self.messages.write().await;
        let now = (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64;

        let mut seq_cursor = cm::max_seq(&store);
        for mut msg in messages {
            if msg.timestamp.unwrap_or(0) == 0 {
                msg.timestamp = Some(now);
            }
            if msg.seq.is_none() {
                seq_cursor += 1;
                msg.seq = Some(seq_cursor);
            } else if let Some(s) = msg.seq {
                if s > seq_cursor {
                    seq_cursor = s;
                }
            }
            store.push(msg);
        }

        let mut user_indices = Vec::new();
        for (idx, msg) in store.iter().enumerate() {
            if msg.role == Some(MessageRole::User) {
                user_indices.push(idx);
            }
        }

        if user_indices.len() > self.max_messages {
            let start_idx = user_indices[user_indices.len() - self.max_messages];
            store.drain(0..start_idx);
        }

        Ok(store.len())
    }

    async fn replace_messages(&self, messages: Vec<ChatMessage>) -> Result<(), PluginError> {
        let mut store = self.messages.write().await;
        let mut messages = backfill_timestamps(messages, now_millis());
        cm::assign_seq(&mut messages, cm::max_seq(&store));
        *store = messages;
        Ok(())
    }

    async fn update_messages(&self, messages: Vec<ChatMessage>) -> Result<(), PluginError> {
        let mut store = self.messages.write().await;
        for patch in messages {
            if let Some(existing) = store.iter_mut().find(|m| m.id == patch.id) {
                existing.apply_patch(&patch);
            }
        }
        Ok(())
    }

    fn session_id(&self) -> &str {
        &self.session_id
    }

    fn line_threshold(&self) -> usize {
        self.line_threshold
    }

    fn compress_keep_recent(&self) -> usize {
        self.keep_recent
    }
}

/// 存储期历史工具链物理裁剪（原 `context.rs` 并入，体检备注 audit-5）：
/// 自动清理历史会话的过程工具调用信息，只保留最后的文本结果。
pub async fn prune_historical_tool_calls(
    messages: &mut Vec<ChatMessage>,
    session_dir: Option<&Path>,
    keep_turns: usize,
) {
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
            if let Some(dir) = session_dir {
                if let Some(rel_path) = msg
                    .meta
                    .as_ref()
                    .and_then(|m| m.get("archive_path"))
                    .and_then(|v| v.as_str())
                {
                    let full_path = dir.join(rel_path);
                    if full_path.exists() {
                        let _ = tokio::fs::remove_file(full_path).await;
                    }
                }
            }
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
mod tests;
