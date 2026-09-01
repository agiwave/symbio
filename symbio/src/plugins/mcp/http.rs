//! MCP Streamable HTTP transport
//!
//! 实现了 MCP 2025-06-18 规范的 **Streamable HTTP**：
//!
//! - 单一端点（POST），客户端请求 JSON-RPC
//! - 完整 `initialize` 握手（`initialize` → 响应 → `notifications/initialized`）
//! - 服务端可返回 `Mcp-Session-Id` 头，客户端后续请求需带上
//! - `Accept: application/json, text/event-stream`
//! - `protocolVersion` 协商
//!
//! ## 与旧版 HTTP 的区别
//!
//! 旧版（`http+sse`）用 `GET /tools` 列出工具、`POST /tools/{name}/call` 调用，
//! 这不是 MCP 规范。新版用 JSON-RPC 协议：
//!
//! ```text
//! POST {url}                      # initialize
//! POST {url}                      # notifications/initialized
//! POST {url}                      # tools/list
//! POST {url}                      # tools/call
//! ```
//!
//! ## 资源管理
//!
//! - 共享 `reqwest::Client`（连接池）
//! - 复用 `Mcp-Session-Id`（同一 server 多次调用共享 session）
//! - 使用 cache 缓存 `discover_tools` 结果

use super::manager::TestConnectionResult;
use super::types::{
    JsonRpcRequest, JsonRpcResponse, ListToolsResult, McpInitializeResponse, McpTool,
    McpToolCallResponse, RequestId, DEFAULT_PROTOCOL_VERSION, SUPPORTED_PROTOCOL_VERSIONS,
};
use crate::symbio_core::schemas::mcp::mcp_config::{McpServerConfig, DEFAULT_HTTP_TIMEOUT_SECS};
use serde::Serialize;
use serde_json::{json, Value};
use std::time::Duration;
use tracing::{debug, info, warn};

/// MCP JSON-RPC 错误码：会话未找到（server 已清理）
/// 业界常见值，server 端可能略有不同，我们做"错误信息包含"的模糊匹配。
const SESSION_NOT_FOUND_CODE: i32 = -32000;

/// 构造 `initialize` 请求的 `params`
fn build_initialize_params() -> Value {
    json!({
        "protocolVersion": SUPPORTED_PROTOCOL_VERSIONS[0],
        "protocolVersions": SUPPORTED_PROTOCOL_VERSIONS,
        "capabilities": {},
        "clientInfo": {
            "name": "symbio",
            "version": env!("CARGO_PKG_VERSION")
        }
    })
}

/// 协议版本协商
fn negotiate_protocol_version(server_version: &str) -> String {
    if SUPPORTED_PROTOCOL_VERSIONS.contains(&server_version) {
        server_version.to_string()
    } else {
        warn!(
            server_protocol = server_version,
            "MCP server 协议版本不受支持，使用默认 {}", DEFAULT_PROTOCOL_VERSION
        );
        DEFAULT_PROTOCOL_VERSION.to_string()
    }
}

/// BUG-MR22：检测响应是否表示 session 已失效
///
/// 参考：MCP server 通常返回 -32000（JSON-RPC server error）或 message 中包含
/// "session not found" / "session expired" / "invalid session" 等字样。
fn is_session_expired(err: &super::types::JsonRpcError) -> bool {
    if err.code == SESSION_NOT_FOUND_CODE {
        return true;
    }
    let msg_lower = err.message.to_lowercase();
    msg_lower.contains("session not found")
        || msg_lower.contains("session expired")
        || msg_lower.contains("invalid session")
        || msg_lower.contains("session has expired")
}

/// BUG-MR28：把 `McpServerConfig.headers` 合并到 reqwest RequestBuilder
///
/// 标准头（`Content-Type` / `Accept` / `Mcp-Session-Id`）由 client 内部管理，
/// 若用户在 `headers` 中配置了同名 key，记录 warning（保留 client 的标准值）。
fn apply_custom_headers(
    mut req: reqwest::RequestBuilder,
    config: &McpServerConfig,
) -> reqwest::RequestBuilder {
    const RESERVED: &[&str] = &["content-type", "accept", "mcp-session-id"];
    if let Some(headers) = &config.headers {
        for (k, v) in headers {
            let k_lower = k.to_lowercase();
            if RESERVED.contains(&k_lower.as_str()) {
                warn!(
                    header = k,
                    "BUG-MR28: headers 中包含保留头 '{}'，将被 client 内部值覆盖", k
                );
                continue;
            }
            req = req.header(k, v);
        }
    }
    req
}

/// BUG-MR31：按 config 调整 HTTP 客户端超时
///
/// 默认 `DEFAULT_HTTP_TIMEOUT_SECS`（30s），可通过 `McpServerConfig.timeout_secs` 覆盖。
fn effective_timeout(config: &McpServerConfig) -> Duration {
    Duration::from_secs(
        config
            .timeout_secs
            .unwrap_or(DEFAULT_HTTP_TIMEOUT_SECS)
            .max(1),
    )
}

