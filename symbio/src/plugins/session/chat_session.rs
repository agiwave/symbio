use super::context::{apply_layered_sliding_window, prune_historical_tool_calls};
use super::store::SessionStore;
use crate::symbio_core::schemas::session::chat_message::{ChatMessage, MessageRole, MessageStatus};
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
        messages.sort_by_key(|m| m.timestamp);

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

        for mut chat_msg in messages {
            if chat_msg.timestamp.unwrap_or(0) == 0 {
                chat_msg.timestamp = Some(now);
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
        session.messages = messages;
        session.updated_at = now;
        self.save_session(&session).await
    }

    async fn update_messages(&self, messages: Vec<ChatMessage>) -> Result<(), PluginError> {
        let mut session = self.load_session().await?;
        let now = (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64;
        for patch in messages {
            if let Some(existing) = session.messages.iter_mut().find(|m| m.id == patch.id) {
                *existing = patch;
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
        Ok(messages.clone())
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

        for mut msg in messages {
            if msg.timestamp.unwrap_or(0) == 0 {
                msg.timestamp = Some(now);
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
        *store = messages;
        Ok(())
    }

    async fn update_messages(&self, messages: Vec<ChatMessage>) -> Result<(), PluginError> {
        let mut store = self.messages.write().await;
        for patch in messages {
            if let Some(existing) = store.iter_mut().find(|m| m.id == patch.id) {
                *existing = patch;
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
