//! MODEL 协议模块 —— Phase A 改造后降级为 re-export shim。
//!
//! 协议契约（`trait ModelProvider`、`FinishReason`、`Usage`、`ProtocolEvent`、
//! `resolve_protocol_id`）已上移到 core：`crate::symbio_core::model_provider`。
//! 本模块仅保留各协议实现（openai_chat / openai_responses / anthropic_messages /
//! gemini_api）。
//!
//! Phase E-②：通用流分发器已随会话引擎迁往 session 插件；协议的连通性
//! 验证统一为 `ModelProvider::ping` 直调，不再经由会话通道收帧。
//!
//! 详见 docs/model-session-refactor.md。

mod anthropic_messages;
mod context_probe;
mod gemini_api;
mod openai_chat;
mod openai_responses;

pub use crate::symbio_core::{resolve_protocol_id, ModelProvider};
