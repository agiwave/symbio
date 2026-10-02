//! ProviderLlmAdapter —— ⑤ `LlmAdapter` 端口的**真实实现**（S2 预留的接线点）。
//!
//! S2 彩排用 [`StubLlmAdapter`]（确定性桩）走通闭环形状；本适配器把端口接到
//! **真实传输层**：持 `Arc<dyn ModelProvider>`（core 契约），`generate` 经
//! `execute_turn` 走「请求体构造 → 真实 HTTP POST → SSE 流解析 → 文本聚合」。
//! 会话链路（chat_loop / classify / compose）与 core 彩排链路（`Reasoner`）
//! 从此共享同一个真实模型入口——**换适配器不换链路**。
//!
//! 依赖方向：本文件只依赖 core 契约（`ModelProvider` / `LlmAdapter` /
//! `ExecEventSink` 均在 core）——纯胶水就该住在契约旁边。真实传输层的全链路
//! 彩排（真实 TCP + HTTP + SSE）挂在 **`plugins/model/bound_provider.test.rs`**：
//! 那份测试要装配 `BoundProvider` / `openai_chat` 等插件内部件，而
//! `plugins::model` 是私有模块（core 侧**在可见性上**就到不了它）——
//! 依赖方向由编译器保证，不靠约定。

use std::sync::Arc;

use async_trait::async_trait;

use super::{AdapterError, DeltaSink, FullModel, LlmAdapter, LlmTurn, SilentDeltas};
use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
};
use crate::symbio_core::CapabilityMeta;
use crate::symbio_core::ExecAbortSignal;
use crate::symbio_core::ExecEnv;
use crate::symbio_core::ExecEventSink;
use crate::symbio_core::{llm_short_id, ExecTranscriptWriter, ModelProvider, PluginError};

/// 流式帧桥：把 `execute_turn` 的转写帧**择要**转成正文增量——
/// - 快照帧（`msg_type = Text × status = Streaming`）：登记节点 id，全文转发；
/// - 窄帧（`delta`）：只有**已登记的正文节点**才转发（reasoning / 工具增量
///   不进 v2 文本面——v2 的推理帧桥接是独立的一步，不在这里顺手做）。
///
/// 为什么不直接转发所有 delta：model 插件的窄帧只带 `id + delta`，不带
/// `msg_type`——不记账就分不清正文增量与推理增量。收束帧不经此桥：终态由
/// 消费方负责（model 插件只发 Streaming 快照与窄帧，见 `state.rs` 的收口）。
struct DeltaBridge {
    sink: Arc<dyn DeltaSink>,
    text_nodes: std::sync::Mutex<std::collections::HashSet<String>>,
}

#[async_trait]
impl ExecTranscriptWriter for DeltaBridge {
    async fn apply(&self, m: ChatMessage) {
        if let Some(d) = &m.delta {
            if !d.is_empty() && self.text_nodes.lock().unwrap().contains(&m.id) {
                self.sink.on_delta(d);
            }
            return;
        }
        if m.msg_type == Some(MessageType::Text) && m.status == Some(MessageStatus::Streaming) {
            if let Some(MessageContent::Text(t)) = &m.content {
                if !t.is_empty() {
                    self.text_nodes.lock().unwrap().insert(m.id.clone());
                    self.sink.on_delta(t);
                }
            }
        }
    }
}

/// ⑤ 端口的真实实现：把 `ModelProvider::execute_turn`（五态机 + SSE 解析）
/// 折进 `LlmAdapter::generate`（prompt 入、文本出）的最小适配。
#[allow(dead_code)] // dead-code-allow R-001: 会话链路切到彩排生成器+本适配器时的接线点；真实 HTTP/SSE 全链路已在 provider_adapter.test 验证
pub struct ProviderLlmAdapter {
    provider: Arc<dyn ModelProvider>,
    /// 会话级系统提示词（full 档经 `for_turn` 注入；缺省 = 彩排用的通用提示）。
    system: Option<String>,
    /// 本轮的中止信号（full 档经 `for_turn` 注入；缺省自建——彩排无需中止）。
    abort: Option<ExecAbortSignal>,
}

impl ProviderLlmAdapter {
    #[allow(dead_code)] // dead-code-allow R-001: 同上——全链路彩排的构造入口
    pub fn new(provider: Arc<dyn ModelProvider>) -> Self {
        ProviderLlmAdapter {
            provider,
            system: None,
            abort: None,
        }
    }

