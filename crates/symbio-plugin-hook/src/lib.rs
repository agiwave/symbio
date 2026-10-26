//! Hooks 子插件模块
//!
//! 提供 Hook 机制，允许在特定事件点执行自定义逻辑
//! Hook 本质上是一个普通插件，通过 PluginMessage/Response 与其他插件通信

mod executor;
mod plugin;
mod registry;
