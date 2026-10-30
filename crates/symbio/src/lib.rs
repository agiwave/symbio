//! Symbio 宿主装配 crate。
//!
//! 插件实现住在各自的 `crates/symbio-plugin-<id>/`；本 crate 只做三件事：
//!
//! 1. **把插件 crate 链接进来**——`pub use <crate>;` 同时完成链接与门面出口；
//!    漏一行，那个插件的 `inventory` 注册项就会被链接器丢掉（不报错，只少功能）。
//!    这些行由 `scripts/facts/plugin-links-rs.mjs` 从目录投影，**不手改**。
//! 2. [`assembly`]：装配策略——一棵标准插件树挂哪些插件、哪些插件不许停用。
//!    它不是机制而是**装配方的数据**（常量由 `PLUGIN.yml` 生成到 `symbio-deploy`），
//!    故不住 `symbio_core`（core 只经 `REQUIRED_PLUGINS` 接收装配结果）。
//! 3. [`init`]：日志初始化与根插件构造。
//!
//! `symbio_core` / `symbio_deploy` 在这里重导出，供宿主与 CLI 取契约；
//! **插件之间不许经本门面互相可见**（E-009：插件只依赖 core / plugin-api / kernel / deploy）。

pub mod assembly;
pub mod init;

// ── 门面出口 ──────────────────────────────────────────────────────────────
// 宿主与 CLI 从这里取契约；插件自身只依赖 core / plugin-api（E-009）。
pub use symbio_core;
pub use symbio_deploy;

// 插件 crate 必须**被链接**，否则 `inventory` 里的工厂注册项会被链接器丢掉。
// `pub use <crate>;` 同时完成两件事：链接 + 门面出口。
pub use symbio_plugin_api::{hooks, routes};
// >>> plugin-links:begin
// 以下链接行由 `scripts/facts/plugin-links-rs.mjs` 从 `crates/symbio-plugin-<id>/` 目录投影，请勿手改；漏一行的后果是那个插件的 inventory 注册项被链接器丢掉（不报错，只少功能）。
pub use symbio_plugin_agent;
pub use symbio_plugin_classify;
pub use symbio_plugin_compose;
pub use symbio_plugin_composite;
pub use symbio_plugin_event_bus;
pub use symbio_plugin_gateway;
pub use symbio_plugin_heartbeat;
pub use symbio_plugin_home;
pub use symbio_plugin_hook;
pub use symbio_plugin_local;
pub use symbio_plugin_loop;
pub use symbio_plugin_mcp;
pub use symbio_plugin_memory;
pub use symbio_plugin_model;
pub use symbio_plugin_notes;
pub use symbio_plugin_plugin_manager;
pub use symbio_plugin_session;
pub use symbio_plugin_setting;
pub use symbio_plugin_skill;
pub use symbio_plugin_task;
pub use symbio_plugin_telegram;
pub use symbio_plugin_vdfs;
pub use symbio_plugin_web;
// <<< plugin-links:end
