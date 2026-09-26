//! LLM 契约层 —— 模型服务的唯一契约面（协议无关、插件无关）。
//!
//! 两个模块各管一段，共同构成 session 与 model 之间的完整契约：
//!
//! | 模块 | 职责 |
//! |---|---|
//! | [`model_provider`] | [`ModelProvider`](self::ModelProvider) trait（session 唯一依赖的模型契约）+ 结束原因与用量（`ModelFinishReason` / `ModelUsage`） |
//! | [`turn`] | 单轮产物（`TurnOutput`）+ 消息帧原语（`emit_*` 家族）+ 消息构造家族（`build_*`） |
//!
//! 依赖关系（单向）：
//!
//! ```text
//! session ──────────────▶ ModelProvider trait（本层）
//! model  ── BoundProvider 实现 ModelProvider
//!        ── execute_turn 内部消费 turn 的帧原语与 TurnOutput
//! ```
//!
//! **只有一侧认的东西都不在这里**：HTTP 重试机器与 SSE 流循环
//! （`plugins/model/http.rs` / `stream.rs`）、行解析契约
//! （`plugins/model/protocols/sse.rs`）、协议事件方言
//! （`plugins/model/protocols/mod.rs` 的 `ModelProtocolEvent`）都只有 model
//! 插件使用，按「依赖方数量」判据（[ADR-023](../../../docs/DECISIONS.md)）住在
//! 该插件内。本层只留**两侧共用**的契约与数据结构。

pub mod model_provider;
pub mod turn;

pub use model_provider::{ModelFinishReason, ModelProvider, ModelUsage};
pub use turn::{
    build_assistant_messages, build_tool_message, emit_delta, emit_message, emit_removed,
    emit_state, message_frame, removed_frame, short_id, state_frame, TurnOutput,
    TurnStreamChildIds, TurnToolCallAccumulator, TurnToolCallInfo,
};
