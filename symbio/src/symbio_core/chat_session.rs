use crate::symbio_core::schemas::session::chat_message::ChatMessage;
use crate::symbio_core::PluginError;
use async_trait::async_trait;
use std::sync::Arc;

#[async_trait]
pub trait ChatSession: Send + Sync + 'static {
    async fn get_messages(&self) -> Result<Vec<ChatMessage>, PluginError>;

    async fn get_context_messages(
        &self,
        max_turns: Option<usize>,
        tool_context_window: Option<usize>,
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

    async fn clear(&self) -> Result<(), PluginError>;

    fn session_id(&self) -> &str;

    fn max_messages(&self) -> usize;

    fn line_threshold(&self) -> usize;
}

pub struct ChatSessionHandle(pub Arc<dyn ChatSession>);

impl ChatSessionHandle {
    pub fn new(session: Arc<dyn ChatSession>) -> Self {
        Self(session)
    }
}
