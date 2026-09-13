//! VFDS 插件 —— **访问层**（对前端 / 对 LLM 的唯一出入口）
//!
//! 见 `docs/design/vdfs.md`。本插件的全部职责就是两件事：
//!
//! 1. 给**前端**提供调用后端整体 VDFS 的接口（`vdfs/*`）；
//! 2. 给**大模型**提供访问 VDFS 的工具（`vdfs_*`）。
//!
//! 两者是同一次翻译（[`host::dispatch_with`]），不存在第二套实现。
//!
//! ## 本层不持有拓扑（重要）
//!
//! 虚拟根 `/` 归**组合容器**所有：`composite` 实现
//! [`crate::symbio_core::vdfs_provider::VdfsProvider`] 并把组合视图注册为 VDFS **根**；
//! 每个实现了该 trait 的插件，以**自己的插件名**作为挂载点出现在根下成为子目录。
//! 本层只取这个根、转发全路径——不认识任何挂载点，也不认识任何资源类型
//! （不认识会话 / 模型 / 设置）。
//!
//! ```text
//! 前端 / LLM ──vdfs/*──▶ 本插件（翻译）──▶ 根 = composite 的组合视图
//! ```
//!
//! ## 与其他层的分工
//!
//! - 纯接口（trait + 域类型 + 组合视图 `VdfsMountTable`）：`symbio_core::vdfs_provider`
//! - symbio 桥（上下文注入 / 错误翻译）：`symbio_core::vdfs::host`
//! - 线路信封（`vdfs/*` 请求响应 + 协议路径）：本插件的 `protocol` 模块
//! - 访问层（取根 / 翻译操作 / 树遍历 / 事件投递）：本插件的 `host` 模块
//! - LLM 工具链路（地址翻译 + 按挂载名直调 provider）：本插件的 `provider` 模块，
//!   它持有 `CapabilityVisitor`，`tools/` 下的每个工具构造时持有它
//! - 拓扑（容器聚合 + 注册为根）：`plugins/composite/vdfs.rs`

mod host;
mod plugin;
mod provider;
mod protocol;
mod tools;
