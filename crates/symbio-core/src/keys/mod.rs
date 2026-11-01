//! 跨插件协议键（`path` / `session_id` / `principal_*` …）。
//!
//! **实现住 `symbio-sdk`**（D-258）：这些是宿主方言的字符串常量，没有一行实现，却也不是
//! 封闭语法（内核只收线格式与闭集）。转发保持 `symbio_core::keys::…` 与控制面根导出不变。

// 本域实现已整体搬走（D-258 / D-259），这个文件只剩**登记面**（`core-naming-audit` N-004）：
// 仓内没有消费者，对外也不再经根转发（D-272）。下面的转出因此是「地址」而不是「依赖」。
#![allow(unused_imports)]

pub use symbio_sdk::keys::*;
