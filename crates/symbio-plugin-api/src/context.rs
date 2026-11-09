//! 上下文端口：注册表、扩展点与宏整块住 `symbio_sdk::plugin::context`（`D-412`）。
//!
//! 门面只留**路径**：插件照旧写 `symbio_plugin_api::context::…`。`ContextProviderDecl` 的
//! 采集实现只在定义类型的 crate 里生成，与 `ContextProvider` / `ContextSection` 是同一个
//! 类型闭包——拆不开（`D-408` 的三条硬事实，`D-412` 复用同一裁决）。
//! `submit_context_provider!` 与模块同处转出：插件写 `symbio_plugin_api::submit_context_provider!` 不变。
pub use symbio_sdk::plugin::context::*;
pub use symbio_sdk::submit_context_provider;
