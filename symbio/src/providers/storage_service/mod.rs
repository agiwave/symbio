//! 通用文件存储服务实现
//!
//! 本目录只放 `StorageService` 和 `EntityStore` 的**具体实现**，
//! trait 抽象在 `crate::symbio_core::providers::storage`。
//!
//! ## 设计原则
//!
//! - **trait 在 symbio_core**：`StorageService` / `EntityStore` 是核心抽象
//! - **实现在这里**：`FileEntityStore` 是文件系统实现，可替换
//! - **统一工厂**：通过 `submit_object_creator!` 工厂注册
//!
//! ## 可见性
//!
//! 所有子模块一律 `mod xxx;`（不带 `pub(crate)`），仅本目录内部互访。
//! 业务模块只能通过 `dyn StorageService` 工厂访问，禁止 `use crate::providers::...`。

mod file_entity_store;
mod path_resolver;
mod service;
