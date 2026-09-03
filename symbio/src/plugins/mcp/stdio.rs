//! MCP stdio transport
//!
//! 每次调用都 **spawn 新的子进程**，与 MCP server 通信完成后立即 kill。
//! 不持有长连接——这是"按需加载（lazy）"设计的一部分：
//! agent 真正请求某个 MCP 工具时才会启动 stdio 进程。
//!
//! ## MCP 协议握手顺序
//!
//! 1. 客户端 → `initialize`（请求 id=1）
//! 2. 服务器 → `initialize` 响应
//! 3. 客户端 → `notifications/initialized`（**无 id 的 notification**，无响应）
//! 4. 客户端 → `tools/call` / `tools/list`（请求 id=2/3...）
//!
//! 第 3 步是关键：MCP 规范要求 `initialize` 后必须发送 `notifications/initialized`，
//! 否则大多数 server 会拒绝后续请求（视为"未握手完成"）。
//!
//! ## 进程管理
//!
//! - spawn 后必须设置 `kill_on_drop(true)`（保证 panic / drop 时回收子进程）
//! - kill 后调用 `wait()` 回收，避免 zombie
//! - `read_until` 使用 `tokio::time::timeout` 防止 server 卡住时永久阻塞
//! - stderr 启动独立 task 持续读取，避免 pipe 缓冲区满导致子进程阻塞

use super::manager::{McpManager, TestConnectionResult};
use super::types::{
    JsonRpcRequest, JsonRpcResponse, McpInitializeResponse, McpTool, McpToolCallResponse,
    RequestId, DEFAULT_PROTOCOL_VERSION, SUPPORTED_PROTOCOL_VERSIONS,
};
use crate::symbio_core::schemas::mcp::mcp_config::McpServerConfig;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::process::Command;
use tracing::{debug, info, warn};

/// stdio 读超时（discover / call 各 30s）
const STDIO_READ_TIMEOUT: Duration = Duration::from_secs(30);

/// graceful shutdown 等待时间（先 SIGTERM，超时后 SIGKILL）
const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

/// 构造 `initialize` 请求的 `params`
///
/// 客户端声明支持的协议版本（多个），让 server 协商选择。
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

/// 选择实际使用的协议版本（server 返回的版本如果在客户端支持列表里，
/// 则用 server 的；否则 fallback 到默认）。
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

impl McpManager {
    /// 通过 stdio 发现工具
    pub async fn discover_tools_stdio(
        &self,
        config: &McpServerConfig,
    ) -> Result<Vec<McpTool>, String> {
        let (mut child, mut stdout, _negotiated, _server_name, _server_version, _instructions) =
            stdio_handshake(config).await?;

        // 4) BUG-MR21 修复：显式调用 `tools/list`，因为很多 server 不在 initialize 响应中返回 tools
        let tools = match stdio_tools_list(&mut child, &mut stdout, STDIO_READ_TIMEOUT).await {
            Ok(t) => t,
            Err(e) => {
                // 优雅关闭后返回错误（list 失败不阻塞）
                shutdown_child_graceful(&mut child).await;
                return Err(e);
            }
        };

        // 5) graceful shutdown
        shutdown_child_graceful(&mut child).await;

        Ok(tools)
    }

    /// 测试 stdio MCP server 的连接（完整握手 + 一次 `tools/list`）
    ///
    /// 不修改任何缓存或配置。仅用于"用户点击测试连接"时的可用性验证。
    ///
    /// 连接测试能力：供统一 `resources` 连接测试复用
    #[allow(dead_code)]
    pub async fn test_connection_stdio(
        &self,
        config: &McpServerConfig,
    ) -> Result<TestConnectionResult, String> {
        let (mut child, mut stdout, negotiated, server_name, server_version, instructions) =
            stdio_handshake(config).await?;
        let tools = match stdio_tools_list(&mut child, &mut stdout, STDIO_READ_TIMEOUT).await {
            Ok(t) => t,
            Err(e) => {
                shutdown_child_graceful(&mut child).await;
                return Err(e);
            }
        };
        shutdown_child_graceful(&mut child).await;
        Ok(TestConnectionResult {
            tool_count: tools.len(),
            protocol_version: negotiated,
            server_name: Some(server_name),
            server_version,
            instructions,
            elapsed_ms: 0,
        })
    }

    /// 通过 stdio 调用工具
    pub async fn call_tool_stdio(
        &self,
        config: &McpServerConfig,
        tool_name: &str,
        args: Value,
    ) -> Result<Value, String> {
        let (mut child, mut stdout, _negotiated, _server_name, _server_version, _instructions) =
            stdio_handshake(config).await?;

        // 2) tools/call（initialize 已用 id=1）
        let call_request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(2),
            method: "tools/call".to_string(),
            params: Some(json!({
                "name": tool_name,
                "arguments": args
            })),
        };
        if let Some(stdin) = child.stdin.as_mut() {
            let line = format!(
                "{}\n",
                serde_json::to_string(&call_request).map_err(|e| e.to_string())?
            );
            stdin
                .write_all(line.as_bytes())
                .await
                .map_err(|e| format!("Failed to write tools/call request: {e}"))?;
            let _ = stdin.flush().await;
        }

