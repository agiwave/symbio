//! MODEL 协议模块 —— 降级为 re-export shim。
//!
//! 协议契约（`trait ModelProtocol`，原 `ModelProvider` trait）与公共类型
//! （`FinishReason`、`Usage`、`ProtocolEvent`、`resolve_protocol_id`）位于
//! core：`crate::symbio_core::model_provider`。本模块仅保留各协议实现
//! （openai_chat / openai_responses / anthropic_messages / gemini_api）。
//!
//! 通用流分发器已随会话引擎迁往 session 插件；协议的连通性验证统一为
//! `ModelProtocol::ping` 直调，不再经由会话通道收帧。

mod anthropic_messages;
mod context_probe;
mod gemini_api;
mod openai_chat;
mod openai_responses;

pub use crate::symbio_core::{resolve_protocol_id, ModelProtocol};