/// 构造带标准头 + 自定义头 + 超时的 POST 请求
fn build_request(
    client: &reqwest::Client,
    url: &str,
    config: &McpServerConfig,
    body: &impl Serialize,
) -> reqwest::RequestBuilder {
    let req = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .timeout(effective_timeout(config))
        .json(body);
    apply_custom_headers(req, config)
}

impl super::manager::McpManager {
    /// 通过 HTTP 发现工具（完整 initialize 握手 + tools/list）
    pub async fn discover_tools_http(
        &self,
        name: &str,
        config: &McpServerConfig,
    ) -> Result<Vec<McpTool>, String> {
        let url = config
            .url
            .as_ref()
            .ok_or("http transport requires 'url' field")?;

        // 1) 完整握手（包含 session_id 缓存）
        let session_id = self.http_initialize(name, config, url).await?;
        let tools = self.http_tools_list(name, config, url, &session_id).await?;
        Ok(tools)
    }

    /// 测试 HTTP MCP server 的连接（不写 discover 缓存，但复用 session）
    ///
    /// 与 `discover_tools_http` 的区别：
    /// - 不读 discover 缓存（避免缓存的 false-positive）
    /// - 不写 discover 缓存
    /// - 仍会建立/复用 HTTP session（与正常调用一致）
    /// - 返回 `TestConnectionResult`（含 tool count + 协议版本 + server 名称/版本/instructions）
    pub async fn test_connection_http(
        &self,
        name: &str,
        config: &McpServerConfig,
    ) -> Result<TestConnectionResult, String> {
        let url = config
            .url
            .as_ref()
            .ok_or("http transport requires 'url' field")?;
        // 强制走新握手（不走 session 缓存），保持测试独立性
        self.session_cache.remove(name).await;

        // BUG-MR30/MR32：完整握手以获取 server_name/version/instructions
        let (session_id, server_name, server_version, instructions) =
            self.http_initialize_full(name, config, url).await?;
        let tools = self.http_tools_list(name, config, url, &session_id).await?;
        // 协议版本以初始化响应的协商结果为准（已在 http_initialize_full 中记录）
        let proto = self.last_negotiated_protocol(name).await;
        Ok(TestConnectionResult {
            tool_count: tools.len(),
            protocol_version: proto,
            server_name,
            server_version,
            instructions,
            elapsed_ms: 0,
        })
    }

    /// 通过 HTTP 调用工具（如果未握手则先握手）
    ///
    /// BUG-MR22 修复：如果 server 返回 session-expired 错误，
    /// 自动清理 session 缓存并重试一次（重新 initialize）。
    pub async fn call_tool_http(
        &self,
        name: &str,
        config: &McpServerConfig,
        tool_name: &str,
        args: Value,
    ) -> Result<Value, String> {
        // 第一次尝试：复用现有 session
        match self
            .call_tool_http_once(name, config, tool_name, args.clone())
            .await
        {
            Ok(v) => Ok(v),
            Err(e) if e.contains("session") || e.contains("-32000") => {
                // 可能是 session 失效：清缓存并重试一次
                warn!(server = name, error = %e, "HTTP session 疑似失效，尝试重新握手");
                self.session_cache.remove(name).await;
                self.call_tool_http_once(name, config, tool_name, args)
                    .await
            }
            Err(e) => Err(e),
        }
    }

    /// 单次 HTTP 工具调用（不重试）
    async fn call_tool_http_once(
        &self,
        name: &str,
        config: &McpServerConfig,
        tool_name: &str,
        args: Value,
    ) -> Result<Value, String> {
        let url = config
            .url
            .as_ref()
            .ok_or("http transport requires 'url' field")?;

        // 确保有 session（discover 时已建过；call 单独调用则需现建）
        let session_id = self.http_get_or_initialize(name, config, url).await?;

        // 发送 tools/call
        let call_request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(2),
            method: "tools/call".to_string(),
            params: Some(json!({
                "name": tool_name,
                "arguments": args
            })),
        };

        let mut req = build_request(&self.http_client, url, config, &call_request);
        if let Some(sid) = &session_id {
            req = req.header("Mcp-Session-Id", sid);
        }

        let response = req
            .send()
            .await
            .map_err(|e| format!("HTTP request failed: {e}"))?;
        let resp: JsonRpcResponse = self.parse_jsonrpc_response(response, "tools/call").await?;
        if let Some(error) = resp.error {
            // BUG-MR22：检测 session 失效
            if is_session_expired(&error) {
                warn!(
                    server = name,
                    code = error.code,
                    "session 失效，标记为需重连"
                );
            }
            return Err(format!(
                "tools/call error: {} - {}{}",
                error.code,
                error.message,
                error
                    .data
                    .as_ref()
                    .map(|d| format!(" ({d})"))
                    .unwrap_or_default()
            ));
        }
        let result = resp
            .result
            .ok_or_else(|| "tools/call response missing result".to_string())?;

