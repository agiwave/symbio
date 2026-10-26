//! 插件核心 Trait（上下文注入版）——宿主侧补充面
//!
//! 契约本体（`Plugin` / 上下文 / 信封 / 错误）已在 `symbio_kernel`（D-188 C2b），
//! 这里保留宿主事实：插件目录与清单（[`dir`]）、遍历端点（[`traverse`]）。

pub use symbio_kernel::plugin::*;

mod dir;
mod traverse;

// 域内子模块私有，公开面在此显式重导出
pub use dir::{
    config_mount_dispatch, plugin_dir_from_ctx, plugin_expand_tilde_path, PluginConfigFile,
    PluginConfigMount, PluginDir, PluginEntry, PLUGIN_DIR, PLUGIN_FILE, PLUGIN_KEY_CAN_DISABLE,
    PLUGIN_KEY_ENABLED, PLUGIN_KEY_NAME, PLUGIN_KEY_PROVIDER, PLUGIN_KEY_REQUIRED,
    PLUGIN_KEY_VERSION,
};
// 锁辅助函数刻意 `pub(crate)`（见 kernel `plugin/error.rs::lock_read` 的说明），不进对外 API
pub(crate) use symbio_kernel::plugin::error::{lock_read, lock_write};
pub use traverse::{TRAVERSE_AVAILABLE_OPTIONS, TRAVERSE_AVAILABLE_TOOLS};
