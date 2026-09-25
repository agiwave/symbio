//! 全项目注册对象 id 统一常量
//!
//! 设计原则：所有通过 `submit_object_creator!` 注册到全局注册表的对象，
//! 其 id 字符串必须在这里统一定义为 `&'static str` 常量。
//!
//! 收益：
//! - **单一真相源**：注册侧（`submit_object_creator!` 第一参）和使用侧
//!   （`create_object(id, ...)`、测试期望等）共享同一常量
//! - **避免拼写漂移**：任何漏改 / 错改一处都会被编译期拦下
//! - **IDE 友好**：所有调用方跳转即可看到完整 id 列表
//!
//! 命名约定：
//! - 插件工厂：`<plugin>`，例如 `home` / `model` / `agent`
//! - Embedding 服务：`<service>`，例如 `local` / `noop`
//!
//! 边界：**LLM 工具名不在这里**。工具名是 `CapabilityMeta.name` 短名，属各插件自己的
//! 实现细节（`agent_run` 在 `plugins/agent/host/subagent.rs`，`vdfs_*` 由
//! `plugins/vfds/protocol.rs::VDFS_OPS` 派生），不经全局注册表。

// ============ 插件工厂 id ============

/// Home 插件工厂
pub const PLUGIN_HOME: &str = "home";
/// Model 插件工厂
pub const PLUGIN_MODEL: &str = "model";
/// Agent 插件工厂
pub const PLUGIN_AGENT: &str = "agent";
/// Composite 插件工厂
pub const PLUGIN_COMPOSITE: &str = "composite";
/// Web 插件工厂
pub const PLUGIN_WEB: &str = "web";
/// Telegram 插件工厂
pub const PLUGIN_TELEGRAM: &str = "telegram";
/// Skill 插件工厂
pub const PLUGIN_SKILL: &str = "skill";
/// 插件管理插件工厂（智能体全部插件的管理与配置入口）
pub const PLUGIN_MANAGER: &str = "plugin_manager";
pub const PLUGIN_GATEWAY: &str = "gateway";
/// Session 插件工厂
pub const PLUGIN_SESSION: &str = "session";
/// MCP 插件工厂
pub const PLUGIN_MCP: &str = "mcp";
/// Local 插件工厂
pub const PLUGIN_LOCAL: &str = "local";
/// Work 插件工厂（工作区记忆）
pub const PLUGIN_WORK: &str = "work";
/// Hook 插件工厂
pub const PLUGIN_HOOK: &str = "hook";
/// VDFS 插件工厂（虚拟文件系统宿主）
pub const PLUGIN_VDFS: &str = "vdfs";
/// Event Bus 插件工厂（统一事件总线）
pub const PLUGIN_EVENT_BUS: &str = "event_bus";

// ============ 系统级插件工厂 ============

/// **系统级插件工厂**：它们的目录就是**系统根本身**，与业务插件**并列**而非包含其中。
///
/// 「系统级插件不参与容器扫描」这条规则原先只写在
/// [`crate::symbio_core::plugin::dir`] 的模块文档里（散文），这里给出它的**唯一实现**：
///
/// - 装配期：容器扫描插件根时，扫到的目录里不会有它们（它们的目录就是根）；
/// - 运行期：它们**不作为可安装插件**出现在插件注册表里——把 `home` 装成一个
///   子插件，系统里就会出现第二个根。
///
/// [`PLUGIN_HOME`]: crate::symbio_core::PLUGIN_HOME
pub const SYSTEM_LEVEL_PROVIDERS: &[&str] = &[PLUGIN_HOME, PLUGIN_COMPOSITE];

// （原「Agent 能力 id」区已整体移除：`agent_chat` / `agent_identity` /
//  `agent_cognition` / `agent_create` 是 OAB v1 的能力对象 id，随 v1 装配实现一并
//  下线；现行 agent 域只经 VDFS 暴露资源、经 `agent_run` 一个工具对外委托，
//  没有需要注册到全局注册表的独立对象。）

// ============ Model 协议 id ============
//
// 协议 id 属于插件内部实现细节，定义在 model 插件
// （plugins/model/protocols/mod.rs）；core 不暴露 `MODEL_PROTOCOL_*` 常量，
// 也不定义 `ModelProtocol` trait。

// ============ Embedding 服务 id ============

/// 本地嵌入服务（tract 纯 Rust ONNX，`providers/embedding/local.rs`）
pub const EMBEDDING_LOCAL: &str = "embedding_local";
/// noop embedding 服务（占位 / 禁用）
pub const EMBEDDING_NOOP: &str = "noop";
