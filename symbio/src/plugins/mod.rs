//! 插件实现模块
//!
//! ## 架构原则
//!
//! 按方案 1 + 插件独立原则：
//! - **所有 plugin 子模块都是私有**（`mod xxx;`），插件之间互相不可见，不能直接相互引用。
//! - plugin 之间唯一的交互方式是：
//!   1. **通用对象创建机制**（`submit_object_creator!` + name 常量）——通过插件名查找构造函数
//!   2. **symbio_core 公共接口**（`Plugin` trait / `InvokeRequest` 等）——通过 trait object 交互
//!   3. **symbio_core 共享设施**——跨插件复用的全局服务（如 `event_bus::EventBus`、
//!      `providers::StorageService`）统一放在 `symbio_core`，插件只依赖 `symbio_core`，
//!      不直接依赖其他插件模块。
//! - `lib.rs` 通过 `pub use` 在 `plugins` 模块**之外**重新导出必要的对外契约。
//! - plugin 内部实现细节全部私有（`mod xxx;`）。

mod agent;
mod composite;
mod event_bus;
mod explorer;
mod home;
mod hook;
mod local;
mod mcp;
mod model;
mod session;
mod setting;
mod skill;
mod telegram;
mod web;
