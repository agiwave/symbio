//! 模型端口（`LlmAdapter` / `ModelProvider`）与其实现骨架、增量汇（`DeltaSink`）。
//!
//! **实现住 `symbio-sdk`**（D-258）：这一域的每条都只碰内核词汇与自己的表，不认容器概念。
//! 本模块只转发，`symbio_core::adapters::…` 这条稳定路径不变。

pub use symbio_sdk::adapters::*;
