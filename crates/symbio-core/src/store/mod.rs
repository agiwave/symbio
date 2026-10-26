//! 事件源（④）：实现住 `symbio_kernel`（DE-023 D-188 C2a）。
//!
//! 本模块只做同名转出，仓内既有路径 `crate::store::…` 因此不变；
//! 根账本仍由 `symbio_core/mod.rs` 逐条声明（C-001 / C-003 / C-004）。
pub use symbio_kernel::store::*;
