//! Symbio 宿主装配 crate。
//!
//! 插件实现住在各自的 `crates/symbio-plugin-<id>/`；本 crate 只做三件事：
//!
//! 1. **把插件 crate 链接进来**——按目录投影出私有的 `use <crate> as _;`（只链接，
//!    不出口）；漏一行，那个插件的 `inventory` 注册项就会被链接器丢掉（不报错，
//!    只少功能）。这些行由 `scripts/facts/plugin-links-rs.mjs` 从目录投影，**不手改**。
//! 2. [`assembly`]：装配策略——一棵标准插件树挂哪些插件、哪些插件不许停用。
//!    它不是机制而是**装配方的数据**（常量由 `PLUGIN.yml` 生成到 `symbio-deploy`），
//!    故不住任何架构 crate（装配结果只经 `REQUIRED_PLUGINS` 回流）。
//! 3. [`init`]：日志初始化与根插件构造。
//!
//! `symbio_kernel` / `symbio_sdk` / `symbio_deploy` 在这里重导出，供宿主与 CLI 取契约
//! （两层各归各位：封闭语法 / 宿主机制与策略 / 部署事实）；
//! **插件之间不许经本门面互相可见**（E-009：插件只依赖 plugin-api）。

pub mod assembly;
#[cfg(test)]
mod assembly_acceptance;
pub mod drive;
pub mod init;
#[cfg(test)]
mod notify_port_acceptance;

/// 部署根是**进程级单例**：Home 构造与两个驱动用例都会写它。本测试二进制里任何
/// 「先 `publish_deployment_root`、后解析」的判据都要持这把锁——并行构造 Home 会把
/// 根换掉，`main_drive()` 解到别人的根上，t29 判据就假红。
#[cfg(test)]
pub(crate) static DEPLOYMENT_ROOT_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

// ── 门面出口 ──────────────────────────────────────────────────────────────
// 宿主与 CLI 从这里取契约；插件自身只依赖插件门面（E-009）。
pub use symbio_deploy;
pub use symbio_kernel;
pub use symbio_plugin_api;
pub use symbio_sdk;

// 插件 crate 必须**被链接**，否则 `inventory` 里的工厂注册项会被链接器丢掉。
// 链接是私有的（`use … as _`，见下面的生成块）：宿主门面只显式导出装配所需符号，
// 不把每个插件的完整公开面挂到 `symbio::` 下（D-417）。
pub use symbio_plugin_api::{hooks, routes};
// >>> plugin-links:begin
// 以下链接行由 `scripts/facts/plugin-links-rs.mjs` 从 `crates/symbio-plugin-<id>/` 目录投影，请勿手改；漏一行的后果是那个插件的 inventory 注册项被链接器丢掉（不报错，只少功能）。链接是私有的（`use … as _`），宿主门面的出口在上半段显式写（D-417）。
use symbio_plugin_agent as _;
use symbio_plugin_classify as _;
use symbio_plugin_compact as _;
use symbio_plugin_compose as _;
use symbio_plugin_composite as _;
use symbio_plugin_conation as _;
use symbio_plugin_cost as _;
use symbio_plugin_event_bus as _;
use symbio_plugin_forecast as _;
use symbio_plugin_gateway as _;
use symbio_plugin_glossary as _;
use symbio_plugin_heartbeat as _;
use symbio_plugin_home as _;
use symbio_plugin_hook as _;
use symbio_plugin_local as _;
use symbio_plugin_loop as _;
use symbio_plugin_mcp as _;
use symbio_plugin_memory as _;
use symbio_plugin_model as _;
use symbio_plugin_persona as _;
use symbio_plugin_plugin_manager as _;
use symbio_plugin_probe as _;
use symbio_plugin_reputation as _;
use symbio_plugin_session as _;
use symbio_plugin_setting as _;
use symbio_plugin_skill as _;
use symbio_plugin_slo as _;
use symbio_plugin_task as _;
use symbio_plugin_telegram as _;
use symbio_plugin_tool as _;
use symbio_plugin_vdfs as _;
use symbio_plugin_web as _;
// <<< plugin-links:end

#[cfg(test)]
mod report_columns_acceptance;
