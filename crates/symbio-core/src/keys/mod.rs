//! 跨插件协议键（`path` / `session_id` / `principal_*` …）。
//!
//! **实现住 `symbio-sdk`**（D-258）：这些是宿主方言的字符串常量，没有一行实现，却也不是
//! 封闭语法（内核只收线格式与闭集）。转发保持 `symbio_core::keys::…` 与控制面根导出不变。

pub use symbio_sdk::keys::*;
