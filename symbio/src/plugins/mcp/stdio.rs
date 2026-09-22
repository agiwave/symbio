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
use crate::plugins::mcp::schemas::mcp_config::McpServerConfig;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::process::Command;
use tracing::{debug, info, warn};

/// stdio 读超时（discover / call 各 30s）
const STDIO_READ_TIMEOUT: Duration = Duration::from_secs(30);

/// graceful shutdown 等待时间（先关闭 stdin，超时后强制终止）
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

        // 4) BUG-MR21：显式调用 `tools/list`，因为很多 server 不在 initialize 响应中返回 tools
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
    /// 连接测试能力：供详情表单的 `test` 动作（`vdfs/action`）复用
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

        // `result` **就是**规范里的 `CallToolResult`（`{content, isError, …}`），
        // 不是 JSON-RPC 信封——信封层（`error`）已在上方处理过。
        // 这里只借它读 `isError`：工具**自己**声明失败时转成 Err，
        // 否则模型会把失败当成功结果继续往下推理。
        //
        // 曾经这里按信封建模，于是 `{content, isError}` 反序列化「成功」
        // 却把内容全丢（三个字段都可选），返回 `Value::Null`——工具白跑，
        // 模型收到字符串 `"null"`，且全程无报错。见 `McpToolCallResponse` 文档。
        let call_result: McpToolCallResponse =
            serde_json::from_value(result.clone()).unwrap_or_default();
        if call_result.is_error == Some(true) {
            return Err(format!("Tool returned error: {}", call_result.text()));
        }
        Ok(result)
    }
}

/// 优雅关闭子进程：
/// - 先关闭 stdin 发送 EOF，再 wait 短时间让 server 自行清理
/// - 超时后强制 kill + wait
///
/// 避免直接 SIGKILL 造成 server 状态损坏（特别是 LSP / 数据库型 server）。
async fn shutdown_child_graceful(child: &mut tokio::process::Child) {
    // Tokio Child::wait 会关闭仍由 Child 持有的 stdin，发送 EOF。
    if let Ok(Ok(_)) = tokio::time::timeout(GRACEFUL_SHUTDOWN_TIMEOUT, child.wait()).await {
        return;
    }
    // 超时或等待失败时强制终止并回收。
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
        stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| format!("Failed to write notifications/initialized: {e}"))?;
        stdin
            .flush()
            .await
            .map_err(|e| format!("Failed to flush notifications/initialized: {e}"))?;
    } else {
        return Err("child stdin unavailable for notifications/initialized".to_string());
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
/// MCP 规范：stdio 传输是 UTF-8。因此**先按 UTF-8 严格校验**，合法即原样采用；
/// 仅当字节流不是合法 UTF-8 时才回退 GBK（兼容个别在 Windows 上按系统 ANSI
/// 编码输出的中文 MCP server）。
///
/// 顺序不能反过来：GBK 几乎能「无错」解码任何 UTF-8 字节流（`has_errors`
/// 对规范 server 的非 ASCII 内容几乎不触发），先 GBK 会把所有规范 server 的
/// 中文内容静默变成乱码再喂给模型 —— e2e（`e2e/mock-mcp.mjs`）已实测复现。
/// 两种编码的有效字节流几乎不相交，所以「先严格 UTF-8、失败再 GBK」是可靠的。
fn decode_line(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return String::new();
    }
    let trim = |s: String| s.trim_end_matches('\n').trim_end_matches('\r').to_string();
    match std::str::from_utf8(bytes) {
        Ok(s) => trim(s.to_string()),
        #[cfg(target_os = "windows")]
        Err(_) => {
            let (res, _, has_errors) = encoding_rs::GBK.decode(bytes);
            if has_errors {
                // 两种编码都解不出来：退回 UTF-8 丢替换符，至少 ASCII 结构可读
                trim(String::from_utf8_lossy(bytes).into_owned())
            } else {
                trim(res.to_string())
            }
        }
        #[cfg(not(target_os = "windows"))]
        Err(_) => trim(String::from_utf8_lossy(bytes).into_owned()),
    }
}

#[cfg(test)]
#[path = "stdio.test.rs"]
mod tests;
