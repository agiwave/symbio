//! Session 插件模块

mod active;
mod chat_loop;
mod chat_session;
mod compression;
mod context_window;
mod handlers;
mod heartbeat;
mod heartbeat_tool;
mod options;
mod orchestrator;
mod model_chat;
pub(crate) mod paths;
pub(crate) mod prompt;
mod rate_limit;
mod resume;
mod tokenizer;
// `plugin` / `types` 作为公共契约层（crate 内可见），让 `lib.rs` 能直接
// `pub use plugins::session::xxx::X` 拿到公共类型（避免在 session 顶层做中间
// reexport 引入 unused_imports 警告）。
mod fs_watcher;
pub(crate) mod plugin;
mod store;
mod text_split;
mod tool_executor;
mod tool_result_guard;
pub(crate) mod types;
mod workdir;
