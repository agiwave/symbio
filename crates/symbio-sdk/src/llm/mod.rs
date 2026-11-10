//! LLM 契约层 —— 模型往返的**词汇与共享帧原语**（协议无关、插件无关）。
//!
//! ## 为什么整块在这里，而不是内核（`D-438`）
//!
//! 这些类型（`RoundOutput` / `RoundToolCallInfo` / `PromptMessage` / `PromptToolCall` /
//! `ModelFinishReason` / `ModelUsage` / `ModelSpend`）住内核的凭据原本只有一条：
//! `D-188` C2b 说「封闭语法 trait 及其**签名类型**进内核」，而 `ModelProvider` 的签名里
//! 全是它们。`D-437` 把那条 trait 退进 model 插件之后，凭据就不存在了——内核里
//! **没有任何生产代码**再读这些名字（`dialog` 读 `PromptMessage`，而 `dialog` 自己
//! 也在内核里零读者）。按 `kernel/src/lib.rs` 抬头的判据（内核只装语法，**词汇**归
//! `symbio_sdk` 与各 owner），它们的归属是本层。
//!
//! 执行契约不在本层：`ModelProvider` 在 `crates/symbio-plugin-model/src/provider.rs`
//! （模块私有，`D-437`），一次往返的执行面是 `port.model` 的执行体。
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

pub mod model;
pub mod prompt_message;
pub mod round;
pub mod round_output;

// `round` 是**子命名空间**，不是契约的对外地址：插件门面按符号取帧原语与 id 原语。
// 这三个在本层根上转出一次，`symbio_sdk::llm::round::…` 留给本 crate 内部与 core 的
// 往返编排用。（名字取「一次往返」这个词而不是容器词：扩展面连内部路径都不用容器词，
// EX-008 的路径段豁免因此只剩 `D-259` 那一条，见 `D-436`。）
pub use round::{llm_emit_message, llm_removed_frame, llm_short_id};

/// 模型往返的**数据词汇**在本层根上转出一次：插件的导入路径不必认识 `model` /
/// `round_output` / `prompt_message` 三个子模块（`D-438`——它们与帧原语同属一次往返）。
pub use model::*;
pub use prompt_message::*;
pub use round_output::*;