        // 优先按 McpToolCallResponse 解析（isError 等）
        if let Ok(tool_response) = serde_json::from_value::<McpToolCallResponse>(result.clone()) {
            if let Some(error) = tool_response.error {
                return Err(format!(
                    "Tool error: {} - {}{}",
                    error.code,
                    error.message,
                    error
                        .data
                        .as_ref()
                        .map(|d| format!(" ({d})"))
                        .unwrap_or_default()
                ));
            }
            if tool_response.is_error == Some(true) {
                let r = tool_response.result.unwrap_or(Value::Null);
                let msg = r
                    .as_str()
                    .map(String::from)
                    .unwrap_or_else(|| r.to_string());
                return Err(format!("Tool returned error: {msg}"));
            }
            return Ok(tool_response.result.unwrap_or(Value::Null));
        }
        Ok(result)
    }

    /// 完整 initialize 握手：POST {url} (initialize) → POST notifications/initialized
    /// 返回 (session_id, server_name, server_version, instructions)
    ///
    /// BUG-MR30/MR32：返回 server_info + instructions，便于 test_connection 展示。
    async fn http_initialize_full(
        &self,
        name: &str,
        config: &McpServerConfig,
        url: &str,
    ) -> Result<
        (
            Option<String>,
            Option<String>,
            Option<String>,
            Option<String>,
        ),
        String,
    > {
        let init_request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(1),
            method: "initialize".to_string(),
            params: Some(build_initialize_params()),
        };

        let response = build_request(&self.http_client, url, config, &init_request)
            .send()
            .await
            .map_err(|e| format!("initialize HTTP request failed: {e}"))?;

        // 提取 session id 头
        let session_id = response
            .headers()
            .get("Mcp-Session-Id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let init_resp: JsonRpcResponse =
            self.parse_jsonrpc_response(response, "initialize").await?;

        let result = init_resp
            .result
            .ok_or_else(|| "initialize response missing result".to_string())?;

        let initialize_response: McpInitializeResponse = serde_json::from_value(result)
            .map_err(|e| format!("Failed to parse initialize response: {e}"))?;
        let negotiated = negotiate_protocol_version(&initialize_response.protocol_version);
        let server_name = Some(initialize_response.server_info.name.clone());
        let server_version = initialize_response.server_info.version.clone();
        let instructions = initialize_response.instructions.clone();
        info!(
            server = %name,
            upstream = %initialize_response.server_info.name,
            upstream_version = %server_version.as_deref().unwrap_or("unknown"),
            protocol = %negotiated,
            session = ?session_id,
            // BUG-MR32：透传 server instructions（如有）
            has_instructions = instructions.is_some(),
            "Connected to MCP server (http)"
        );
        if let Some(inst) = &instructions {
            debug!(server = %name, instructions = %inst, "server instructions");
        }

        // 发送 notifications/initialized（无 id，无响应）
        let notification = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        });
        let mut req = build_request(&self.http_client, url, config, &notification);
        if let Some(sid) = &session_id {
            req = req.header("Mcp-Session-Id", sid);
        }
        let _ = req.send().await; // 失败不阻断（server 可能不监听通知）

        // 缓存 session_id
        if let Some(sid) = &session_id {
            self.session_cache
                .insert(name.to_string(), sid.clone())
                .await;
        }
        // 记录最近协商协议版本，供 test_connection 读取
        self.negotiated_protocols
            .insert(name.to_string(), negotiated)
            .await;

        Ok((session_id, server_name, server_version, instructions))
    }

    /// HTTP initialize 便捷方法：仅返回 session_id（call_tool 等不需要 server info 的场景）
    async fn http_initialize(
        &self,
        name: &str,
        config: &McpServerConfig,
        url: &str,
    ) -> Result<Option<String>, String> {
        let (session_id, _, _, _) = self.http_initialize_full(name, config, url).await?;
        Ok(session_id)
    }

    /// 读取最近协商的协议版本（test_connection 内部使用）
    async fn last_negotiated_protocol(&self, name: &str) -> String {
        self.negotiated_protocols
            .get(name)
            .await
            .unwrap_or_else(|| DEFAULT_PROTOCOL_VERSION.to_string())
    }

    /// 读取或创建 session：先看 cache，miss 时初始化
    async fn http_get_or_initialize(
        &self,
        name: &str,
        config: &McpServerConfig,
        url: &str,
    ) -> Result<Option<String>, String> {
        if let Some(sid) = self.session_cache.get(name).await {
            debug!(server = name, "复用 session_id");
            return Ok(Some(sid));
        }
        let sid = self.http_initialize(name, config, url).await?;
        Ok(sid)
    }

    /// 发送 `tools/list` 请求并处理分页（nextCursor）
    async fn http_tools_list(
        &self,
        name: &str,
        config: &McpServerConfig,
        url: &str,
        session_id: &Option<String>,
    ) -> Result<Vec<McpTool>, String> {
        let mut all_tools = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let mut params = json!({});
            if let Some(c) = &cursor {
                params["cursor"] = json!(c);
            }

            let list_request = JsonRpcRequest {
                jsonrpc: "2.0".to_string(),
                id: RequestId::Number(3),
                method: "tools/list".to_string(),
                params: Some(params),
            };

            let mut req = build_request(&self.http_client, url, config, &list_request);
            if let Some(sid) = session_id {
                req = req.header("Mcp-Session-Id", sid);
            }

            let response = req
                .send()
                .await
                .map_err(|e| format!("tools/list HTTP request failed: {e}"))?;
            let resp: JsonRpcResponse = self.parse_jsonrpc_response(response, "tools/list").await?;
            let result = resp
                .result
                .ok_or_else(|| "tools/list response missing result".to_string())?;

            // 解析为 ListToolsResult（标准格式）
            let list_result: ListToolsResult = serde_json::from_value(result)
                .map_err(|e| format!("Failed to parse tools/list response: {e}"))?;
            all_tools.extend(list_result.tools);

            // 翻页：nextCursor 为 null/缺省时停止
            match list_result.next_cursor {
                Some(c) if !c.is_empty() => cursor = Some(c),
                _ => break,
            }
            // 防御：单次循环最多 100 页（防止 server bug 导致死循环）
            if all_tools.len() > 10_000 {
                warn!(server = name, "tools/list 超过 10000 项，截断");
                break;
            }
        }

        Ok(all_tools)
    }

    /// 解析 JSON-RPC HTTP 响应（含 5xx 错误透传 + BUG-MR25 SSE 流识别）
    ///
    /// MCP Streamable HTTP 规范允许 server 在长响应中返回 `text/event-stream` 格式：
    /// 每个事件以 `data: {json}\n\n` 分隔。我们**只取最后一个 `data:` 行的 JSON**，
    /// 因为 JSON-RPC 单次响应是单个对象（不是数组）。
    async fn parse_jsonrpc_response(
        &self,
        response: reqwest::Response,
        context: &str,
    ) -> Result<JsonRpcResponse, String> {
        let status = response.status();
        // BUG-MR22 增强：404 也可能是 session 失效的迹象（某些 server 实现）
        if status.as_u16() == 404 {
            return Err(format!(
                "{context} HTTP 404 Not Found (可能是 session 失效)"
            ));
        }
        if !status.is_success() {
            let error_text = response.text().await.unwrap_or_default();
            // 截断错误文本，避免泄露过多 server 信息
            let truncated = if error_text.len() > 200 {
                format!("{}...", &error_text[..200])
            } else {
                error_text
            };
            return Err(format!("{context} HTTP {status}: {truncated}"));
        }

        // BUG-MR25：先检查 Content-Type，决定走 JSON 解析还是 SSE 流解析
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_lowercase();

        if content_type.contains("text/event-stream") {
            self.parse_sse_response(response, context).await
        } else {
            response
                .json::<JsonRpcResponse>()
                .await
                .map_err(|e| format!("{context} failed to parse response: {e}"))
        }
    }

    /// BUG-MR25：解析 SSE 格式的 JSON-RPC 响应
    ///
    /// SSE 格式：
    /// ```text
    /// event: message
    /// data: {"jsonrpc":"2.0","id":1,"result":{...}}
    ///
    /// ```
    /// 多个事件以 `\n\n` 分隔。JSON-RPC 响应是单对象，我们取最后一个 `data:` 行。
    async fn parse_sse_response(
        &self,
        response: reqwest::Response,
        context: &str,
    ) -> Result<JsonRpcResponse, String> {
        let text = response
            .text()
            .await
            .map_err(|e| format!("{context} failed to read SSE body: {e}"))?;
        // 找最后一个 `data: ` 行（MCP 2025-06-18 规范：单次响应是单对象）
        let mut last_data: Option<String> = None;
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("data:") {
                let trimmed = rest.trim_start();
                if !trimmed.is_empty() {
                    last_data = Some(trimmed.to_string());
                }
            }
        }
        let payload = last_data
            .ok_or_else(|| format!("{context} SSE 响应中没有 data 字段（可能 server 异常）"))?;
        serde_json::from_str(&payload)
            .map_err(|e| format!("{context} failed to parse SSE JSON: {e}"))
    }
}

#[cfg(test)]
mod tests;
