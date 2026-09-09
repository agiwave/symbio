use crate::symbio_core::schemas::session::chat_message::ChatMessage;
use crate::symbio_core::PluginError;
use async_trait::async_trait;
use std::sync::Arc;

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

    async fn clear(&self) -> Result<(), PluginError>;

    fn session_id(&self) -> &str;

    fn max_messages(&self) -> usize;

    fn line_threshold(&self) -> usize;

    /// 压缩给定消息批次（内容级骨架化：原文存档至持久层 + 正文替换为压缩骨架）。
    ///
    /// 默认实现**原样返回**（不做任何压缩）：
    /// - 无存档能力的会话做压缩即纯截断 → 原文永久丢失（decompress 还原将无源可读）；
    /// - 对 ephemeral / fallback 会话（无持久存储）原样返回恰是现状行为：
    /// 持久会话由 PersistentChatSession 覆写（存档批处理，与自动压缩语义一致）。
    async fn compress_messages(
        &self,
        messages: Vec<ChatMessage>,
    ) -> Result<Vec<ChatMessage>, PluginError> {
        let _ = self;
        Ok(messages)
    }
}

pub struct ChatSessionHandle(pub Arc<dyn ChatSession>);

impl ChatSessionHandle {
    pub fn new(session: Arc<dyn ChatSession>) -> Self {
        Self(session)
    }
}
