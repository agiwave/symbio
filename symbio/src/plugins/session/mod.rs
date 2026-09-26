//! Session 插件模块

mod active;
mod chat_loop;
mod chat_pipeline;
mod chat_session;
mod compression;
pub(crate) mod config;
mod context_window;
mod handlers;
mod heartbeat;
mod heartbeat_tool;
mod inbox;
mod memory;
mod message_build;
mod model_chat;
mod options;
mod orchestrator;
pub(crate) mod paths;
pub(crate) mod prompt;
mod rate_limit;
mod resume;
mod tokenizer;
mod transcript;
// `plugin` / `types` 作为公共契约层（crate 内可见），让 `lib.rs` 能直接
// `pub use plugins::session::xxx::X` 拿到公共类型（避免在 session 顶层做中间
// reexport 引入 unused_imports 警告）。
mod frames;
mod fs_watcher;
pub(crate) mod plugin;
mod store;
mod text_split;
mod tool_executor;
mod tool_result_guard;
pub(crate) mod types;
mod workdir;

/// 测试用插件目录：配置文件（`PLUGIN.yml`）的落盘目标
///
/// 单元测试不装配容器，因此没有 `PLUGIN_DIR`；指向临时目录即可——这些用例
/// 要么只读、要么在校验阶段就被拦下，不会真的写盘。
#[cfg(test)]
pub(crate) fn test_dir() -> crate::symbio_core::PluginDir {
    crate::symbio_core::PluginDir::at(std::env::temp_dir().join("symbio-test-session"), "session")
}
