//! 嵌入服务实现层
//!
//! **本目录只放 `EmbeddingService` 的具体实现，不通过 `pub use` 暴露给 crate 外部。**
//! trait 抽象在 [`crate::symbio_core::providers::embedding`]。
//! 各实现通过 `submit_object_creator!` 工厂注册，业务模块通过
//! `create_object::<dyn EmbeddingService>("fastembed", ctx)` 获取实例。

// ⭐ 子模块一律不带 `pub(crate)`，保持完全私有。
// 业务模块必须通过 `dyn EmbeddingService` 工厂模式访问。
mod fastembed;
