// Corresponding Frontend: tauri/src/protocols/mcp_common.ts
//!
//! MCP (Model Context Protocol) 协议层类型定义
//!
//! 包含 JSON-RPC 2.0 协议消息、MCP 工具 / 错误 / 初始化响应等。
//!
//! ## 职责划分（2026-07-06 重构）
//!
//! - **后端**（`symbio/src/plugins/mcp/`）：实现 MCP 客户端（stdio / http transport），
//!   作为 agent 工具机制的延伸，**仅在 agent 实际需要某个 MCP 工具时**才按需
//!   spawn 子进程 / 建立 HTTP 连接（lazy-load），调用结束后立即关闭。
//! - **前端**（`tauri`）：仅负责 MCP Server 的**配置管理**（CRUD），不实现
//!   任何 transport 客户端。
//!
//! 因此 `McpServerConfig` 的权威定义在 `symbio_core/schemas/mcp_config.rs`
//! （持久化层）；本文件中的 `JsonRpcRequest` / `JsonRpcResponse` /
//! `McpTool` / `McpToolCallResponse` 等是 JSON-RPC 协议消息，与持久化类型
//! 通过 plugin 层转换。

use serde::{Deserialize, Serialize};

// ============================================================================
// 协议层（JSON-RPC 2.0 + MCP）
// ============================================================================

/// MCP 支持的协议版本
///
/// 参考 MCP 2025-06-18 规范：客户端发送，server 协商选择一个。
/// 当前最新为 `2025-06-18`，兼容 `2024-11-05`。
pub const SUPPORTED_PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2024-11-05"];

/// 默认协议版本（协商失败时使用）
pub const DEFAULT_PROTOCOL_VERSION: &str = "2024-11-05";

/// JSON-RPC 2.0 Request ID（string 或 number，符合 RFC）
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(untagged)]
pub enum RequestId {
    Number(i64),
    String(String),
}

impl std::fmt::Display for RequestId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RequestId::Number(n) => write!(f, "{n}"),
            RequestId::String(s) => write!(f, "{s}"),
        }
    }
}

/// MCP 工具注解（参考 2025-06-18 规范）
///
/// agent 可以根据这些注解做安全决策：例如不调用 destructiveHint=true 的工具
/// 在自动批处理时。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolAnnotations {
    #[serde(default, skip_serializing_if = "is_false")]
    pub read_only_hint: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub destructive_hint: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub idempotent_hint: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub open_world_hint: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// MCP 工具定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<ToolAnnotations>,
}

/// BUG-MR27：校验 MCP tool name 合法性
///
/// MCP 2025-06-18 规范：tool name 必须匹配 `^[a-zA-Z0-9_-]{1,64}$`。
/// 非法名称在 agent 路由时会与系统工具冲突或触发 LLM API 拒绝。
pub fn validate_tool_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Tool name 不能为空".to_string());
    }
    if name.len() > 64 {
        return Err(format!("Tool name '{}' 超过 64 字符", name));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(format!(
            "Tool name '{}' 包含非法字符（仅允许字母/数字/下划线/连字符）",
            name
        ));
    }
    Ok(())
}

/// `tools/list` 响应（支持分页）
#[derive(Debug, Clone, Deserialize)]
pub struct ListToolsResult {
    pub tools: Vec<McpTool>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

/// MCP 工具调用响应
#[derive(Debug, Clone, Deserialize)]
pub struct McpToolCallResponse {
    #[serde(default)]
    pub result: Option<serde_json::Value>,
    #[serde(default)]
    pub error: Option<McpError>,
    /// MCP 2025-06-18 规范：tool 自身标记的失败（区别于 protocol error）
    #[serde(default, rename = "isError")]
    pub is_error: Option<bool>,
}

/// MCP 错误（含 data 字段，符合 JSON-RPC 2.0）
#[derive(Debug, Clone, Deserialize)]
pub struct McpError {
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

/// MCP 初始化响应
#[derive(Debug, Clone, Deserialize)]
pub struct McpInitializeResponse {
    pub protocol_version: String,
    pub server_info: McpServerInfo,
    #[serde(default)]
    pub capabilities: McpServerCapabilities,
    /// BUG-MR32：server 提供给 client 的人类可读使用说明
    ///
    /// 行业最佳实践：server 可在 initialize 响应中返回 `instructions` 字段，
    /// 描述如何使用该 server 的工具。客户端应在 UI 中展示给用户。
    #[serde(default)]
    pub instructions: Option<String>,
}

/// MCP 服务器能力
///
/// 当前主要使用 `tools.list_changed`；其它 server capability 通过 `extra`
/// 字段捕获（`resources` / `prompts` / `logging` 等），保留原始 JSON 以便
/// 未来扩展。`extra` 在 logging 时被打印。
#[derive(Debug, Clone, Default, Deserialize)]
pub struct McpServerCapabilities {
    #[serde(default)]
    pub tools: Option<ToolsCapability>,
    /// 其它未严格建模的 capability（如 resources / prompts / logging）
    #[serde(default, flatten)]
    pub extra: std::collections::BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ToolsCapability {
    #[serde(default, rename = "listChanged")]
    pub list_changed: bool,
}

/// MCP 服务器信息
#[derive(Debug, Clone, Deserialize)]
pub struct McpServerInfo {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
}

/// JSON-RPC 请求
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: String,
    pub id: RequestId,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// JSON-RPC 响应
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcResponse {
    #[serde(default)]
    pub id: Option<RequestId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

impl JsonRpcResponse {
    /// 把响应 id 渲染为日志可读字符串（None → "?"）
    pub fn id_display(&self) -> String {
        self.id
            .as_ref()
            .map(|i| i.to_string())
            .unwrap_or_else(|| "?".to_string())
    }
}

/// JSON-RPC 错误
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

// ============================================================================
// 工具函数
// ============================================================================

/// 根据 `include_tools` / `exclude_tools` 过滤工具列表
pub fn filter_tools(
    tools: Vec<McpTool>,
    include: &Option<Vec<String>>,
    exclude: &Option<Vec<String>>,
) -> Vec<McpTool> {
    tools
        .into_iter()
        .filter(|t| {
            if let Some(inc) = include {
                if !inc.iter().any(|n| n == &t.name) {
                    return false;
                }
            }
            if let Some(exc) = exclude {
                if exc.iter().any(|n| n == &t.name) {
                    return false;
                }
            }
            true
        })
        .collect()
}

/// BUG-MR27：过滤掉非法 tool name
///
/// 行业最佳实践：server 返回的 tools 中可能包含非规名称（不规范实现 / 内部工具），
/// 应在 client 端过滤掉，避免污染 agent 工具路由。
///
/// 返回 `(合法工具, 非法数量)`。调用方可以基于 `invalid_count` 决定是否 warn。
pub fn filter_valid_tool_names(tools: Vec<McpTool>) -> (Vec<McpTool>, usize) {
    let mut valid = Vec::with_capacity(tools.len());
    let mut invalid_count = 0;
    for t in tools {
        match validate_tool_name(&t.name) {
            Ok(()) => valid.push(t),
            Err(e) => {
                tracing::warn!("过滤非法 tool name: {e}");
                invalid_count += 1;
            }
        }
    }
    (valid, invalid_count)
}

#[cfg(test)]
mod tests;
