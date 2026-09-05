//! agent —— OAB（Open Agent Bundle）宿主插件（OAB 协议的 symbio 接入方）。
//!
//! ## 定位
//!
//! 本插件是 **OAB 规范的第一个参考实现（第一接入方）**，不是协议的必要组成。
//! 协议才是核心：见 `docs/design/open-agent-bundle-spec.md`。
//!
//! ## 模块结构
//!
//! - [`core`]：OAB 协议核心（**零 symbio_core 依赖**，可整体抽出为独立规范库）
//!   - `core::spec`：manifest 数据结构 + 加载期校验 + 约定目录装配（`assembly`）
//! - [`host`]：symbio 适配层（薄）——把约定目录装配映射到 traverse/Capability
//!   机制，提供 bundle 存储、内置工具执行器与管理路由

pub mod core;
pub mod host;
