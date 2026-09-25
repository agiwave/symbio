//! VDFS（Virtual Dynamic File System）—— 统一的资源与数据访问层
//!
//! 规范：`docs/design/vdfs.md`。
//!
//! ## 模块边界（重要）
//!
//! ```text
//! symbio_core/vdfs_provider.rs   纯接口：VdfsProvider trait + 域类型
//! symbio_core/vdfs/host.rs       symbio 桥：上下文注入 + 错误翻译 + 变更广播
//! symbio_core/vdfs/address.rs    当前父地址机制：根的静态声明 + 拼接
//! providers/vdfs_service/        存储实现：单文件 / 目录 / 内存三种拓扑
//! plugins/vdfs/protocol.rs       线路信封：vdfs/* 请求响应 + 协议路径常量
//! plugins/vdfs/fs.rs             地址规则（根名在这里）+ 两半分流 + 展示口径映射
//! plugins/vdfs/host.rs           访问层：取「根服务者」+ 翻译操作 + 树遍历 + 事件投递
//! plugins/composite/vdfs.rs      拓扑：包含子目录列表的 provider（子目录 = 子插件名）
//! ```
//!
//! **core 只暴露纯接口**（[`crate::symbio_core::vdfs_provider`]），线上形状与
//! 访问层都在 vdfs 插件内部——与 [`crate::symbio_core::model_provider`] 的组织
//! 方式一致（core = 纯 trait；协议适配在插件）。
//!
//! ## 谁拥有拓扑
//!
//! 虚拟根之下没有 root 级 provider 的概念——只有一个**恰好包含若干子目录的
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
//! 同样的道理，**根叫什么也不归 core**：名字是 vdfs 插件的挂载规则，经
//! [`address::AddrRootDecl`] 静态声明（字面量只在 `plugins/vdfs`）。绝对地址
//! 由**上下文里的当前父地址**承载：父插件把请求转发给子插件时（route /
//! traverse 两个转发点），把子上下文里的父地址改成子插件的挂载点
//! `join(根名, 子插件名)`；子插件需要全局地址的少数协议级场合，经
//! [`address::absolute_addr`] 用「上下文父地址 + 相对地址」拼出——不写死、
//! 不问全局。
//!
//! [`VdfsProvider`]: crate::symbio_core::vdfs_provider::VdfsProvider
//!
//! 本模块（`vdfs`）是 symbio 侧的**薄桥**：把纯接口接到
//! `InvokeRequest` / `PluginError` 上，供实现方（如 `plugin_manager` 插件）复用。
//! 换宿主只需重写这一个文件。
//!
//! ## 四条不变量
//!
//! 1. **一个协议**：所有资源只用 `vdfs/*` 一组操作访问，不新造私有协议。
//! 2. **provider 自持**：读写校验、状态、呈现描述都由实现方负责；机制只认
//!    [`VdfsAccess`] 的四个访问位，不做任何按类型的特判。
//! 3. **路径是唯一地址**：provider 只见自己子树内的相对路径（`""` = 自身根），
//!    绝不见外层目录前缀；展示地址由宿主门面（plugins/vdfs/fs.rs）统一映射。
//! 4. **根名只有一个所有者**：挂载点叫什么归 vdfs 插件（静态声明）；其它模块的
//!    绝对地址一律从**上下文里的当前父地址 + 相对地址**拼接
//!    （[`address::absolute_addr`]），不得持有根名。

pub mod address;
pub mod host;

// ---- 纯接口（宿主无关，定义在 `symbio_core::vdfs_provider`）----
pub use super::vdfs_provider::*;

// ---- 当前父地址机制（根声明 + 拼接；消费方全在本 crate 内）----
pub(crate) use address::{absolute_addr, descend_addr, join_addr, AddrRootDecl};

// ---- symbio 桥 ----
pub use host::{
    from_plugin_error, host_ctx, notify_change, unwatch_changes, vdfs_context, watch_changes,
    ChangeSubscriptions,
};
