//! projection 插件 —— **可选**的投影内省口。
//!
//! B2 的投影表是进程内登记机制；本插件给它一个只读窗口（`projection/list` /
//! `projection/run`），供审计 / 调试 / 测试观察"有哪些投影""某个投影跑出什么"。
//!
//! 三条边界（与 `fact_log` 同款）：**可选**（不在装配清单，目录不在则不挂载）、
//! **只读**（只查表 / 跑纯投影）、**零插件依赖**（只经 `symbio_core` 投影 API）。
//!
//! 消费方（检索者 / 巩固者）**不**经本插件取投影——它们直接调
//! [`symbio_core::projection_run`](crate::symbio_core::projection_run)。本插件纯属
//! "人看的口"，与机制面分离。
//!
//! ## 文件
//!
//! - [`plugin`]：插件本体（`Plugin` 实现 + 两条只读路由）

mod plugin;
