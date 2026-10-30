//! 模型层的宿主侧小工具（短 id、消息格序、移除帧）。
//!
//! **实现住 `symbio-sdk`**（D-258）：它只对消息与 id 说话，不认轮次容器。本模块只转发，
//! `symbio_core::llm::…` 这条稳定路径不变。

pub use symbio_sdk::llm::*;
