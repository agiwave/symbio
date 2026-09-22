//!
//! MCP (Model Context Protocol) 协议层类型定义
//!
//! 包含 JSON-RPC 2.0 协议消息、MCP 工具 / 错误 / 初始化响应等。
//!
//! ## 职责划分
//!
//! - **后端**（`symbio/src/plugins/mcp/`）：实现 MCP 客户端（stdio / http transport），
//!   作为 agent 工具机制的延伸，**仅在 agent 实际需要某个 MCP 工具时**才按需
//!   spawn 子进程 / 建立 HTTP 连接（lazy-load），调用结束后立即关闭。
//! - **前端**（`tauri`）：仅负责 MCP Server 的**配置管理**（CRUD），不实现
//!   任何 transport 客户端。
//!
//! 因此 `McpServerConfig` 的权威定义在本插件的 `schemas/mcp_config.rs`
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
///
/// `rename_all = "camelCase"`：MCP 报文用 camelCase（`readOnlyHint`…），
/// 而 Rust 侧是 snake_case。**协议类型必须显式声明这层映射**——漏一处，
/// 反序列化就在运行期静默失败（见 `McpInitializeResponse` 的注释）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
pub struct McpTool {
    pub name: String,
    /// 规范里 `description` 是**可选**的（工具可以没有描述）——
    /// 缺 `default` 会让「没写描述的工具」把整个 `tools/list` 拖垮。
    #[serde(default)]
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
#[serde(rename_all = "camelCase")]
pub struct ListToolsResult {
    pub tools: Vec<McpTool>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

/// `tools/call` 的**结果载荷**（规范里的 `CallToolResult`）。
///
/// ## ⚠ 这**不是** JSON-RPC 信封
///
/// 信封由 [`JsonRpcResponse`] 承载（`{result, error}`）；调用方拿到的
/// `response.result` **已经**是这一层。曾经这里按信封建模
/// （`{result, error, isError}`），于是拿 `{content, isError}` 去反序列化它：
/// 三个字段全部可选 ⇒ 「解析成功」⇒ 返回
/// `tool_response.result.unwrap_or(Value::Null)` ⇒ **结果被静默换成 `null`**。
///
/// 实测后果（批次 J 的端到端）：MCP 工具确实被调用了（mock server 收到
/// `tools/call`），但喂给模型的结果正文是字符串 `"null"`——工具白跑，
/// 模型拿着空结果继续推理。全程无任何报错。
///
/// 教训：**协议类型宁可让字段缺失时报错，也不要全字段可选**——全可选的结构
/// 会把「形状不匹配」变成「解析成功但内容为空」，而后者无法从日志看出来。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolCallResponse {
    /// 内容块列表（`text` / `image` / `resource` …）
    #[serde(default)]
    pub content: Vec<serde_json::Value>,
    /// 结构化结果（规范可选字段）
    #[serde(default)]
    pub structured_content: Option<serde_json::Value>,
    /// 工具**自身**标记的失败（区别于 protocol error）
    #[serde(default)]
    pub is_error: Option<bool>,
}

impl McpToolCallResponse {
    /// 把内容块压成一段文本：`text` 块取 `text`，其余块保留其 JSON 表示。
    ///
    /// 压平发生在 **MCP 插件内**（调用点），不外泄给会话层——「内容块数组」
    /// 是 MCP 自己的形状，会话层只认工具结果的通用字段名。
    pub fn text(&self) -> String {
        if self.content.is_empty() {
            return self
                .structured_content
                .as_ref()
                .map(|v| v.to_string())
                .unwrap_or_default();
        }
        self.content
            .iter()
            .map(|block| match block.get("text").and_then(|v| v.as_str()) {
                Some(t) => t.to_string(),
                None => block.to_string(),
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// MCP 初始化响应
///
/// ## `rename_all` 不是装饰：漏了它，MCP 整条链路静默失效
///
/// MCP 2025-06-18 规范用 camelCase（`protocolVersion` / `serverInfo`），
/// 而 Rust 字段是 snake_case。**没有这行映射时**，`initialize` 的响应解析必然
/// 失败（`missing field 'protocol_version'`）→ `discover_tools` 报错 →
/// 该 server 的工具一个都注册不上。而失败发生在 `initialize` 阶段、
/// 错误只经 `tracing::warn!` 输出（CLI 不显示），因此表象是
/// 「配置了 MCP server，工具列表里却什么都没有」——排查成本极高。
///
/// 已在真实链路验证：修复前 mock server 只收到 `initialize`，`tools/list`
/// 根本没发出（见 `symbio/src/plugins/mcp/docs/` 与批次 J 的端到端记录）。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
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
#[serde(rename_all = "camelCase")]
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
#[path = "types.test.rs"]
mod tests;
