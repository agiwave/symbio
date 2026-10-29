//! VDFS（Virtual Dynamic File System）—— 统一的资源与数据访问层（宿主侧）
//!
//! 规范：`../../../../docs/003-design/009-VDFS规范.md`。
//!
//! ## 模块边界（重要）
//!
//! ```text
//! crates/symbio-kernel/src/vdfs/*.rs     纯接口：VdfsProvider trait + 域类型（words / access / node / …）
//! symbio_core/vdfs/host.rs        symbio 桥：上下文注入 + 错误翻译 + 变更广播
//! symbio_core/vdfs/address.rs     当前父地址机制：根的静态声明 + 拼接 + 路径守卫
//! providers/vdfs_service/         存储实现：单文件 / 目录 / 内存三种拓扑
//! crates/symbio-plugin-vdfs/src/protocol.rs        线路信封：vdfs/* 请求响应 + 协议路径常量
//! crates/symbio-plugin-vdfs/src/fs.rs              地址规则（根名在这里）+ 两半分流 + 展示口径映射
//! crates/symbio-plugin-vdfs/src/host.rs            访问层：取「根服务者」+ 翻译操作 + 树遍历 + 事件投递
//! crates/symbio-plugin-composite/src/vdfs.rs       拓扑：包含子目录列表的 provider（子目录 = 子插件名）
//! ```
//!
//! **纯接口已在 `symbio_kernel`**（D-188 C2b），这里只留宿主侧的两项：symbio 桥与
//! 当前父地址机制。线上形状与访问层都在 vdfs 插件内部。
//!
//! ## 四条不变量
//!
//! 1. **一个协议**：所有资源只用 `vdfs/*` 一组操作访问，不新造私有协议。
//! 2. **provider 自持**：读写校验、状态、呈现描述都由实现方负责；机制只认
//!    [`VdfsAccess`] 的四个访问位，不做任何按类型的特判。
//! 3. **路径是唯一地址**：provider 只见自己子树内的相对路径（`""` = 自身根），
//!    绝不见外层目录前缀；展示地址由宿主门面（crates/symbio-plugin-vdfs/src/fs.rs）统一映射。
//! 4. **根名只有一个所有者**：挂载点叫什么归 vdfs 插件（静态声明）；其它模块的
//!    绝对地址一律从**上下文里的当前父地址 + 相对地址**拼接
//!    （`address::absolute_addr`），不得持有根名。

pub use symbio_kernel::vdfs::*;

pub mod address;
pub mod host;

// ---- 当前父地址机制（根声明 + 拼接；消费方全在本 crate 内）----
pub use address::{absolute_addr, declared_addr_root, descend_addr, join_addr, AddrRootDecl};
// 路径守卫（`..` 段判定 / 前缀段比较）已随纯接口住内核，经上面的 glob 取用

// ---- symbio 桥 ----
pub use host::{
    vdfs_context, vdfs_from_plugin_error, vdfs_host_ctx, vdfs_notify_change, vdfs_unwatch_changes,
    vdfs_watch_changes, VdfsChangeSubscriptions,
};
