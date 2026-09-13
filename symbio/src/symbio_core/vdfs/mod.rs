//! VDFS（Virtual Dynamic File System）—— 统一的资源与数据访问层
//!
//! 规范：`docs/design/vdfs.md`。
//!
//! ## 模块边界（重要）
//!
//! ```text
//! symbio_core/vdfs_provider.rs   纯接口：VdfsProvider trait + 域类型 + 路径工具
//!                                + 组合视图 VdfsMountTable（容器聚合的通用机制）
//! symbio_core/vdfs/host.rs       symbio 桥：上下文注入 + 错误翻译
//! plugins/vdfs/protocol.rs       线路信封：vdfs/* 请求响应 + 协议路径常量
//! plugins/vdfs/host.rs           访问层：取「根」+ 翻译操作 + 树遍历 + 事件投递
//! plugins/composite/vdfs.rs      拓扑：聚合子插件挂载 + 注册为 VDFS 根
//! ```
//!
//! **core 只暴露纯接口**（[`crate::symbio_core::vdfs_provider`]），线上形状与
//! 访问层都在 vdfs 插件内部——与 [`crate::symbio_core::model_provider`] 的组织
//! 方式一致（core = 纯 trait；协议适配在插件）。
//!
//! ## 谁拥有拓扑
//!
//! 虚拟根 `/` 归**组合容器**所有：`composite` 用 [`VdfsMountTable`] 把自己的子插件
//! 聚合为根视图，并在 `traverse` 里经 `CapabilityVisitor::register_vdfs_root` 登记。
//! vdfs 插件只取这个根再转发（`get_vdfs_root`），因此**拓扑知识不在访问层**。
//!
//! 每个实现了 [`VdfsProvider`] 的插件，以**自己的插件名**作为挂载名出现在根下，
//! 成为一级子目录——「新增资源 = 新挂载点」，访问层与前端都无需改动。
//!
//! [`VdfsMountTable`]: crate::symbio_core::vdfs_provider::VdfsMountTable
//! [`VdfsProvider`]: crate::symbio_core::vdfs_provider::VdfsProvider
//!
//! 本模块（`vdfs`）是 symbio 侧的**薄桥**：把纯接口接到
//! `InvokeRequest` / `PluginError` 上，供实现方（如 `setting` 插件）复用。
//! 换宿主只需重写这一个文件。
//!
//! ## 三条不变量
//!
//! 1. **一个协议**：所有资源只用 `vdfs/*` 一组操作访问，不新造私有协议。
//! 2. **provider 自持**：读写校验、状态、呈现描述都由实现方负责；机制只认
//!    [`VdfsAccess`] 的四个访问位，不做任何按类型的特判。
//! 3. **路径是唯一地址**：`/` = 虚拟根，`/<mount>/<rel>` = 节点；provider 只见
//!    自己的相对路径，绝不见挂载前缀。

pub mod host;

// ---- 纯接口（宿主无关，定义在 `symbio_core::vdfs_provider`）----
pub use super::vdfs_provider::*;

// ---- symbio 桥 ----
pub use host::{from_plugin_error, host_ctx, vdfs_context};
