//! ProviderLlmAdapter —— ⑤ `LlmAdapter` 端口的**真实实现**（S2 预留的接线点）。
//!
//! S2 彩排用 [`StubLlmAdapter`]（确定性桩）走通闭环形状；本适配器把端口接到
//! **真实传输层**：持 `Arc<dyn ModelProvider>`（core 契约），`generate` 经
//! `execute_turn` 走「请求体构造 → 真实 HTTP POST → SSE 流解析 → 文本聚合」。
//! 会话链路（chat_loop / classify / compose）与 core 彩排链路（`Reasoner`）
//! 从此共享同一个真实模型入口——**换适配器不换链路**。
//!
//! 依赖方向：plugins → core（本文件在 plugins/model，依赖方向合法；
//! core 的 ⑤ 不认识任何插件类型）。

use std::sync::Arc;

use async_trait::async_trait;

use crate::symbio_core::adapters::{AdapterError, FullModel, LlmAdapter};
use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole,
};
use crate::symbio_core::ExecAbortSignal;
use crate::symbio_core::ExecEnv;
use crate::symbio_core::ExecEventSink;
use crate::symbio_core::{llm_short_id, ModelProvider};

/// ⑤ 端口的真实实现：把 `ModelProvider::execute_turn`（五态机 + SSE 解析）
/// 折进 `LlmAdapter::generate`（prompt 入、文本出）的最小适配。
#[allow(dead_code)] // dead-code-allow R-001: 会话链路切到彩排生成器+本适配器时的接线点；真实 HTTP/SSE 全链路已在 provider_adapter.test 验证
pub struct ProviderLlmAdapter {
    provider: Arc<dyn ModelProvider>,
}

impl ProviderLlmAdapter {
    #[allow(dead_code)] // dead-code-allow R-001: 同上——全链路彩排的构造入口
    pub fn new(provider: Arc<dyn ModelProvider>) -> Self {
        ProviderLlmAdapter { provider }
    }
}

#[async_trait]
impl LlmAdapter for ProviderLlmAdapter {
    fn model_id(&self) -> &str {
        self.provider.provider_id()
    }

    /// 单轮生成。`_tok` 是闸门证据：能拿到本方法的调用方必然已持 `FullModel`
    /// （闸门在 [`crate::symbio_core::Reasoner::reply`] 的签名上，适配器不重复查）。
    async fn generate(&self, _tok: &FullModel, prompt: &str) -> Result<String, AdapterError> {
        let message = ChatMessage {
            id: llm_short_id(),
            role: Some(MessageRole::User),
            content: Some(MessageContent::Text(prompt.to_string())),
            ..Default::default()
        };
        // 内部请求：出口静默、中止信号独立（同 classify 直呼路径的形态）。
        let env = ExecEnv::new(ExecEventSink::silent(), ExecAbortSignal::new());
        let output = self
            .provider
            .execute_turn(
                "You are a helpful assistant.",
                &[message],
                &[],
                &llm_short_id(),
                &env,
            )
            .await
            .map_err(|e| AdapterError::GenerationFailed(format!("{e}")))?;
        if output.text.trim().is_empty() {
            // 空文本不是成功：调用方拿到 Ok(空串) 会把「模型没答」当「答了空话」
            // 落成 final——必须走兜底路径（I3），所以这里按失败返回。
            return Err(AdapterError::GenerationFailed(
                "model returned empty text".into(),
            ));
        }
        Ok(output.text)
    }
}

#[cfg(test)]
#[path = "provider_adapter.test.rs"]
mod tests;
