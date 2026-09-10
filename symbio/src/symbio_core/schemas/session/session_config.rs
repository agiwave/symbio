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
    /// 自动压缩（上下文 Token 用量达到有效上限 70% 时触发 LLM 语义快照）
    #[serde(default = "default_auto_compress")]
    pub auto_compress: bool,
    /// 上下文会话轮数限制（0 表示不限制，每一轮以一个 User 消息开始）
    #[serde(default = "default_context_messages")]
    pub context_messages: usize,
    /// 会话ID（用于标识具体会话的配置）
    #[serde(default)]
    pub session_id: Option<String>,
    /// 存储后端类型
    #[serde(default)]
    pub store_kind: StoreKind,
    /// 最大工具调用迭代轮数
    #[serde(default = "default_max_tool_rounds")]
    pub max_tool_rounds: usize,
    /// 内容节点淡化行数阈值（请求视图中超过此行数或 token 超预算时做头尾淡化；存储恒为完整原文）
    #[serde(default = "default_compress_line_threshold")]
    pub compress_line_threshold: usize,
    /// 内容节点淡化的"最近 N 条内容节点"保护数（B1 保护窗口）：最近的 Text/Reasoning
    /// 节点在请求视图中豁免淡化（末条消息恒受保护），0 表示不保护
    #[serde(default = "default_compress_keep_recent")]
    pub compress_keep_recent: usize,
    /// 保留完整结果的最近工具调用数量限制（滑动窗口）
    #[serde(default = "default_tool_context_window")]
    pub tool_context_window: usize,
    /// 是否启用工具压缩：向模型暴露主动压缩工具（context_compact）并允许水位
    /// 提醒引导模型调用（独立于自动压缩开关）
    #[serde(default = "default_enable_compact_tool")]
    pub enable_compact_tool: bool,
}

fn default_max_messages() -> usize {
    100
}
fn default_auto_compress() -> bool {
    true
}
fn default_context_messages() -> usize {
    6
}
fn default_max_tool_rounds() -> usize {
    65535
}
fn default_compress_line_threshold() -> usize {
    200
}
fn default_compress_keep_recent() -> usize {
    3
}
fn default_tool_context_window() -> usize {
    15
}
fn default_enable_compact_tool() -> bool {
    false
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            max_messages: default_max_messages(),
            auto_compress: default_auto_compress(),
            context_messages: default_context_messages(),
            session_id: None,
            store_kind: StoreKind::default(),
            max_tool_rounds: default_max_tool_rounds(),
            compress_line_threshold: default_compress_line_threshold(),
            compress_keep_recent: default_compress_keep_recent(),
            tool_context_window: default_tool_context_window(),
            enable_compact_tool: default_enable_compact_tool(),
        }
    }
}
