//! VDFS（Virtual Dynamic File System）—— 统一的资源与数据访问层
//!
//! 规范：`docs/design/vdfs.md`。
//!
//! ## 模块边界（重要）
//!
//! ```text
//! symbio_core/vdfs_provider.rs   纯接口：VdfsProvider trait + 域类型
//! symbio_core/vdfs/host.rs       symbio 桥：上下文注入 + 错误翻译 + 变更广播
//! symbio_core/entities.rs        存储原语：写盘 / 删除 / 导入 / 导出（自由函数）
//! plugins/vdfs/protocol.rs       线路信封：vdfs/* 请求响应 + 协议路径常量
//! plugins/vdfs/host.rs           访问层：取「.vdfs 服务者」+ 翻译操作 + 树遍历 + 事件投递
//! plugins/composite/vdfs.rs      拓扑：包含子目录列表的 provider（子目录 = 子插件名）
//! ```
//!
//! **core 只暴露纯接口**（[`crate::symbio_core::vdfs_provider`]），线上形状与
//! 访问层都在 vdfs 插件内部——与 [`crate::symbio_core::model_provider`] 的组织
//! 方式一致（core = 纯 trait；协议适配在插件）。
//!
//! ## 谁拥有拓扑
//!
//! `.vdfs` 之下没有 root 级 provider 的概念——只有一个**恰好包含若干子目录的
//! provider**（当前是 composite：它的目录内容 = 实现了该接口的子插件名）。容器
//! 在 `traverse` 里经 `CapabilityVisitor::register_vdfs_root` 把自己登记进访问层
//! 的单槽位；vdfs 插件只取这个登记项再转发（`get_vdfs_root`），因此**拓扑知识
//! 不在访问层**。composite 可被别的目录包含、子插件也可以是另一个 composite
//! （嵌套时同样只是普通 provider）——登记进单槽只是装配安排，不是 composite 的
//! 属性。
//!
//! 每个实现了 [`VdfsProvider`] 的插件，以**自己的插件名**成为根下的一级子目录——
//! 「新增资源 = 新增一个子目录」，访问层与前端都无需改动。
//!
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
//! 3. **路径是唯一地址**：provider 只见自己子树内的相对路径（`""` = 自身根），
//!    绝不见外层目录前缀；展示地址由宿主门面（plugins/vdfs/fs.rs）统一映射。

pub mod host;

// ---- 纯接口（宿主无关，定义在 `symbio_core::vdfs_provider`）----
pub use super::vdfs_provider::*;

// ---- symbio 桥 ----
pub use host::{
    from_plugin_error, host_ctx, notify_change, unwatch_changes, vdfs_context, watch_changes,
};