    /// 会话轮构造：系统提示词 + 本轮中止信号直通——中止与提示词是**调用方的
    /// 语境**，适配器不再自作主张（此前硬编码通用提示 + 自建中止信号，
    /// full 档接管会话轮后两者都必须由 chat_loop 传入）。
    #[allow(dead_code)] // dead-code-allow R-001: v2_exec（full 档）的构造入口
    pub fn for_turn(
        provider: Arc<dyn ModelProvider>,
        system: &str,
        abort: ExecAbortSignal,
    ) -> Self {
        ProviderLlmAdapter {
            provider,
            system: Some(system.to_string()),
            abort: Some(abort),
        }
    }

    fn system_prompt(&self) -> &str {
        self.system
            .as_deref()
            .unwrap_or("You are a helpful assistant.")
    }

    fn abort_signal(&self) -> ExecAbortSignal {
        self.abort.clone().unwrap_or_default()
    }
}

#[async_trait]
impl LlmAdapter for ProviderLlmAdapter {
    fn model_id(&self) -> &str {
        self.provider.provider_id()
    }

    /// 单轮生成（委托流式路径 + 静默接收口——**同一条执行路径**，不为
    /// 「不要流式」造第二条）。
    async fn generate(&self, tok: &FullModel, prompt: &str) -> Result<String, AdapterError> {
        let (text, _) = self
            .generate_streaming(tok, prompt, Arc::new(SilentDeltas))
            .await?;
        Ok(text)
    }

    /// 流式生成：经帧桥把 SSE 增量逐片转给 `sink`；返回值 = 全文 + 实测耗时
    ///（收束语义与流式与否无关）。
    ///
    /// **无工具 = [`Self::generate_turn`] 的入参为空的同一路径**（`&[]`）：不另写
    /// 一条请求构造。「空文本」在这里是**失败**（调用方拿到 Ok(空串) 会把「模型没答」
    /// 当「答了空话」落成 final）——判定归这里，因为无工具轮没有别的产出可指望。
    async fn generate_streaming(
        &self,
        tok: &FullModel,
        prompt: &str,
        sink: Arc<dyn DeltaSink>,
    ) -> Result<(String, u64), AdapterError> {
        let turn = self.generate_turn(tok, prompt, &[], sink).await?;
        if turn.text.trim().is_empty() {
            // 空文本不是成功：调用方拿到 Ok(空串) 会把「模型没答」当「答了空话」
            // 落成 final——必须走兜底路径（I3），所以这里按失败返回。
            return Err(AdapterError::GenerationFailed(
                "model returned empty text".into(),
            ));
        }
        Ok((turn.text, turn.cost_ms))
    }

    /// 工具通道：工具清单下行 + 工具调用上行。
    ///
    /// **空文本在工具轮上是合法的**（纯工具调用轮不说话）——所以这里不做空文本
    /// 判定，判定归「本轮收束」的那一侧（`TurnRunner::run_with_tools`）：只有
    /// 「无工具调用 + 空文本」才是失败。
    async fn generate_turn(
        &self,
        _tok: &FullModel,
        prompt: &str,
        tools: &[CapabilityMeta],
        sink: Arc<dyn DeltaSink>,
    ) -> Result<LlmTurn, AdapterError> {
        let started = std::time::Instant::now();
        let message = ChatMessage {
            id: llm_short_id(),
            role: Some(MessageRole::User),
            content: Some(MessageContent::Text(prompt.to_string())),
            ..Default::default()
        };
        // 流式出口：转写帧经桥转成正文增量（桥不落盘——帧面只进回调）；
        // 中止信号直通调用方（会话轮的中止语义不因换执行路径而丢）。
        // 工具调用帧不进 v2 文本面：工具节点的构造权在分发方（`DispatchPort` 实现），
        // 由它按**同一份** `tool_calls` 建节点——两处各建一份必然出现重复卡片。
        let env = ExecEnv::new(
            ExecEventSink::direct(Arc::new(DeltaBridge {
                sink,
                text_nodes: Default::default(),
            })),
            self.abort_signal(),
        );
        let output = self
            .provider
            .execute_turn(
                self.system_prompt(),
                &[message],
                tools,
                &llm_short_id(),
                &env,
            )
            .await
            .map_err(|e| match e {
                // 中止不压成失败：调用方据此**不落兜底格**（ADR-045 同源纪律）。
                PluginError::Aborted => AdapterError::Aborted,
                other => AdapterError::GenerationFailed(format!("{other}")),
            })?;
        Ok(LlmTurn {
            text: output.text,
            tool_calls: output.tool_calls,
            cost_ms: started.elapsed().as_millis() as u64,
        })
    }
}
