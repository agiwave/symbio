//! LLM 契约层 —— 模型服务的唯一契约面（协议无关、插件无关）。
//!
//! 两个模块各管一段，共同构成 session 与 model 之间的完整契约：
//!
//! | 模块 | 职责 |
//! |---|---|
//! | [`model_provider`] | [`model_provider::ModelProvider`] trait（session 唯一依赖的模型契约）+ 结束原因与用量（`ModelFinishReason` / `ModelUsage`） |
//! | [`turn`] | 单轮产物（`TurnOutput` · `TurnToolCallInfo`）+ 共享帧原语（`llm_emit_message` / `llm_message_frame` / `llm_removed_frame`）与 id 原语 `llm_short_id` |
//!
//! **只被一个插件消费的部分不在本层**（D-023 的依赖方数量判据）：
//! 落库视图与消息构造（`llm_build_assistant_messages` / `llm_build_tool_message` /
//! `TurnStreamChildIds` / `TurnOutput` 的三个方法）在 `crates/symbio-plugin-loop/src/message_build.rs`，
//! 状态帧与删除帧的发射口在 `crates/symbio-sdk/src/llm/frames.rs`，增量帧 `llm_emit_delta`
//! 在 `crates/symbio-plugin-model/src/stream.rs`。留守的六个导出逐个都有两个以上消费方
//! （`llm_message_frame` 是登记在案的例外）。
//!
//! 依赖关系（单向）：
//!
//! ```text
//! session ──────────────▶ ModelProvider trait（本层）
//! model  ── BoundProvider 实现 ModelProvider
//!        ── round_trip 内部消费 turn 的帧原语与 TurnOutput
//! ```
//!
//! **只有一侧认的东西都不在这里**：HTTP 重试机器与 SSE 流循环
//! （`crates/symbio-plugin-model/src/http.rs` / `stream.rs`）、行解析契约
//! （`crates/symbio-plugin-model/src/protocols/sse.rs`）、协议事件方言
//! （`crates/symbio-plugin-model/src/protocols/mod.rs` 的 `ModelProtocolEvent`）都只有 model
//! 插件使用，按「依赖方数量」判据（D-023）住在
//! 该插件内。本层只留**多消费方**共用的契约与数据结构。

pub mod model_provider;
pub mod turn;

mod frames;

pub use frames::{llm_emit_state, llm_state_frame};

// `turn` 是**子命名空间**，不是契约的对外地址：插件门面按符号取帧原语与 id 原语，
// 路径里不该出现「一轮」这个词——那是控制面的容器概念（D-252 的 EX-008 判的就是
// 扩展面的这条边界）。所以这三个在本层根上转出一次，`symbio_sdk::llm::turn::…`
// 留给本 crate 内部与 core 的轮次编排用。
pub use turn::{llm_emit_message, llm_removed_frame, llm_short_id};
