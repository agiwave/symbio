//! Symbio SDK：**插件可以依赖的宿主机制层**（DE-023 D-258）
//!
//! ## 这一层治什么
//!
//! `symbio-plugin-api` 的门面按来源分两臂之后（D-255），机制臂还剩 115 条——每一条都指向
//! core 的一个宿主子系统（`PluginDir` 的配置 IO、`EventBus` 的订阅表、`capability_invoke`
//! 的调用栈、`principal` 的授权表）。那些子系统**不认容器概念**：它们只碰内核的词汇
//! （`PluginInvokeRequest` / `Event` / `VdfsChange` / `Store`）与自己的表，却住在 core 里，
//! 于是「插件要用一条机制」和「插件要认识整个宿主」被绑成同一件事。
//!
//! 本 crate 把这些机制搬出来，只依赖 `symbio-kernel`：插件门面从此 = 语法（kernel）+
//! 机制（sdk），**core 退出插件的依赖图**。core 保留控制面运行时（主体策略、轮次编排、
//! 声明的装配与校验），继续依赖 kernel + sdk。
//!
//! ## 三层判据（照 D-256 / D-257 的口径）
//!
//! | 层 | 收什么 | 判据 |
//! |---|---|---|
//! | `symbio-kernel` | 封闭语法：形状、闭集、线格式常量 | **没有一行实现**，加字段是数据变更 |
//! | `symbio-sdk`（本 crate） | 插件要用的宿主机制：配置 IO、事件总线、地址装配、授权判定、日志 | 有实现，但**不认容器概念**（不认识 `TurnSpec` / `Phase` / `Session` 这类控制面词） |
//! | `symbio-core` | 控制面运行时：主体策略、轮次编排、装配与校验 | 认识容器概念（DE-026 的影响类） |
//!
//! 「认不认识容器概念」是这一条的判据，不是「在哪一层被用到」：`vdfs_watch_changes` 有实现，
//! 但它只对地址与变更说话，所以它在 SDK；`turnstate` 也是纯类型，但它描述的是**一轮**，
//! 所以它留在 core 的上层（由控制面持有，D-197）。
//!
//! ## 根为什么平铺内核词汇
//!
//! 本 crate 的模块沿用 core 的 `use super::*;` 取词汇（那是它们搬过来之前的写法），而它们要的
//! 词汇就是内核的封闭语法。根平铺 `symbio_kernel::*` 让这 20 多个模块一次搬完而不必逐个改写
//! import，代价是 `symbio_sdk::<符号>` 也能写到内核的类型——**门面的白名单仍然逐条列出**，
//! 插件看不到这份平铺（HS-001 判的是插件的依赖面，不是 sdk 的内部写法）。
//!
//! `inventory` 同样出根：`submit_object_creator!` 宏里写的是 `$crate::inventory::submit!`，
//! 而 `$crate` 展开成本 crate——宏的使用方不必依赖 `inventory`。

pub use inventory;
pub use symbio_kernel::*;

pub mod adapters;
pub mod agent;
pub mod capability;
pub mod chat;
pub mod clock;
pub mod creator;
pub mod embedding;
pub mod event_bus;
pub mod keys;
pub mod llm;
pub mod logger;
pub mod plugin;
pub mod text;
pub mod vdfs;
