use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// MCP 传输类型
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpTransportType {
    /// 本地进程通信（spawn 子进程 + stdin/stdout JSON-RPC）
    #[default]
    Stdio,
    /// HTTP REST API
    Http,
    /// Server-Sent Events
    Sse,
}

/// MCP 服务器配置（持久化层权威定义）
///
/// 字段设计：
/// - `transport_type`：决定使用 stdio / http / sse 哪条 transport 路径
/// - `command` + `args` + `env`：stdio transport 必需
/// - `url`：http / sse transport 必需
/// - `include_tools` / `exclude_tools`：工具过滤
/// - `enabled`：运行时是否启用（仅作为"激活"标记，不影响持久化存储）
///
/// 该结构同时被前端（`tauri` 配置 UI）和后端（`symbio/src/plugins/mcp/`）使用。
/// 前端只做配置的增删改查；后端在 agent 真正需要某个工具时按需加载 transport。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// 传输类型（stdio / http / sse）
    #[serde(rename = "type", default)]
    pub transport_type: McpTransportType,

    /// stdio transport 命令（stdio 必需，http/sse 不使用）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,

    /// stdio transport 命令参数
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,

    /// stdio transport 环境变量
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,

    /// HTTP / SSE transport URL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    /// BUG-MR28：HTTP / SSE transport 自定义请求头（如 `Authorization: Bearer ...`）
    ///
    /// key = header 名称，value = header 值。stdio transport 忽略此字段。
    /// 注意：保留 `Mcp-Session-Id`、`Content-Type`、`Accept` 三个标准头由客户端管理，
    /// 用户配置同名 key 会被覆盖（warning 日志告知）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,

    /// 仅包含的工具列表（白名单，None = 全部）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub include_tools: Option<Vec<String>>,

    /// 排除的工具列表（黑名单，优先级高于 include_tools）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_tools: Option<Vec<String>>,

    /// BUG-MR31：HTTP transport 请求超时（秒），None = 使用默认值 30s
    ///
    /// 不同 server 工具耗时差异大（filesystem ~ms vs docker inspect ~10s），
    /// 暴露此字段供用户按需调整。stdio transport 不使用此字段（受 `STDIO_READ_TIMEOUT` 约束）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,

    /// 是否启用（仅在 McpManager 中作为"激活"标记使用）
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl Default for McpServerConfig {
    fn default() -> Self {
        Self {
            transport_type: McpTransportType::Stdio,
            command: None,
            args: None,
            env: None,
            url: None,
            headers: None,
            include_tools: None,
            exclude_tools: None,
            timeout_secs: None,
            enabled: true,
        }
    }
}

/// BUG-MR31：默认 HTTP 超时
pub const DEFAULT_HTTP_TIMEOUT_SECS: u64 = 30;

fn default_true() -> bool {
    true
}

/// MCP configuration - Single Source of Truth
///
/// 持久化路径：`~/.symbio/plugins/mcps/<name>/server.json`。
/// 内存视图（`servers: HashMap<name, McpServerConfig>`）通过 `McpPlugin`
/// 从磁盘加载/回写保持一致。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct McpConfig {
    #[serde(default)]
    pub servers: HashMap<String, McpServerConfig>,
}

#[cfg(test)]
mod tests;
