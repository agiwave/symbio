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
//! - `storage`：`EntityStore` / `StorageService` 抽象 + 业务常量
//!
//! ## 关于 workspace
//!
//! 历史上曾存在 `WorkspaceService` 抽象（用于缓存全局"活跃 workdir"）。
//! 现已删除：workdir 始终由前端在每个请求的 ctx.WORKDIR 中显式传递，
//! 不需要在后端再维护一份全局缓存。

mod embedding;
mod storage;

pub use embedding::{EmbeddingError, EmbeddingService};
pub use storage::{categories, manifests, EntityStore, EntityStoreError, StorageService};
