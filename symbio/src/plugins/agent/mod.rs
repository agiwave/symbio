// 按方案1：所有子模块私有——实现细节不对外暴露
// `core` 作为接口层对子模块可见（`pub(crate)`），让 crate 内任意位置
// 可用 `core::X` 拿到核心接口。lib.rs 顶层按需 reexport。
mod capabilities;
pub(crate) mod core;
mod handlers;
mod manager;
mod plugin;
pub(crate) mod store;
