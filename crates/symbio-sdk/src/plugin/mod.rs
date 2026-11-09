//! 插件核心 Trait（上下文注入版）——宿主侧补充面
//!
//! 契约本体（`Plugin` / 上下文 / 信封 / 错误）已在 `symbio_kernel`（D-188 C2b），
//! 这里保留宿主事实：插件目录与清单（[`dir`]）、遍历端点（[`traverse`]），
//! 能力效果的机制面（[`effects`] / [`behaviors`]，随 `submit_behavior!` 从门面
//! 整块搬来，`D-408`）、上下文贡献的机制面（[`context`]，随 `submit_context_provider!`
//! 整块搬来，`D-412`），以及五个链接期注册表（[`retrieval`] / [`rows`] / [`settle`] /
//! [`fact_source`] / [`drive_entry`]，随各自的提交宏整块搬来，`D-413`）；跨插件报表的
//! 登记与读取（[`reports`]）与视图缺席原因（[`views`]）随注册表搬来（`D-416`）。

// 契约词汇由本域子模块直接从 `symbio_kernel::plugin` 取；本域**不再**整模块转出内核的
// 公开面——那会让 `symbio_sdk::plugin::<内核符号>` 成为第二条合法路径（DE-018 的 D-419）。

// 例外：`submit_object_creator!` 的宏体写 `$crate::plugin::PluginInvokeRequest`，
// 展开在消费方 crate——这一条必须是公开路径。
pub use symbio_kernel::plugin::PluginInvokeRequest;

mod dir;
mod traverse;

pub mod behaviors;
pub mod context;
pub mod drive_entry;
pub mod effects;
pub mod fact_source;
pub mod reports;
pub mod retrieval;
pub mod rows;
pub mod settle;
pub mod views;

// 域内子模块私有，公开面在此显式重导出
pub use dir::{
    config_mount_dispatch, plugin_dir_from_ctx, plugin_expand_tilde_path, PluginConfigFile,
    PluginConfigMount, PluginDir, PluginEntry, PLUGIN_DIR, PLUGIN_FILE, PLUGIN_KEY_CAN_DISABLE,
    PLUGIN_KEY_ENABLED, PLUGIN_KEY_NAME, PLUGIN_KEY_PROVIDER, PLUGIN_KEY_REQUIRED,
    PLUGIN_KEY_VERSION,
};
// 锁辅助函数不在这里转出：扩展层经门面从 `symbio_kernel::plugin::error` 取，
// 本 crate 内没有消费方（DE-018 的 D-419：内核符号不留第二条路径）。
pub use traverse::{TRAVERSE_AVAILABLE_OPTIONS, TRAVERSE_AVAILABLE_TOOLS};
