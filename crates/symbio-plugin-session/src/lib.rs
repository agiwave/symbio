//! Session 插件模块

mod active;
mod autonomy;
mod capabilities;
mod changes;
mod chat_session;
mod commands;
mod conation;
pub(crate) mod config;
mod convergence;
mod declarations;
mod grants;
mod memory;
pub(crate) mod message_events;
mod options;
mod orchestrator;
// 会话口径的纯折叠（原 core 的 checkpoint / fallback / slo / span / changes，
// DE-025 D-221）：读方只在本插件，判定跟着调用方走。
pub(crate) mod paths;
pub(crate) mod projections;
pub(crate) mod runtime;
mod spend_ledger;
mod stats;
mod stats_reading;
mod temporal;
mod token_ledger;
mod transcript;
pub(crate) mod worker_progress;
mod working;
// `plugin` / `types` 作为公共契约层（crate 内可见），让 `lib.rs` 能直接
// `pub use plugins::session::xxx::X` 拿到公共类型（避免在 session 顶层做中间
// reexport 引入 unused_imports 警告）。
pub(crate) mod plugin;
mod store;
pub(crate) mod types;
mod workdir;

/// 测试用插件目录：配置文件（`PLUGIN.yml`）的落盘目标
///
/// 单元测试不装配容器，因此没有 `PLUGIN_DIR`；指向临时目录即可——这些用例
/// 要么只读、要么在校验阶段就被拦下，不会真的写盘。
#[cfg(test)]
pub(crate) fn test_dir() -> symbio_core::PluginDir {
    symbio_core::PluginDir::at(std::env::temp_dir().join("symbio-test-session"), "session")
}
