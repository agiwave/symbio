//! `ModelProvider` —— 一次模型往返的执行契约，**本插件的私有端口**。
//!
//! ## 为什么它住在这里（D-437）
//!
//! 把它放进内核的理由是「会话引擎与 model 插件之间唯一的模型契约」。那个理由
//! 描述的是一个**不存在的形状**：
//!
//! - 环（`symbio-plugin-loop`）从不认这个 trait。它按端口名 `model` 取一行，
//!   交一份载荷，拿回 `PortOutcome`——`crates/symbio-plugin-api/src/port.rs` 的模块
//!   文档把这条写成了判据：「端口名是**数据**，不是类型」。
//! - 解析、构造、发请求、解析流全在本插件内部（`ModelPort` + `ModelExecutor`）。
//! - 架构层剩下的三个暴露面没有一个有活读者：跨插件直调
//!   （`capability_model_active` / `capability_model_resolve`）零生产调用方；
//!   `ProviderLlmAdapter`（把 trait 折成 `LlmAdapter` 的胶水）只在测试里被构造；
//!   模型行的载荷装着一个 `Arc<dyn ModelProvider>`，而读它的两个函数就是前一句那对。
//!
//! 于是四样东西一起离场：trait 落在这里，行只留目录事实，直调面与桥删除，执行面
//! 只有 `PortExecutor` 一条（`D-437`）。
//!
//! ## 判据（一件事只有一个 owner）
//!
//! - 架构层不再有模型调用词汇：数据词汇（`RoundOutput` / `ModelFinishReason` /
//!   `ModelUsage` / `ModelSpend` / `PromptMessage`），它们跨侧共用、不是「按世界命名的
//!   trait」，`CS-001` 因此从五个内核 trait 收到四个；
//! - 本 trait 不出本 crate：`pub(crate)` 都不需要，模块本身是私有的
//!   （`lib.rs` 的 `mod provider;`）——插件外没有任何合法读者；
//! - 环的判据 I-6（生产面不得出现按世界命名的 trait）因此从「靠豁免维持」变成
//!   「结构上不可表达」：那个名字已经不在它依赖的任何一个 crate 里。
//!
//! 哪天真需要第二个实现方（比如把模型执行拆成独立进程），正确动作是把
//! `PortExecutor` 的实现搬过去，而不是把这个 trait 再抬回公共面——端口是**数据面
//! 的接缝**，不是类型面的接缝。

use async_trait::async_trait;
use symbio_plugin_api::chat_node::ChatNode;
use symbio_plugin_api::{CapabilityMeta, ExecEnv, PluginError, RoundOutput};

/// 一次模型往返的执行契约（本插件私有）。
///
/// 实现体只有一个：[`crate::bound_provider::BoundProvider`]（持久化配置
/// `ModelProviderConfig` + 协议钩子 `Arc<dyn ModelProtocol>` 的绑定）。
///
/// 面只开实现体真正需要被外部按 trait 调用的那些：
/// - **身份/参数**：`provider_id`（目录键与日志归组）、`max_context_tokens`；
/// - **上下文收敛**：`effective_context_tokens`（服务端上报值与用户设置取 min）；
/// - **单轮执行**：`round_trip`（构造请求 → 带中止的 POST → SSE 流解析 →
///   `RoundOutput`），协议差异由实现体内部的协议钩子吸收。
#[async_trait]
pub trait ModelProvider: Send + Sync {
    /// Provider 唯一标识（目录键；限流器与日志按此归组）
    fn provider_id(&self) -> &str;

    /// 用户设置的上下文窗口上限（生效值见 [`Self::effective_context_tokens`]）
    fn max_context_tokens(&self) -> u32;

    /// 计算生效上下文 token 上限：`min(用户设置, 服务上报)`。
    ///
    /// 服务端能主动上报上限时（本地模型常见）取两者较小值，避免请求超限；
    /// 否则（云端 API 普遍不暴露）直接使用用户设置。
    async fn effective_context_tokens(&self) -> u32;

    /// 执行完整单轮 LLM 请求：构造请求体 → 带中止的 POST → SSE 流解析。
    ///
    /// 五态匹配（`Aborted` / `RetryWithoutContextId` / `Err` / `RateLimited` / `Ok`）
    /// 与 SSE 解析由实现体提供，使任何实例都自带完整的「请求-解析」能力，
    /// `ModelExecutor` 因此只需要解载荷、取输入、落事实。
    ///
    /// ## 名字为什么是「往返」而不是「轮」
    ///
    /// 它的语义是**一次模型往返的产物**（`RoundOutput`）；轮是环的身份，不是端口
    /// （`D-222` 裁决 7）。方法名与返回类型现在说的是同一件事：`round_trip` 交
    /// `RoundOutput`（`D-436`）。
    ///
    /// `env` 与 [`symbio_plugin_api::Capability::execute`] 共用同一类型
    /// （[`ExecEnv`]）：两者都是「一次带中止的流式执行」——出方向写事件、
    /// 入方向读中止。模型执行不被路由，因此没有 `ctx` 入参。
    async fn round_trip(
        &self,
        system_prompt: &str,
        messages: &[ChatNode],
        tools: &[CapabilityMeta],
        root_id: &str,
        env: &ExecEnv,
    ) -> Result<RoundOutput, PluginError>;
}
