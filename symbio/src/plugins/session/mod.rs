//! Session 插件模块

mod active;
mod chat_session;
mod compress;
mod context;
mod handlers;
mod heartbeat;
mod orchestrator;
pub(crate) mod prompt;
// `plugin` / `types` 作为公共契约层（crate 内可见），让 `lib.rs` 能直接
// `pub use plugins::session::xxx::X` 拿到公共类型（避免在 session 顶层做中间
// reexport 引入 unused_imports 警告）。
pub(crate) mod plugin;
mod store;
pub(crate) mod types;
