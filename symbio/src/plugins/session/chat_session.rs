use super::context::{apply_layered_sliding_window, prune_historical_tool_calls};
use super::store::SessionStore;
use crate::symbio_core::schemas::session::chat_message as cm;
use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus,
};
use crate::symbio_core::schemas::session::session_config::SessionConfig;
use crate::symbio_core::{ChatSession, PluginError};
use async_trait::async_trait;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;

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
    if msg.role.is_none() {
        // 占位消息（无角色）直接丢弃
        return None;
    }
    let content_ok = matches!(
        &msg.content,
        Some(MessageContent::Parts(_)) | Some(MessageContent::Text(_))
    );
    if content_ok {
        return Some(msg);
    }
    let raw = msg.content.as_ref().map(|c| c.to_text()).unwrap_or_default();
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
    let ids: std::collections::HashSet<String> =
        messages.iter().map(|m| m.id.clone()).collect();
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

    fn resolve_display_path(&self, session_dir: Option<&std::path::Path>) -> PathBuf {
        // session 存储位置由 SessionPlugin::session_storage_dir() 派生（<homedir>/plugins/session）。
        // 这里的 display path 仅作 UI 展示（用于压缩消息的 archive path 等）。
        // 与实际写入路径 (session_dir) 保持一致：<homedir>/plugins/session/<safe_id>
        let _ = session_dir;
        let safe_session_id = self.session_id.replace(['/', '\\', ':'], "_");
        let storage_dir = super::plugin::SessionPlugin::session_storage_dir();
        storage_dir.join(safe_session_id)
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

        if let Some(last_msg) = messages.last_mut() {
            if let Some(session_dir) = self.store.session_dir(&self.session_id) {
                match super::compress::decompress_message(&session_dir, last_msg).await {
                    Ok(restored) => {
                        *last_msg = restored;
                    }
                    Err(e) => {
                        tracing::warn!("还原最后一条消息失败: {}", e);
                    }
                }
            }
        }

        Ok(messages)
    }

    async fn get_context_messages(
        &self,
        max_turns: Option<usize>,
        tool_context_window: Option<usize>,
    ) -> Result<Vec<ChatMessage>, PluginError> {
        let messages = self.get_messages().await?;
        // 过滤 Failed 消息：Failed 仅作为用户可见的失败终态（带重试按钮），
        // 不进入 LLM 上下文，避免污染后续对话。
        // 依赖 persist_failure 已将 Failed Turn 下所有半截 Streaming/Pending 子节点标记为 Failed，
        // 因此过滤 Failed 即可整树移除失败 Turn。
        let messages: Vec<ChatMessage> = messages
            .into_iter()
            .filter(|m| m.status != Some(MessageStatus::Failed))
            .collect();
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
        let mut result = sliding_window(&messages, turns);
        let window = tool_context_window.unwrap_or_else(|| {
            self.config
                .try_read()
                .map(|c| c.tool_context_window)
                .unwrap_or(15)
        });
        if window > 0 {
            result = apply_layered_sliding_window(&result, window);
        }
        Ok(result)
    }

    async fn append_messages(&self, messages: Vec<ChatMessage>) -> Result<usize, PluginError> {
        let mut session = self.load_session().await?;
        let now = (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64;

        let session_dir = self.store.session_dir(&self.session_id);
        let display_path = self.resolve_display_path(session_dir.as_deref());

        let cfg = self.config.read().await;
        let line_threshold = cfg.compress_line_threshold;

        // 分配单调序号：起点取当前会话已有最大 seq，保证追加的消息严格排在其后。
        let mut seq_cursor = cm::max_seq(&session.messages);

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

            let compressed = if let Some(ref dir) = session_dir {
                let ts = (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000) as i64;
                let archive_filename =
                    format!("{}/m{:x}.txt", super::compress::MESSAGES_SUBDIR, ts);
                let archive_display_path = display_path
                    .join(&archive_filename)
                    .to_string_lossy()
                    .replace("\\", "/");

                super::compress::compress_message(
                    dir,
                    &chat_msg,
                    line_threshold,
                    &archive_filename,
                    &archive_display_path,
                )
                .await
            } else {
                Ok(None)
            };

            let final_msg = match compressed {
                Ok(Some(c)) => c,
                Ok(None) => chat_msg,
                Err(e) => {
                    tracing::warn!("压缩消息失败: {}", e);
                    chat_msg
                }
            };

            session.messages.push(final_msg);
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

    async fn clear(&self) -> Result<(), PluginError> {
        self.store.delete_session(&self.session_id).await
    }

    async fn replace_messages(&self, messages: Vec<ChatMessage>) -> Result<(), PluginError> {
        let mut session = self.load_session().await?;
        let now = (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64;
        // 回填缺失的 timestamp 与 seq：replace 会整体重写消息列表，若保留 `None`，
        // `get_messages` 只能靠"哨兵 + 稳定排序"兜底，容易打乱"父先于子"的顺序。
        // seq 按调用方给出的数组顺序递增分配，因此**数组顺序即权威顺序**。
        let mut messages = backfill_timestamps(messages, now);
        cm::assign_seq(&mut messages, cm::max_seq(&session.messages));
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

    fn max_messages(&self) -> usize {
        self.config
            .try_read()
            .map(|c| c.max_messages)
            .unwrap_or(100)
    }

    fn line_threshold(&self) -> usize {
        self.config
            .try_read()
            .map(|c| c.compress_line_threshold)
            .unwrap_or(200)
    }
}

// EphemeralChatSession

pub struct EphemeralChatSession {
    session_id: String,
    messages: RwLock<Vec<ChatMessage>>,
    context_messages: usize,
    max_messages: usize,
    line_threshold: usize,
}

impl EphemeralChatSession {
    pub fn new(config: &SessionConfig) -> Self {
        Self {
            session_id: uuid::Uuid::new_v4().to_string(),
            messages: RwLock::new(Vec::new()),
            context_messages: config.context_messages,
            max_messages: config.max_messages.max(500),
            line_threshold: config.compress_line_threshold,
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
        tool_context_window: Option<usize>,
    ) -> Result<Vec<ChatMessage>, PluginError> {
        let messages = self.messages.read().await;
        // 过滤 Failed 消息：Failed 仅作为用户可见的失败终态（带重试按钮），
        // 不进入 LLM 上下文，避免污染后续对话。
        let messages: Vec<ChatMessage> = messages
            .iter()
            .filter(|m| m.status != Some(MessageStatus::Failed))
            .cloned()
            .collect();
        // 与 PersistentChatSession 保持一致：剔孤儿 + content 归一，
        // 否则 content: None 会在请求体里序列化成 null 被 Provider 拒绝。
        let messages: Vec<ChatMessage> = drop_orphan_messages(messages);
        let messages: Vec<ChatMessage> = messages
            .into_iter()
            .filter_map(normalize_message_content)
            .collect();
        let turns = max_turns.unwrap_or(self.context_messages);
        let mut result = sliding_window(&messages, turns);
        let window = tool_context_window.unwrap_or(15);
        if window > 0 {
            result = apply_layered_sliding_window(&result, window);
        }
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

    async fn clear(&self) -> Result<(), PluginError> {
        let mut messages = self.messages.write().await;
        messages.clear();
        Ok(())
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

    fn max_messages(&self) -> usize {
        self.max_messages
    }

    fn line_threshold(&self) -> usize {
        self.line_threshold
    }
}
