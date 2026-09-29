//! Triage 插件模块（对话面：判决）
//!
//! 本模块是**私有**的：插件之间互不可见（见 [`crate::plugins`] 的架构原则），
//! 跨插件调用一律经容器 `route` + `symbio_core::schemas::dialog` 的共享契约。

mod plugin;