        // 3) 读取 tools/call 响应
        let response: JsonRpcResponse =
            read_one_jsonrpc_line(&mut stdout, STDIO_READ_TIMEOUT).await?;

        shutdown_child_graceful(&mut child).await;

        if let Some(error) = response.error {
            return Err(format!(
                "Tool call error: {} - {}{}",
                error.code,
                error.message,
                error
                    .data
                    .as_ref()
                    .map(|d| format!(" ({d})"))
                    .unwrap_or_default()
            ));
        }
        let result = response
            .result
            .ok_or_else(|| "tools/call response missing result".to_string())?;

        // 优先按 McpToolCallResponse 解析
        if let Ok(tool_response) = serde_json::from_value::<McpToolCallResponse>(result.clone()) {
            if let Some(error) = tool_response.error {
                return Err(format!(
                    "Tool call error: {} - {}{}",
                    error.code,
                    error.message,
                    error
                        .data
                        .as_ref()
                        .map(|d| format!(" ({d})"))
                        .unwrap_or_default()
                ));
            }
            // tool 自身标记的失败（isError=true）
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
        // 否则直接返回原始 result
        Ok(result)
    }
}

/// 优雅关闭子进程：
/// - 先尝试 wait 短时间（让 server 自行清理）
/// - 超时后强制 kill + wait
///
/// 避免直接 SIGKILL 造成 server 状态损坏（特别是 LSP / 数据库型 server）。
async fn shutdown_child_graceful(child: &mut tokio::process::Child) {
    // 尝试优雅退出（部分 server 收到 EOF stdin 后会自行退出）
    let _ = tokio::time::timeout(GRACEFUL_SHUTDOWN_TIMEOUT, child.wait()).await;
    // 强制 kill 并 wait
    let _ = child.kill().await;
    let _ = child.wait().await;
}

/// stdio transport 完整握手：spawn 子进程 + initialize + notifications/initialized
///
/// 返回 `(child, stdout_buf, 协商后的 protocol_version, server_name, server_version)`。
/// 调用方负责后续 `tools/list` / `tools/call` 和子进程关闭。
async fn stdio_handshake(
    config: &McpServerConfig,
) -> Result<
    (
        tokio::process::Child,
        tokio::io::BufReader<tokio::process::ChildStdout>,
        String,
        String,
        Option<String>,
        Option<String>,
    ),
    String,
> {
    let command = config
        .command
        .as_ref()
        .ok_or("stdio transport requires 'command' field")?;

    let mut cmd = Command::new(command);
    if let Some(args) = &config.args {
        cmd.args(args);
    }
    if let Some(env) = &config.env {
        for (k, v) in env {
            cmd.env(k, v);
        }
    }
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Failed to start process: {e}"))?;

    // 启动独立 task 读取 stderr（避免 pipe 满）
    drain_stderr(&mut child);

    // 1) initialize 请求（id=1）
    let request = JsonRpcRequest {
        jsonrpc: "2.0".to_string(),
        id: RequestId::Number(1),
        method: "initialize".to_string(),
        params: Some(build_initialize_params()),
    };
    if let Some(stdin) = child.stdin.as_mut() {
        let line = format!(
            "{}\n",
            serde_json::to_string(&request).map_err(|e| e.to_string())?
        );
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| format!("Failed to write initialize request: {e}"))?;
        stdin
            .flush()
            .await
            .map_err(|e| format!("Failed to flush stdin: {e}"))?;
    }

    // 2) 读取 initialize 响应
    let stdout = child
        .stdout
        .take()
        .ok_or("child has no stdout".to_string())?;
    let mut stdout = tokio::io::BufReader::new(stdout);
    let response: JsonRpcResponse = read_one_jsonrpc_line(&mut stdout, STDIO_READ_TIMEOUT).await?;
    debug!(resp_id = %response.id_display(), "initialize 响应");

    let result = response
        .result
        .ok_or_else(|| "initialize response missing result".to_string())?;

    let initialize_response: McpInitializeResponse = serde_json::from_value(result)
        .map_err(|e| format!("Failed to parse initialize response: {e}"))?;
    let negotiated = negotiate_protocol_version(&initialize_response.protocol_version);
    let server_name = initialize_response.server_info.name.clone();
    let server_version = initialize_response.server_info.version.clone();
    info!(
        server = %server_name,
        server_version = %server_version.as_deref().unwrap_or("unknown"),
        protocol = %negotiated,
        server_protocol = %initialize_response.protocol_version,
        list_changed = ?initialize_response.capabilities.tools.as_ref().map(|t| t.list_changed),
        extra_caps = ?initialize_response.capabilities.extra,
        // BUG-MR32：透传 server instructions
        has_instructions = initialize_response.instructions.is_some(),
        "Connected to MCP server (stdio)"
    );

    // 3) 发送 notifications/initialized（无 id，无响应）
    if let Some(stdin) = child.stdin.as_mut() {
        let notification = json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        });
        let line = format!("{notification}\n");
        let _ = stdin.write_all(line.as_bytes()).await;
        let _ = stdin.flush().await;
    }

    Ok((
        child,
        stdout,
        negotiated,
        server_name,
        server_version,
        initialize_response.instructions.clone(),
    ))
}

