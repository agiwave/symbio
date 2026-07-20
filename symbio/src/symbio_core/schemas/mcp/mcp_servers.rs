// Corresponding Frontend: tauri/src/schemas/mcp_servers.ts
//!
//! MCP 服务器 CRUD 路由的请求/响应类型
//!
//! 设计原则：
//! - 与 LLM Providers 保持一致的"列表 + 增改删"四件套
//! - Server name 是 `McpConfig.servers` 的 key（不可修改）
//! - 真实 transport（stdio / http）+ 工具发现 / 调用**完全在后端**
//!   `symbio/src/plugins/mcp/` 中实现：见 `manager.rs` + `stdio.rs` + `http.rs`。
//!   前端（`tauri`）**只**负责配置表单 + 调用这些 CRUD API。

use super::mcp_config::{McpConfig, McpServerConfig};

/// servers/list - 列出全部 MCP 服务器
pub mod mcp_servers_list {
    use super::McpConfig;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    pub struct Request {}

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    pub struct Response {
        /// 完整 MCP 配置（包含全部 servers）
        pub config: McpConfig,
    }
}

/// servers/get - 获取单个 MCP 服务器
pub mod mcp_servers_get {
    use super::McpServerConfig;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Request {
        /// 服务器名称（即 McpConfig.servers 的 key）
        pub name: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    pub struct Response {
        /// 服务器名称（与 Request.name 相同）
        pub name: String,
        /// 服务器详细配置
        pub server: McpServerConfig,
    }
}

/// servers/set - 创建或更新一个 MCP 服务器（按 name upsert）
///
/// - 当 `McpConfig.servers` 中不存在同名 key 时，行为为"创建"
/// - 当已存在时，行为为"覆盖"（仅替换该 entry，其它 servers 保持不变）
pub mod mcp_servers_set {
    use super::{McpConfig, McpServerConfig};
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Request {
        /// 服务器名称（即 McpConfig.servers 的 key），创建后不可修改
        pub name: String,
        /// 服务器详细配置
        pub server: McpServerConfig,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    pub struct Response {
        /// 完整 MCP 配置（包含全部 servers）
        pub config: McpConfig,
    }
}

/// servers/delete - 删除一个 MCP 服务器
pub mod mcp_servers_delete {
    use super::McpConfig;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Request {
        /// 服务器名称
        pub name: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize, Default)]
    pub struct Response {
        /// 完整 MCP 配置（删除后的最新状态）
        pub config: McpConfig,
    }
}

/// servers/test - 测试一个 MCP 服务器的连接（不持久化）
///
/// 设计目的：在用户保存前 / 保存后调用，验证 server 是否能成功 handshake 并返回工具列表。
/// 不会修改任何配置或缓存。
pub mod mcp_servers_test {
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Request {
        /// 服务器名称（在 McpConfig.servers 中的 key）
        pub name: String,
    }

    #[derive(Debug, Clone, Serialize, Deserialize)]
    pub struct Response {
        /// 服务器名称
        pub name: String,
        /// 测试是否通过
        pub ok: bool,
        /// 发现的工具数量
        pub tool_count: usize,
        /// 协议版本（协商结果）
        pub protocol_version: String,
        /// BUG-MR30：server 报告的名称（来自 initialize 响应）
        #[serde(default)]
        pub server_name: Option<String>,
        /// BUG-MR30：server 报告的版本（来自 initialize 响应）
        #[serde(default)]
        pub server_version: Option<String>,
        /// BUG-MR32：server 提供的使用说明（来自 initialize 响应）
        #[serde(default)]
        pub instructions: Option<String>,
        /// 失败时的错误信息（成功时为 None）
        pub error: Option<String>,
        /// 测试耗时（毫秒）
        pub elapsed_ms: u64,
    }
}
