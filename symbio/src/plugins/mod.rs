//! 插件实现模块
//!
//! ## 架构原则
//!
//! 插件独立原则：
//! - **所有 plugin 子模块都是私有**（`mod xxx;`），插件之间互相不可见，不能直接相互引用。
//! - plugin 之间唯一的交互方式是：
//!   1. **通用对象创建机制**（`submit_object_creator!` + name 常量）——通过插件名查找构造函数
//!   2. **symbio_core 公共接口**（`Plugin` trait / `PluginInvokeRequest` 等）——通过 trait object 交互
//!   3. **symbio_core 共享设施**——跨插件复用的全局服务（如 `event_bus::EventBus`、
//!      `vdfs::VdfsProvider`）统一放在 `symbio_core`，插件只依赖
//!      `symbio_core`，不直接依赖其他插件模块。
//! - `lib.rs` 通过 `pub use` 在 `plugins` 模块**之外**重新导出必要的对外契约。
//! - plugin 内部实现细节全部私有（`mod xxx;`）。
//!
//! ## 插件内部分层（规范不是机制）
//!
//! - **单向**：`plugin.rs`（插件根，及各插件自己的编排目录，如 `session` 的
//!   `orchestrator/` `chat_loop/`）是编排层，其余文件是领域层。编排层可引用
//!   领域层；领域层不得反向 `use` 编排层的**函数 / 常量**。
//! - **豁免只认两条**：① 插件 trait **类型本身**——`impl XxxPlugin` 允许分布
//!   在插件根之外的模块（`session` 的 `commands.rs` / `heartbeat` / `options`、
//!   `agent` 的 `host/vdfs.rs` 皆如此，本仓一贯）；② 各插件自己模块文档里
//!   **登记过的存量豁免**（如 `session/docs/module-layout.md` §1.3 的 8 处，
//!   每条带归位方向，只减不增）。
//! - ⚠️ 本条当前**没有判定审计**（2026-09-27 排查：除上述豁免外，领域层对
//!   编排层的引用为 0，靠评审把持）。若将来建守卫，必须是**全仓规则**
//!   （所有插件同一判据），不为单个插件立脚本。

mod agent;
mod composite;
mod event_bus;
mod gateway;
mod home;
mod hook;
mod local;
mod mcp;
mod model;
mod plugin_manager;
mod session;
mod skill;
mod telegram;
mod vdfs;
mod web;
mod work;