/// 通过 stdio 调用 `tools/list` 并支持分页（nextCursor）
async fn stdio_tools_list<R: tokio::io::AsyncBufRead + Unpin>(
    child: &mut tokio::process::Child,
    stdout: &mut R,
    timeout: Duration,
) -> Result<Vec<McpTool>, String> {
    let mut all_tools: Vec<McpTool> = Vec::new();
    let mut cursor: Option<String> = None;
    let mut request_id: i64 = 2; // initialize 已用 id=1

    loop {
        let mut params = json!({});
        if let Some(c) = &cursor {
            params["cursor"] = json!(c);
        }

        let list_request = JsonRpcRequest {
            jsonrpc: "2.0".to_string(),
            id: RequestId::Number(request_id),
            method: "tools/list".to_string(),
            params: Some(params),
        };
        request_id += 1;

        if let Some(stdin) = child.stdin.as_mut() {
            let line = format!(
                "{}\n",
                serde_json::to_string(&list_request).map_err(|e| e.to_string())?
            );
            stdin
                .write_all(line.as_bytes())
                .await
                .map_err(|e| format!("Failed to write tools/list request: {e}"))?;
            let _ = stdin.flush().await;
        } else {
            return Err("child stdin unavailable for tools/list".to_string());
        }

        let response: JsonRpcResponse = read_one_jsonrpc_line(stdout, timeout).await?;
        if let Some(error) = response.error {
            return Err(format!(
                "tools/list error: {} - {}{}",
                error.code,
                error.message,
                error
                    .data
                    .as_ref()
                    .map(|d| format!(" ({d})"))
                    .unwrap_or_default()
            ));
        }
        let result = response
            .result
            .ok_or_else(|| "tools/list response missing result".to_string())?;
        let list_result: super::types::ListToolsResult = serde_json::from_value(result)
            .map_err(|e| format!("Failed to parse tools/list response: {e}"))?;
        all_tools.extend(list_result.tools);

        // 翻页：nextCursor 为 null/缺省时停止
        match list_result.next_cursor {
            Some(c) if !c.is_empty() => cursor = Some(c),
            _ => break,
        }
        // 防御：单次循环最多 100 页（防止 server bug 导致死循环）
        if all_tools.len() > 10_000 {
            warn!("tools/list 超过 10000 项，截断");
            break;
        }
    }

    Ok(all_tools)
}

/// 从 child 的 stderr 持续 drain 出来（避免 pipe 满导致子进程阻塞）
fn drain_stderr(child: &mut tokio::process::Child) {
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(async move {
            let mut reader = tokio::io::BufReader::new(stderr);
            let mut buf = Vec::new();
            loop {
                buf.clear();
                match reader.read_until(b'\n', &mut buf).await {
                    Ok(0) => break,
                    Ok(_) => {
                        let line = decode_line(&buf);
                        if !line.is_empty() {
                            tracing::debug!(target: "mcp_stderr", "{}", line);
                        }
                    }
                    Err(_) => break,
                }
            }
        });
    }
}

/// 从 stdout 读取一行 JSON-RPC 响应（含超时）
async fn read_one_jsonrpc_line<R: tokio::io::AsyncBufRead + Unpin>(
    reader: &mut R,
    timeout: Duration,
) -> Result<JsonRpcResponse, String> {
    let mut buf = Vec::new();
    let n = tokio::time::timeout(timeout, reader.read_until(b'\n', &mut buf))
        .await
        .map_err(|_| format!("MCP stdio read timed out after {timeout:?}"))?
        .map_err(|e| format!("MCP stdio read error: {e}"))?;
    if n == 0 {
        return Err("MCP stdio EOF before response".to_string());
    }
    let line = decode_line(&buf);
    serde_json::from_str(&line).map_err(|e| format!("MCP stdio parse error: {e}"))
}

/// 解码一行字节到字符串
///
/// Windows 平台优先尝试 GBK（兼容部分中文 MCP server），失败回退 UTF-8。
/// 其他平台直接 UTF-8。
fn decode_line(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    #[cfg(target_os = "windows")]
    {
        let (res, _, has_errors) = encoding_rs::GBK.decode(bytes);
        if !has_errors {
            return res.to_string();
        }
    }
    let _ = STDIO_READ_TIMEOUT; // suppress dead_code on non-windows
    String::from_utf8_lossy(bytes)
        .trim_end_matches('\n')
        .trim_end_matches('\r')
        .to_string()
}
