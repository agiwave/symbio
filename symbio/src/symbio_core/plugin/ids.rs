//! 插件工厂 id —— 全局对象注册表里的**插件身份**
//!
//! 所有经 `submit_object_creator!` 注册的插件工厂，其 id 字符串在此统一定义为
//! `&'static str`。收益：
//!
//! - **单一真相源**：注册侧（宏的第一参）、装配侧（`PluginMeta::new(id, …)`）与
//!   使用侧（`create_object(id, …)`、测试期望）共享同一常量；
//! - **避免拼写漂移**：漏改 / 错改一处即被 `cargo check` 拦下；
//! - **IDE 友好**：跳转即可看到完整 id 列表。
//!
//! ## 命名：`PLUGIN_ID_<工厂名>`
//!
//! **值**与**标识符**分开：值（`"home"`）是跨进程 / 跨文件的字面量——插件目录名、
//! `PLUGIN.yml` 的 `plugin_provider`、前端路由前缀都取它，**不能改**；标识符只服务
//! 编译期，必须让调用点自证归属，故加 `PLUGIN_ID_` 前缀。
//!
//! 归属本域的理由：插件 id 描述的是「一个插件是谁」，与 `Plugin` trait、`PluginDir`、
//! `PLUGIN.yml` 是同一概念面（见 `symbio_core/README.md` §1.2 的域前缀表）。
//!
//! ## 边界
//!
//! - **LLM 工具名不在这里**：工具名是 `CapabilityMeta.name` 短名，属各插件自己的
//!   实现细节（`agent_run` 在 `plugins/agent/host/subagent.rs`，`vdfs_*` 由
//!   `plugins/vdfs/protocol.rs::VDFS_OPS` 派生），不经全局注册表。
//! - **model 协议 id 不在这里**：协议 id 是插件内部实现细节，定义在
//!   `plugins/model/protocols/mod.rs`；core 既不暴露 `MODEL_PROTOCOL_*` 常量，
//!   也不定义 `ModelProtocol` trait。
//! - **嵌入服务 id 不在这里**：它描述的是嵌入服务而非插件，归 `embedding` 域
//!   （`embedding::ids`）。
//! - **路由地址不在这里**：`<插件目录名>/<子路径>` 是调用路径而非对象身份，
//!   见 `plugin::route`。

// ============ 插件工厂 id ============

/// Home 插件工厂
pub const PLUGIN_ID_HOME: &str = "home";
/// Model 插件工厂
pub const PLUGIN_ID_MODEL: &str = "model";
/// Agent 插件工厂
pub const PLUGIN_ID_AGENT: &str = "agent";
/// Composite 插件工厂
pub const PLUGIN_ID_COMPOSITE: &str = "composite";
/// Web 插件工厂
pub const PLUGIN_ID_WEB: &str = "web";
/// Telegram 插件工厂
pub const PLUGIN_ID_TELEGRAM: &str = "telegram";
/// Skill 插件工厂
pub const PLUGIN_ID_SKILL: &str = "skill";
/// 插件管理插件工厂（智能体全部插件的管理与配置入口）
pub const PLUGIN_ID_MANAGER: &str = "plugin_manager";
/// Gateway 插件工厂（HTTP / WebSocket 入站网关）
pub const PLUGIN_ID_GATEWAY: &str = "gateway";
/// Session 插件工厂
pub const PLUGIN_ID_SESSION: &str = "session";
/// MCP 插件工厂
pub const PLUGIN_ID_MCP: &str = "mcp";
/// Local 插件工厂
pub const PLUGIN_ID_LOCAL: &str = "local";
/// Work 插件工厂（工作区记忆）
pub const PLUGIN_ID_WORK: &str = "work";
/// Hook 插件工厂
pub const PLUGIN_ID_HOOK: &str = "hook";
/// VDFS 插件工厂（虚拟文件系统宿主）
pub const PLUGIN_ID_VDFS: &str = "vdfs";
/// Event Bus 插件工厂（统一事件总线）
pub const PLUGIN_ID_EVENT_BUS: &str = "event_bus";

// 注：`SYSTEM_LEVEL_PROVIDERS`（「系统级插件不参与容器扫描」）不在这里——它只被
// `plugins/composite` 一个模块消费，按 ADR-023 的依赖方判据已下沉到该模块。
