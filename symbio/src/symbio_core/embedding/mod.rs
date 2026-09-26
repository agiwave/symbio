//! 嵌入服务抽象（symbio_core 层）
//!
//! 这是 `providers/`（具体实现层）之上的**抽象接口层**：trait 在此定义，
//! 具体实现（Local / Noop 等）放在 `src/providers/embedding`，
//! 并通过通用对象创建机制自注册到 `local` / `noop` id，
//! 业务模块用 `create_object::<dyn EmbeddingService>(EMBEDDING_LOCAL, ctx)` 获取实例。
//!
//! 所有插件通过 `dyn EmbeddingService` 访问嵌入能力，
//! **不**直接引用 `crate::providers::embedding::LocalEmbeddingService` 等具体实现。
//!
//! ## 设计原则
//!
//! - **抽象在 symbio_core**：所有可插拔服务的 trait 都在这里定义
//! - **实现在 `providers/`**：trait 的具体实现放在 `src/providers/`
//! - **统一工厂**：所有服务的实例都通过 `create_object::<dyn XXXService>(...)` 获取
//! - **不依赖具体实现**：业务模块只 `use` 这里的 trait，不 `use` `providers::xxx` 的具体类型
//!
//! 这里只保留**存在第二种实现**的服务抽象。历史上的 `storage`
//! （`StorageService` / `EntityStore`）已随实体机制一并废除：资源存储的
//! 唯一机制面是 `VdfsProvider`，其集中实现见 `crate::providers::vdfs_service`
//! （插件直接组合具体类型，不经工厂查表）。
//!
//! ## workdir 无服务端缓存
//!
//! workdir 始终由前端在每个请求的 ctx.WORKDIR 中显式传递，
//! 后端不维护全局"活跃 workdir"缓存，因此不设 `WorkspaceService` 抽象。

mod ids;

pub use ids::{EMBEDDING_LOCAL, EMBEDDING_NOOP};

use async_trait::async_trait;
use thiserror::Error;

/// 嵌入服务错误
#[derive(Debug, Error)]
pub enum EmbeddingError {
    #[error("嵌入模型初始化失败: {0}")]
    Init(String),
    #[error("嵌入失败: {0}")]
    Embed(String),
}

/// 嵌入服务：把一段文本映射为稠密向量，用于语义检索 / 记忆查找。
///
/// 是否真正支持语义检索由实现决定——不支持时（如 `NoopEmbeddingService`）
/// 上层应降级到精确名称匹配（例如 `codebase_search` 退化为 ripgrep 关键词检索）。
#[async_trait]
pub trait EmbeddingService: Send + Sync {
    /// 把一段文本编码为稠密向量
    async fn embed(&self, text: &str) -> Option<Vec<f32>>;
}
