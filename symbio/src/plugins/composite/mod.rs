//! Composite 插件模块 - 通用插件容器
//!
//! 按方案 1：本 mod.rs 仅声明子模块 + reexport 跨模块契约。
//! 所有实现细节（Composite 结构体、Plugin trait impl、辅助方法）都在 `composite` 子文件中。

#[allow(clippy::module_inception)]
mod composite;
