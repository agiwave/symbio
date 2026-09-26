//! 通用服务实现层
//!
//! 各可插拔服务的**具体实现**在这里。抽象（trait）在 [`crate::symbio_core::embedding`]
//! 与 [`crate::symbio_core::vdfs`]。
//!
//! ## 子模块
//!
//! - `embedding`：[`crate::symbio_core::EmbeddingService`] 的实现
//!   （经工厂 `creator_create_object::<dyn EmbeddingService>(...)` 取用）
//! - `vdfs_service`：**基于 `VdfsProvider` 接口的集中实现**（单文件 / 目录 / 内存）
//!
//! ## 两种接线方式，各自说清理由
//!
//! - **可替换的宿主服务**走工厂：trait 在 core、实现在本层、业务模块只 `use` trait，
//!   例如 `embedding`。这类服务的价值正是「换实现不改调用方」。
//! - **不存在第二种实现的机制底座**直接组合具体类型：`vdfs_service` 的三个实现
//!   本身就是 `VdfsProvider`（接口在 core 已定，不会换），再套一层 `dyn` 工厂只是
//!   把一次构造调用换成一次字符串查表。因此业务模块**直接**
//!   `use crate::providers::vdfs_service::{DirVdfs, SingleFileVdfs, MemoryVdfs}`。
//!
//! ## 磁盘布局（统一约定）
//!
//! ```text
//! ~/.symbio/
//! ├── PLUGIN.yml                               # 系统级插件（home）自身配置
//! ├── <插件>/                                  # ⭐ 一个插件 = 一个目录（配置 + 数据同处）
//! │   ├── PLUGIN.yml                           # 该插件自己的配置
//! │   └── <id>/<主文件>                         # model/<id>/provider.json · mcp/<id>/server.json
//! └── session/<id>/session.json               # Session（自有 store，不经 vdfs_service）
//! ```
//!
//! **重要**：一个插件 = 系统根下的一个目录，**便于通过遍历
//! `~/.symbio/` 即可知道加载了哪些插件**。
//!
//! workdir 不在本层持有：它始终由前端在每个请求的 `ctx.WORKDIR` 中显式传递。

mod embedding;
pub(crate) mod vdfs_service;
