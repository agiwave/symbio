//! 执行期事件出口的生产实现（[`ExecTranscriptWriter`] → 转写唯一写入点）。
//!
//! ## 为什么在编排层
//!
//! 本文件取代的是**历史上消费循环里的那段分派**：
//!
//! ```text
//! // 收口前（consume.rs）
//! match serde_json::from_value::<NodeOp>(data.clone()) {
//!     Ok(NodeOp::Warn { warning }) => emit_session_state(...),   // 会话级状态
//!     Ok(op)                        => transcript.apply(op),     // 转写面
//!     Err(e)                        => plugin_warn!("无法解析的数据帧"),
//! }
//! ```
//!
//! 分派规则（告警归会话节点、消息归转写）现在由**出口类型本身**表达：
//! `apply` 只收消息（一条 [`ChatMessage`]），`warn` 是 trait 上的独立通道——
//! 两个域在类型层面分离，分派规则无需再被每个调用方记住。
//!
//! ## 与收口前的三点差别
//!
//! 1. **零 serde**：事件从生产者出来就是类型化的消息，直达唯一写入点，
//!    不再「序列化成帧 → 反序列化回来只为分辨变体」；
//! 2. **没有解析失败分支**：类型化之后不存在「无法解析的数据帧」这种降级路径；
//! 3. **写入不再与消费循环同寿**：生产者（`run_chat_loop` 任务）直接写，
//!    消费循环因此可以退化为纯生命周期管理（见 [`super::consume`]）。

use super::*;
use crate::symbio_core::schemas::session::chat_message::ChatMessage;
use crate::symbio_core::ExecTranscriptWriter;
use async_trait::async_trait;

/// 执行期事件出口：直连转写唯一写入点 + 会话级状态分派。
pub(crate) struct TranscriptSink {
    plugin: Arc<SessionPlugin>,
    state: Arc<ActiveSessionState>,
}

impl TranscriptSink {
    pub(crate) fn new(plugin: Arc<SessionPlugin>, state: Arc<ActiveSessionState>) -> Self {
        Self { plugin, state }
    }
}

#[async_trait]
impl ExecTranscriptWriter for TranscriptSink {
    async fn apply(&self, message: ChatMessage) {
        // 消息帧：进唯一写入点（内存图 → seq → 一行核心日志 → 发布）。
        self.state.transcript.lock().await.apply(message);
    }

    async fn warn(&self, warning: Option<String>) {
        // 会话级告警（持久化失败 / 长度截断 / 工具轮次上限）：直通会话节点
        // `attributes.warning`（VDFS watch 域，状态非事件），**不进转写**。
        self.plugin
            .emit_session_state(&self.state, SessionStateChange::Warning(warning))
            .await;
    }
}
