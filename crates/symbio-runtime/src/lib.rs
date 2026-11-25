//! Symbio 运行时——CLI / Tauri / 浏览器 Server 三种入口共用的同一份宿主。
//!
//! - [`host`]：进程内宿主 `Host`（根插件 + 事实源）、义务环驱动入口 `HostDrive`，
//!   以及主部署驱动入口的装配登记。
//! - [`RuntimeHost`]：三种入口共用的运行时树——控制面路由与会话执行体监督。
//! - [`worker`]：会话执行体（`__worker` 子命令、监督者、持久收件箱、epoch fencing）。
//! - 浏览器 Server（[`start`] / [`ServerIngress`]）与入站目录选择。
//!
//! 运行时**不认识任何具体插件**：它只接收装配根 `symbio` 构造出的根插件。
//! 传输由 `symbio-http` 提供——它同时被扩展层的接入类插件依赖，所以留在
//! 插件之下，不并入本 crate。

mod directories;
pub mod host;
mod runtime;
mod server;
pub mod worker;

pub use host::Host;
pub use runtime::RuntimeHost;
pub use server::{start, ServerConfig, ServerIngress};
pub use symbio_http::stop;

/// 部署根是**进程级单例**：Home 构造与驱动用例都会写它。本测试二进制里任何
/// 「先 `publish_deployment_root`、后解析」的判据都要持这把锁——并行构造 Home 会把
/// 根换掉，`main_drive()` 解到别人的根上，t29 判据就假红。
#[cfg(test)]
pub(crate) static DEPLOYMENT_ROOT_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
