//! 嵌入服务抽象层（`EmbeddingService` trait 与服务 id）。
//!
//! **实现住 `symbio-sdk`**（D-258）：trait 与 id 是对「怎么把文本变成向量」的抽象，
//! 不认容器概念。本模块只转发，`symbio_core::embedding::…` 这条稳定路径不变。

pub use symbio_sdk::embedding::*;
