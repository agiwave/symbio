//! Event Bus 插件模块
//!
//! 用于把多个插件（session、explorer 等）的事件统一汇聚，
//! 由前端建立一个连接即可订阅所有事件。
//!
//! 归属规则：`EventBus` 全局门面位于 `symbio_core::event_bus`（跨插件共享设施），
//! 各插件统一 `use crate::symbio_core::event_bus::EventBus` 访问，
//! 不得直接引用本模块（保持"插件互不可见"的分层原则）。

mod plugin;
