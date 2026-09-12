//! Universal Model Agent Engine
//!
//! 职责：**无状态 LLM 网关**。
//! - `types`:           Unified type definitions (ModelProviderConfig, NativeMessage, etc.)
//! - `detail`:          Model 详情页定义（definition-driven detail）
//! - `handlers`:        Non-streaming handlers (status, list_models, config)
//! - `message_builder`: NativeMessage 构造与会话持久化（写 session 走存储锚点）
//! - `protocols`:       插件私有协议契约 `ModelProtocol`（钩子收 `&ModelProviderConfig`）
//!   + 四协议适配（SSE → 标准化事件流，`ping` 健康检查）
//!   + `resolve_protocol_id` 与 `MODEL_PROTOCOL_*` 注册常量
//! - `bound_provider`:  `BoundProvider`——绑定配置与协议实现，实现 core 纯
//!   `ModelProvider` trait（session 的唯一模型契约）
//! - `plugin`:          Core ModelPlugin entry point + factory registration + provider 注册表
//!
//! 边界：会话循环（chat_loop / turn_processor / tool_executor / resume /
//! compression 等）属于 session 插件，model 对 session 零依赖；
//! 协议抽象 `ModelProtocol` 是本插件私有契约，core 不承载协议抽象。

mod bound_provider;
mod detail;
mod handlers;
pub mod message_builder;
mod model_providers;
mod plugin;
mod protocols;
mod types;
