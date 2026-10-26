//! 插件 API —— 插件**可以**依赖的宿主面。
//!
//! 只有三类东西住在这里，三者都不是机制实现、而是插件与宿主共用的词汇：
//!
//! - [`routes`]：跨插件调用路径（地址）常量
//! - [`hooks`]：`fire_hook` 助手与 hook 事件枚举
//! - [`declarations`]：声明表的链接期提交口（事件 / 事实 / 视图 / 端口 / 执行体 / 行）
//!
//! 插件不得依赖宿主 crate（`symbio`）——那会形成「插件 ← 宿主 ← 插件」的环。
//! `declarations` 只依赖 `symbio_core` 的声明容器，不认识任何具体插件；收集在链接期
//! 发生，因此插件之间仍然零依赖（DE-024 D-198）。

pub mod declarations;
pub mod hooks;
pub mod routes;

pub use declarations::{
    assemble_declarations, declared_event_cell, sealed_declarations, DeclarationsDecl,
};
// 宏用 `$crate::inventory` 展开（卫生），与 `symbio_core` 的 `submit_object_creator!` 同形。
pub use inventory;
