//! 巩固合并的纯计算内核（字符口径的截断 / 分句 / 合并与保真度）。
//!
//! **实现在 `symbio-sdk`**（D-258）。本文件只保留**命名空间**：`symbio_core::text::merge_kernel::…`
//! 这条路径在 §1.2 里登记为「主题命名空间」（N-001 对模块名不判前缀，判的是它的函数名），
//! 而模块住在哪个 crate 是住址问题——控制面的 import 面不必跟着搬。

pub use symbio_sdk::text::merge_kernel::*;
