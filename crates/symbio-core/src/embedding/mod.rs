//! 嵌入服务抽象层（`EmbeddingService` trait 与服务 id）。
//!
//! **实现住 `symbio-sdk`**（D-258）：trait 与 id 是对「怎么把文本变成向量」的抽象，
//! 不认容器概念。本模块只转发，`symbio_core::embedding::…` 这条稳定路径不变。

// 本域实现已整体搬走（D-258 / D-259），这个文件只剩**登记面**（`core-naming-audit` N-004）：
// 仓内没有消费者，对外也不再经根转发（D-272）。下面的转出因此是「地址」而不是「依赖」。
#![allow(unused_imports)]

pub use symbio_sdk::embedding::*;
