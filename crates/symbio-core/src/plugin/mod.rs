//! 插件目录与配置 IO（`PluginDir` / `PLUGIN.yml` 的读取与落盘）、响应壳、配置挂载。
//!
//! **实现住 `symbio-sdk`**（D-258）：它只对文件与协议键说话，不认轮次容器。本模块只转发，
//! `symbio_core::plugin::…` 这条稳定路径不变。

pub use symbio_sdk::plugin::*;
