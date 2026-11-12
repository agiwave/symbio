//! LLM 契约层 —— 模型往返的**请求词汇与共享帧原语**（协议无关、插件无关）。
//!
//! ## 一次往返的两个方向，只有一个住在这里
//!
//! | 方向 | 形状 | 读者 | 住哪 |
//! |---|---|---|---|
//! | **请求** | `PromptMessage` / `PromptToolCall` | `classify` 造、`tool` 分发、`model` 消费（三个插件） | 本层 |
//! | **产物** | `RoundOutput` / `RoundToolCallInfo` / `ModelFinishReason` / `ModelUsage` / `ModelSpend` | 只有 `model` | `crates/symbio-plugin-model/src/round.rs`（`D-439`） |
//!
//! 判据是 `D-023` 的**依赖方数量**，不是「够不够底层」。产物那五个类型在
//! `port.model` 之前确实是两侧共用（会话侧直接持有 `RoundOutput`，甚至以
//! `impl RoundOutput` 的形式在 loop 里有三个读方法）；端口落地后会话侧改读执行体
//! 落下的**事实键**（`usage` / `spend` / `reason` / `truncated`），全仓再没有
//! `impl RoundOutput`，于是它们退回插件内部——`D-438` 把它们放到本层时的那条理由，
//! 是端口之前的形状。
//!
//! ## 请求词汇为什么在这里，而不是内核（`D-438`）
//!
//! 它们住内核的凭据原本只有一条：`D-188` C2b 说「封闭语法 trait 及其**签名类型**进
//! 内核」，而 `ModelProvider` 的签名里全是它们。`D-437` 把那条 trait 退进 model 插件
//! 之后凭据就不存在了——内核里**没有任何生产代码**再读这些名字。按 `kernel/src/lib.rs`
//! 抬头的判据（内核只装语法，**词汇**归 `symbio_sdk` 与各 owner），它们的归属是本层。
//!
//! 执行契约不在本层：`ModelProvider` 在 `crates/symbio-plugin-model/src/provider.rs`
//! （模块私有，`D-437`），一次往返的执行面是 `port.model` 的执行体。
//!
//! ## 帧原语为什么在这里
//!
//! **只被一个插件消费的部分不在本层**（同一条依赖方数量判据）：落库视图与消息构造在
//! `crates/symbio-plugin-loop/src/message_build.rs`，增量帧 `llm_emit_delta` 在
//! `crates/symbio-plugin-model/src/stream.rs`。留守的三个导出
//! （`llm_emit_message` / `llm_removed_frame` / `llm_short_id`）逐个都有两个以上消费方
//! （`llm_message_frame` 是登记在案的例外）。
//!
//! 依赖关系（单向）：
//!
//! ```text
//! loop   ──────────────▶ port.model（端口名是数据，环不认识 provider 类型）
//! model  ── 实现 PortExecutor；请求词汇取自本层，产物形状在它自己的 `round`
//! ```
//!
//! **只有一侧认的东西都不在这里**：HTTP 重试机器与 SSE 流循环
//! （`crates/symbio-plugin-model/src/http.rs` / `stream.rs`）、行解析契约
//! （`crates/symbio-plugin-model/src/protocols/sse.rs`）、协议事件方言
//! （`crates/symbio-plugin-model/src/protocols/mod.rs` 的 `ModelProtocolEvent`）都只有 model
//! 插件使用，按「依赖方数量」判据住在该插件内。本层只留**多消费方**共用的请求词汇
//! 与帧原语。

pub mod round;

// `round` 是**子命名空间**，不是契约的对外地址：插件门面按符号取帧原语与 id 原语。
// 这三个在本层根上转出一次，`symbio_sdk::llm::round::…` 留给本 crate 内部与 core 的
// 往返编排用。（名字取「一次往返」这个词而不是容器词：扩展面连内部路径都不用容器词，
// EX-008 的路径段豁免因此只剩 `D-259` 那一条，见 `D-436`。）
pub use round::{llm_emit_message, llm_removed_frame, llm_short_id};

// 请求词汇（`PromptMessage` / `PromptToolCall`）已迁契约层（`D-486`）：sdk 内部不再
// 读它们（projection 只有注释引用），门面的契约臂从 `symbio_contract::llm` 直接转出。
