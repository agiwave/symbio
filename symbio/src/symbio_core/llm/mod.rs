//! LLM 契约层 —— 模型服务的唯一契约面（协议无关、插件无关）。
//!
//! 三个模块各管一段，共同构成 session 与 model 之间的完整契约：
//!
//! | 模块 | 职责 |
//! |---|---|
//! | [`model_provider`] | [`ModelProvider`](self::ModelProvider) trait（session 唯一依赖的模型契约）+ 协议事件方言（[`ProtocolEvent`](self::ProtocolEvent) / `FinishReason` / `Usage`） |
//! | [`sse`] | SSE 行解析契约（[`SseLineParser`](self::SseLineParser)，model 协议适配器实现） |
//! | [`turn`] | 单轮产物（`TurnOutput`）+ 消息帧原语（`emit_*` 家族）+ 消息构造家族（`build_*`） |
//!
//! 依赖关系（单向）：
//!
//! ```text
//! session ──────────────▶ ModelProvider trait（本层）
//! model  ── BoundProvider 实现 ModelProvider
//!        ── 协议适配器实现 SseLineParser（本层契约）
//!        ── execute_turn 内部消费 turn 的帧原语与 TurnOutput
//! ```
//!
//! **HTTP 重试机器与 SSE 流循环不在这里**：它们只有 model 插件的
//! `execute_turn` 使用（实现细节而非契约），住在 `plugins/model/`
//! （`http.rs` / `stream.rs`）。本层只留契约与共享数据结构。

pub mod model_provider;
pub mod sse;
pub mod turn;

pub use model_provider::{FinishReason, ModelProvider, ProtocolEvent, Usage};
pub use sse::{PartialLineExtractor, SseLineParser};
pub use turn::{
    build_assistant_messages, build_tool_message, emit_delta, emit_message, emit_removed,
    emit_state, removed_frame, short_id, state_frame, message_frame, StreamChildIds,
    ToolCallAccumulator, ToolCallInfo, TurnOutput,
};
