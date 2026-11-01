//! 插件目录与配置 IO（`PluginDir` / `PLUGIN.yml` 的读取与落盘）、响应壳、配置挂载。
//!
//! **实现住 `symbio-sdk`**（D-258）：它只对文件与协议键说话，不认轮次容器。本模块只转发，
//! `symbio_core::plugin::…` 这条稳定路径不变。

// 本域实现已整体搬走（D-258 / D-259），这个文件只剩**登记面**（`core-naming-audit` N-004）：
// 仓内没有消费者，对外也不再经根转发（D-272）。下面的转出因此是「地址」而不是「依赖」。
#![allow(unused_imports)]

pub use symbio_sdk::plugin::*;
