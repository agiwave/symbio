//! 通用服务抽象层
//!
//! 这是 `providers/`（具体实现层）之上的**抽象接口层**。
//! 所有具体的可插拔服务（存储、未来的网络、日志等）都在这里
//! 定义 trait，crate 内的所有模块（`plugins/*`、`providers/*`）通过
//! 这些 trait 引用服务。
//!
//! ## 设计原则
//!
//! - **抽象在 symbio_core**：所有可插拔服务的 trait 都在这里定义
//! - **实现在 `providers/`**：trait 的具体实现放在 `src/providers/`
//! - **统一工厂**：所有服务的实例都通过 `create_object::<dyn XXXService>(...)` 获取
//! - **不依赖具体实现**：业务模块只 `use` 这里的 trait，不 `use` `providers::xxx` 的具体类型
//!
//! ## 子模块
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

mod embedding;

pub use embedding::{EmbeddingError, EmbeddingService};
