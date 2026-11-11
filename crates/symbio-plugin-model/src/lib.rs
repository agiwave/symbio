//! Universal Model Agent Engine
//!
//! 职责：**无状态 LLM 网关**。
//! - `types`:           Unified type definitions (ModelProviderConfig, NativeMessage, etc.)
//! - `detail`:          Model 详情页定义（definition-driven detail）
//! - `handlers`:        Non-streaming handlers (status, list_models, config)
//! - `message_builder`: ChatNode → `NativeMessage` 的请求视图构造（只读已有节点，不落库）
//! - `protocols`:       插件私有协议契约 `ModelProtocol`（钩子收 `&ModelProviderConfig`）
//!   + 协议事件方言 `ModelProtocolEvent` + 行解析契约 `SseLineParser`（`protocols/sse`）
//!   + 四协议适配（SSE → 标准化事件流，`ping` 健康检查）
//!   + `resolve_protocol_id` 与 `MODEL_PROTOCOL_*` 注册常量
//! - `provider`:        `ModelProvider`——一次往返的执行契约（**本插件私有**，D-437）
//! - `round`:           一次往返的产物与判决（`RoundOutput` / `ModelFinishReason` /
//!   `ModelUsage` / `ModelSpend`，**本插件私有**，`D-439`）
//! - `bound_provider`:  `BoundProvider`——绑定配置与协议实现，是上面那条契约的唯一实现
//! - `port`:            `ModelPort`——`port.model` 的世界提供者（D-298）：解析与
//!   适配器构造收在本插件，环只按端口名取
//! - `http`:            HTTP 重试机器（客户端单例 + 带中止的 POST，五态 `PostResult`）
//! - `stream`:          SSE 流循环（字节流 → `RoundOutput`，流式子节点实时下发）
//! - `tool_accumulator`: 工具调用增量累积（分片 → 完整调用的状态机，收口为 `RoundOutput.tool_calls`）
//! - `tool_projection`: 端口身份 → wire 别名的唯一实现与本轮暴露投影（`D-469`）
//! - `plugin`:          Core ModelPlugin entry point + factory registration + provider 注册表
//!
//! 边界：会话循环（chat_loop / turn_processor / tool_executor / resume /
//! compression 等）属于 session 插件，model 对 session 零依赖；
//! 协议抽象 `ModelProtocol`、事件方言 `ModelProtocolEvent`、行解析契约
//! `SseLineParser`、执行契约 `ModelProvider` 与一次往返的产物形状
//! （`RoundOutput` / `ModelFinishReason` / `ModelUsage` / `ModelSpend`）都是本插件
//! 私有词汇，架构层不承载它们（`D-437` / `D-439`）——环只按 `port.model` 收事实，
//! 事实的键名才是跨侧契约。

mod assist;
mod bound_provider;
mod debt;
mod detail;
mod http;
/// 本插件写的那两本账（用量 / 金额）：写方在此，折叠也在此（`D-461`）。
mod ledger;
mod message_builder;
mod model_providers;
mod plugin;
mod port;
mod protocols;
mod provider;
mod round;
mod stream;
mod tool_accumulator;
mod tool_projection;

mod types;
mod vocab;
