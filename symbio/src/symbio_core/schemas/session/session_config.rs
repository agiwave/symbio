use serde::{Deserialize, Serialize};

// 存储后端类型

/// 可选的存储后端
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StoreKind {
    /// 目录 + 文件（JSON），默认
    #[default]
    File,
    /// SQLite 数据库
    Sqlite,
}

/// Session configuration - Single Source of Truth
///
/// ## 存储目录
///
/// Session 存储目录**不再**作为配置项，而是从 [`crate::symbio_core::HomedirRegistry`]
/// 直接派生：`<homedir>/plugins/session`。
/// 这样 session 存储始终跟随系统目录，与 homedir 切换逻辑天然契合。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    /// 最大保存会话轮数（每一轮以一个 User 消息开始）
    #[serde(default = "default_max_messages")]
    pub max_messages: usize,
    /// 自动压缩
    #[serde(default = "default_auto_compress")]
    pub auto_compress: bool,
    /// 压缩阈值（消息数）
    #[serde(default = "default_compress_threshold")]
    pub compress_threshold: usize,
    /// 上下文会话轮数限制（0 表示不限制，每一轮以一个 User 消息开始）
    #[serde(default = "default_context_messages")]
    pub context_messages: usize,
    /// 默认 Agent ID
    #[serde(default = "default_agent_id")]
    pub default_agent_id: String,
    /// 会话ID（用于标识具体会话的配置）
    #[serde(default)]
    pub session_id: Option<String>,
    /// 存储后端类型
    #[serde(default)]
    pub store_kind: StoreKind,
    /// 最大工具调用迭代轮数
    #[serde(default = "default_max_tool_rounds")]
    pub max_tool_rounds: usize,
    /// 单条消息行数阈值（超过此值才压缩存档）
    #[serde(default = "default_compress_line_threshold")]
    pub compress_line_threshold: usize,
    /// 保留完整结果的最近工具调用数量限制（滑动窗口）
    #[serde(default = "default_tool_context_window")]
    pub tool_context_window: usize,
}

fn default_max_messages() -> usize {
    100
}
fn default_auto_compress() -> bool {
    true
}
fn default_compress_threshold() -> usize {
    50
}
fn default_context_messages() -> usize {
    6
}
fn default_agent_id() -> String {
    "default_assistant".to_string()
}
fn default_max_tool_rounds() -> usize {
    65535
}
fn default_compress_line_threshold() -> usize {
    200
}
fn default_tool_context_window() -> usize {
    15
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            max_messages: default_max_messages(),
            auto_compress: default_auto_compress(),
            compress_threshold: default_compress_threshold(),
            context_messages: default_context_messages(),
            default_agent_id: default_agent_id(),
            session_id: None,
            store_kind: StoreKind::default(),
            max_tool_rounds: default_max_tool_rounds(),
            compress_line_threshold: default_compress_line_threshold(),
            tool_context_window: default_tool_context_window(),
        }
    }
}
