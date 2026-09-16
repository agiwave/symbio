//! 全项目注册对象 id 统一常量
//!
//! 设计原则：所有通过 `submit_object_creator!` 注册到全局注册表的对象，
//! 其 id 字符串必须在这里统一定义为 `&'static str` 常量。
//!
//! 收益：
//! - **单一真相源**：注册侧（`submit_object_creator!` 第一参）和使用侧
//!   （`AGENT_CAPABILITY_IDS`、`create_object(id, ...)`、测试期望等）共享同一常量
//! - **避免拼写漂移**：任何漏改 / 错改一处都会被编译期拦下
//! - **IDE 友好**：所有调用方跳转即可看到完整 id 列表
//!
//! 命名约定：
//! - 插件工厂：`<plugin>`，例如 `home` / `model` / `agent`
//! - Agent 能力：`<capability>`，例如 `agent_chat` / `agent_memory`
//! - Model 协议：`<protocol>`，例如 `anthropic_messages` / `openai_responses`
//! - 存储后端：`<backend>`，例如 `memory_storage`
//! - Embedding 服务：`<service>`，例如 `fastembed` / `noop`

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
/// Setting 插件工厂
pub const PLUGIN_SETTING: &str = "setting";
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

// ============ Agent 能力 id ============

/// Agent 对话能力（子智能体委托）
pub const CAPABILITY_AGENT_CHAT: &str = "agent_chat";
/// Agent 身份能力：把智能体人格（身份/规则/策略/预算）随工具说明送达 LLM
pub const CAPABILITY_AGENT_IDENTITY: &str = "agent_identity";
/// Agent 统一认知能力（合并 memory/reason/learn/plan/metacognition，27 个操作）
pub const CAPABILITY_AGENT_COGNITION: &str = "agent_cognition";
/// Agent 创建能力
pub const CAPABILITY_AGENT_CREATE: &str = "agent_create";

// ============ Model 协议 id ============
//
// 协议 id 属于插件内部实现细节，定义在 model 插件
// （plugins/model/protocols/mod.rs）；core 不暴露 `MODEL_PROTOCOL_*` 常量，
// 也不定义 `ModelProtocol` trait。

// ============ Embedding 服务 id ============

/// fastembed embedding 服务
pub const EMBEDDING_FASTEMBED: &str = "fastembed";
/// noop embedding 服务（占位 / 禁用）
pub const EMBEDDING_NOOP: &str = "noop";
