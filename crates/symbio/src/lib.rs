//! Symbio - Fractal Plugin Architecture Library
//!
//! Symbio 是一个基于分形插件架构的核心库，提供:
//! - 统一的插件接口
//! - 通用对象创建注册表（基于 `inventory` 的静态注册 + `PluginInvokeRequest` 驱动的运行时构造）
//! - 流式调用支持
//! - 能力路由系统
//!
//! ## 模块分层
//!
//! - `plugins/`：业务插件（私有，插件之间不直接相互引用）
//! - `symbio_core/`：核心抽象（Plugin trait、PluginInvokeRequest、schemas、**服务 trait**）
//!   与**架构元素**——含 `authz`（本机部署的授权策略表：谁持有什么能力 + 可见域），
//!   表与它校验用的机制（`governance`）同住一处，`pub(crate)` 开给全 crate
//! - `assembly`：宿主装配策略（私有）——一棵标准插件树挂哪些插件、哪些插件不许停用。
//!   它不是机制而是**装配方的数据**，故不住 `symbio_core`（core 只经 `REQUIRED_PLUGINS`
//!   接收装配结果）
//! - `hooks` / `routes`：宿主侧的机制助手与跨插件地址——两者互相配套，
//!   `fire_hook` 是地址的唯一 core 外消费方，故一起住宿主（DE-023 D-184）。
//!   `hooks` 私有；`routes` 公开（同仓 `cli` 是独立 crate，订阅地址要从这里取）
//! - `providers/`：通用服务基础设施（私有，**不**对外暴露）
//!   - 各服务的**抽象**在 `symbio_core::embedding`
//!   - 各服务的**实现**在 `src/providers/`
//!   - 各服务通过 `submit_object_creator!` 工厂注册
//!   - 业务模块通过 `creator_create_object::<dyn XXXService>(...)` 获取实例
//!   - **不**通过 `pub use` 暴露给 crate 外部

pub mod assembly;
pub mod init;

// ── 门面出口 ──────────────────────────────────────────────────────────────
// 旧的 `symbio::symbio_core` / `symbio::routes` 路径在此保持可用。
pub use symbio_core;
pub use symbio_deploy;

// 插件 crate 必须**被链接**，否则 `inventory` 里的工厂注册项会被链接器丢掉。
// `pub use <crate>;` 同时完成两件事：链接 + 门面出口。
pub use symbio_plugin_agent;
pub use symbio_plugin_api::{hooks, routes};
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
pub use symbio_plugin_plugin_manager;
pub use symbio_plugin_session;
pub use symbio_plugin_setting;
pub use symbio_plugin_skill;
pub use symbio_plugin_telegram;
pub use symbio_plugin_vdfs;
pub use symbio_plugin_web;
