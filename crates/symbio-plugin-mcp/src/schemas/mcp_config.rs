use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// MCP 传输类型
///
/// 只有两条**真实存在**的路径：本地子进程与 HTTP。未发布产品、无存量配置需兼容
/// （SP-003 §6），旧值 `sse` 不再读入。
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum McpTransportType {
    /// 本地进程通信（spawn 子进程 + stdin/stdout JSON-RPC）
    #[default]
    Stdio,
    /// HTTP REST API
    Http,
}

/// MCP 服务器配置（持久化层权威定义）
///
/// 字段设计：
/// - `transport_type`：决定使用 stdio / http 哪条 transport 路径
/// - `command` + `args` + `env`：stdio transport 必需
/// - `url`：http transport 必需
/// - `include_tools` / `exclude_tools`：工具过滤
/// - `enabled`：运行时是否启用（仅作为"激活"标记，不影响持久化存储）
///
/// 该结构同时被前端（`tauri` 配置 UI）和后端（`crates/symbio-plugin-mcp/src/`）使用。
/// 前端只做配置的增删改查；后端在 agent 真正需要某个工具时按需加载 transport。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// 传输类型（stdio / http）
    #[serde(rename = "type", default)]
    pub transport_type: McpTransportType,

    /// stdio transport 命令（stdio 必需，http 不使用）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,

    /// stdio transport 命令参数
    #[serde(skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,

    /// stdio transport 环境变量
    #[serde(skip_serializing_if = "Option::is_none")]
    pub env: Option<HashMap<String, String>>,

    /// HTTP transport URL
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,

    /// BUG-MR28：HTTP transport 自定义请求头（如 `Authorization: Bearer ...`）
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

    /// 请求超时（秒），None = 使用默认值 30s；stdio 与 HTTP 共用
    ///
    /// 不同 server 工具耗时差异大（filesystem ~ms vs docker inspect ~10s），
    /// 暴露此字段供用户按需调整。stdio 用它约束 initialize / tools/list / tools/call
    /// 的读超时——e2e 因此能在秒级钉住「server 卡住」的形态，而不必等默认 30s。
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

/// 默认请求超时（秒）：stdio 读一行 / HTTP 一次请求共用
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;

impl McpServerConfig {
    /// 本次请求实际使用的超时：`timeout_secs` 显式给出时按它（下限 1 秒），否则默认 30 秒。
    ///
    /// stdio 与 HTTP 走同一个口径——同一个字段在两处各解释一遍，就会有一处开始漂。
    pub fn effective_timeout(&self) -> std::time::Duration {
        std::time::Duration::from_secs(self.timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS).max(1))
    }
}

fn default_true() -> bool {
    true
}

/// MCP configuration - 运行期内存视图
///
/// 磁盘上每个 server 是**自己的目录条目**：`<本插件目录>/<name>/server.json`
/// （见 `plugin.rs` 的 `MANIFEST`）。本结构只承载这些条目的内存视图，由
/// `load_from_storage` 灌入、
/// 写 / 删后同步——不落盘、也不读配置文件。
#[derive(Debug, Clone, Default)]
pub struct McpConfig {
    pub servers: HashMap<String, McpServerConfig>,
}

#[cfg(test)]
#[path = "mcp_config.test.rs"]
mod tests;
