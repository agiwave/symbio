//! Event Bus 插件模块
//!
//! 用于把多个插件（session、explorer 等）的事件统一汇聚，
//! 由前端建立一个连接即可订阅所有事件。
//!
//! 注意：`EventBus` 全局门面已上移至 `symbio_core::event_bus`，
//! 各插件应 `use crate::symbio_core::event_bus::EventBus` 访问，
//! 不再直接引用本模块（保持"插件互不可见"的分层原则）。

mod plugin;
