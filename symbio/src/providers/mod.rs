//! 通用服务实现层
//!
//! **本目录只放 `StorageService` 的具体实现，**
//! **不通过 `pub use` 暴露给 crate 外部**。trait 抽象在
//! [`crate::symbio_core::providers`]。
//!
//! ## 可见性规则
//!
//! - 本目录在 `lib.rs` 中以 `pub(crate) mod providers;` 声明
//! - 所有子模块一律 `mod xxx;`（**不带** `pub(crate)`），仅 `providers` 内部可见
//! - 任何业务模块**不允许**直接 `use crate::providers::xxx::...`
//! - 具体实现类型（`FileEntityStore`、`FileStorageService` 等）**不** re-export
//!   业务模块必须通过 `dyn StorageService` trait 引用
//!
//! ## 磁盘布局（统一约定）
//!
//! ```text
//! ~/.symbio/
//! ├── config.yaml                              # 仅 home 自身配置（work / recent_workspaces）
//! └── plugins/                                 # ⭐ 所有插件的实体数据
//!     ├── model/<provider_id>/provider.json    # Model Providers
//!     ├── mcps/<server_name>/server.json       # MCP Servers
//!     ├── skills/<skill_name>/SKILL.md         # Skills
//!     ├── channels/<channel_id>/channel.json   # Channels
//!     ├── agents/<agent_id>/                   # Agents
//!     └── sessions/<session_id>/               # Sessions
//! ```
//!
//! **重要**：所有插件数据都放在 `plugins/` 下，**便于通过遍历
//! `~/.symbio/plugins/` 即可知道加载了哪些插件**。
//!
//! ## 业务模块的正确用法
//!
//! ```ignore
//! // ✅ 正确：通过工厂模式获取 dyn trait
//! use crate::symbio_core::providers::{StorageService, EntityStore, categories, manifests};
//! let store = create_object::<dyn StorageService>("storage_service", ctx)?;
//! let es = store.entity_store();
//! let ids = es.list_entities(categories::MODEL).await?;
//! ```
//!
//! ```ignore
//! // ❌ 错误：直接 use 具体实现类型
//! use crate::providers::storage_service::FileEntityStore;  // 编译错误：私有
//! // ❌ 错误：直接 use 子模块工具
//! use crate::providers::storage_service::path_resolver::safe_id;  // 编译错误：私有
//! ```
//!
//! ## 子模块
//!
//! - `storage_service`
//!   - `path_resolver`：路径解析（ID 安全化）
//!   - `file_entity_store`：`EntityStore` 的文件系统实现
//!   - `service`：`StorageService` 工厂注册
//!
//! ## 关于 workspace_service
//!
//! 历史上曾存在 `workspace_service` 模块（缓存全局"活跃 workdir"）。
//! 现已删除：workdir 始终由前端在每个请求的 ctx.WORKDIR 中显式传递。

// ⭐ 子模块一律不带 `pub(crate)`，保持完全私有。
// 业务模块必须通过 `dyn StorageService` 工厂模式访问。
// 跨模块的 tilde 展开等通用工具，统一使用 `shellexpand::tilde`（与其它插件一致）。
mod embedding;
mod storage_service;
