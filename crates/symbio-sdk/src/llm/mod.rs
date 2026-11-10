//! LLM 契约层 —— 模型服务的唯一契约面（协议无关、插件无关）。
//!
//! 模型往返的**数据词汇**住在内核（`D-188` C2b 的残留）：`ModelFinishReason` /
//! `ModelUsage` / `ModelSpend` 在 `symbio_kernel::llm`，单轮产物 `RoundOutput` /
//! `RoundToolCallInfo` 在 `symbio_kernel::llm::round`；执行契约 `ModelProvider`
//! 已随 `D-437` 退回 model 插件（本层不再有模型端口）。本模块只留**有实现的共享帧原语**：
//! `llm_emit_message` / `llm_message_frame` / `llm_removed_frame` 与 id 原语
//! `llm_short_id`——它们的消费方在两个以上，且构造即约定（完整消息必带状态）。
//!
//! **只被一个插件消费的部分不在本层**（D-023 的依赖方数量判据）：
//! 落库视图与消息构造（`llm_build_assistant_messages` / `llm_build_tool_message` /
//! `RoundStreamChildIds` / `RoundOutput` 的三个方法）在 `crates/symbio-plugin-loop/src/message_build.rs`，
//! 增量帧 `llm_emit_delta` 在 `crates/symbio-plugin-model/src/stream.rs`。留守的三个导出
//! （`llm_emit_message` / `llm_removed_frame` / `llm_short_id`）逐个都有两个以上消费方
//! （`llm_message_frame` 是登记在案的例外）。
//!
//! 依赖关系（单向）：
//!
//! ```text
//! loop   ──────────────▶ port.model（端口名是数据，环不认识 provider 类型）
//! model  ── ModelExecutor 实现 PortExecutor
//!        ── round_trip 内部消费本层的帧原语与 RoundOutput
//! ```
//!
//! **只有一侧认的东西都不在这里**：HTTP 重试机器与 SSE 流循环
//! （`crates/symbio-plugin-model/src/http.rs` / `stream.rs`）、行解析契约
//! （`crates/symbio-plugin-model/src/protocols/sse.rs`）、协议事件方言
//! （`crates/symbio-plugin-model/src/protocols/mod.rs` 的 `ModelProtocolEvent`）都只有 model
//! 插件使用，按「依赖方数量」判据（D-023）住在
//! 该插件内。本层只留**多消费方**共用的契约与数据结构。

pub mod round;

// `round` 是**子命名空间**，不是契约的对外地址：插件门面按符号取帧原语与 id 原语。
// 这三个在本层根上转出一次，`symbio_sdk::llm::round::…` 留给本 crate 内部与 core 的
// 往返编排用。（名字取「一次往返」这个词而不是容器词：扩展面连内部路径都不用容器词，
// EX-008 的路径段豁免因此只剩 `D-259` 那一条，见 `D-436`。）
pub use round::{llm_emit_message, llm_removed_frame, llm_short_id};
