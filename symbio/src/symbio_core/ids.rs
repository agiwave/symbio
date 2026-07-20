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
/// Model 插件工厂（原 AI 插件，更名以贴合行业惯例）
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
/// Session 插件工厂
pub const PLUGIN_SESSION: &str = "session";
/// MCP 插件工厂
pub const PLUGIN_MCP: &str = "mcp";
/// Local 插件工厂
pub const PLUGIN_LOCAL: &str = "local";
/// Hook 插件工厂
pub const PLUGIN_HOOK: &str = "hook";
/// Explorer 插件工厂
pub const PLUGIN_EXPLORER: &str = "explorer";
/// Event Bus 插件工厂（统一事件总线）
pub const PLUGIN_EVENT_BUS: &str = "event_bus";

// ============ Agent 能力 id ============

/// Agent 对话能力
pub const CAPABILITY_AGENT_CHAT: &str = "agent_chat";
/// Agent 统一认知能力（合并 memory/reason/learn/plan/metacognition，27 个操作）
pub const CAPABILITY_AGENT_COGNITION: &str = "agent_cognition";
/// Agent 创建能力
pub const CAPABILITY_AGENT_CREATE: &str = "agent_create";

// ============ Model 协议 id ============

/// Anthropic Messages 协议
pub const MODEL_PROTOCOL_ANTHROPIC_MESSAGES: &str = "anthropic_messages";
/// OpenAI Chat Completions 协议
pub const MODEL_PROTOCOL_OPENAI_CHAT: &str = "openai_chat";
/// OpenAI Responses 协议
pub const MODEL_PROTOCOL_OPENAI_RESPONSES: &str = "openai_responses";
/// Gemini API 协议
pub const MODEL_PROTOCOL_GEMINI_API: &str = "gemini_api";

// ============ Agent 存储后端 id ============

/// 内存存储后端
pub const AGENT_STORE_MEMORY: &str = "memory_storage";

// ============ Embedding 服务 id ============

/// fastembed embedding 服务
pub const EMBEDDING_FASTEMBED: &str = "fastembed";
/// noop embedding 服务（占位 / 禁用）
pub const EMBEDDING_NOOP: &str = "noop";
