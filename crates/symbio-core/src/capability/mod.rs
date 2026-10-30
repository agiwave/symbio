//! 能力登记表、四元组与调用栈（`capability_invoke` 的机制一半）。
//!
//! **实现住 `symbio-sdk`**（D-258）：这一域的每条都只碰内核词汇与自己的表，不认容器概念。
//! 本模块只转发，`symbio_core::capability::…` 这条稳定路径不变。

pub use symbio_sdk::capability::*;
