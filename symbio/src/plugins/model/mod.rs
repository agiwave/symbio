//! Universal Model Agent Engine
//!
//! Phase E-② 定型后职责：**无状态 LLM 网关**。
//! - `types`:           Unified type definitions (ModelConfig, NativeMessage, etc.)
//! - `detail`:          Model 详情页定义（definition-driven detail）
//! - `handlers`:        Non-streaming handlers (status, list_models, config)
//! - `message_builder`: NativeMessage 构造与会话持久化（写 session 走存储锚点）
//! - `protocols`:       四协议适配（SSE → 标准化事件流，`ping` 健康检查）
//! - `plugin`:          Core ModelPlugin entry point + factory registration + provider 注册表
//!
//! Phase E-②：会话循环（chat_loop / turn_processor / tool_executor / resume /
//! compression 等）已整体迁往 session 插件，model 对 session 零依赖。

mod detail;
mod handlers;
pub mod message_builder;
mod plugin;
mod protocols;
mod types;
