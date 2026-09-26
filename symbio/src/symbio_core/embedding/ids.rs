//! 嵌入服务 id —— 全局对象注册表里的**嵌入服务身份**
//!
//! 与插件工厂 id 同源（`plugin::ids`）：经 `submit_object_creator!` 注册到全局注册表，
//! 业务侧用 `create_object::<dyn EmbeddingService>(EMBEDDING_LOCAL, ctx)` 取实例。
//!
//! ## 命名：`EMBEDDING_<服务名>`
//!
//! 值 = 服务名（小写下划线），标识符加 `EMBEDDING_` 前缀。值本身是跨进程字面量，
//! 标识符只服务编译期——两者分开，理由同 `plugin::ids`。
//!
//! 归属本域的理由：它描述的是「哪个嵌入实现」，与 `EmbeddingService` 是同一概念面。
//! 服务 id 与**实现**分居两处：id 在此（core 抽象层），实现在 `providers/embedding`。

/// 本地嵌入服务（tract 纯 Rust ONNX，`providers/embedding/local.rs`）
pub const EMBEDDING_LOCAL: &str = "embedding_local";
/// noop embedding 服务（占位 / 禁用）
pub const EMBEDDING_NOOP: &str = "noop";
