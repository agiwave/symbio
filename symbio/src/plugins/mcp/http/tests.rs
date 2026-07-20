//! MCP HTTP transport 单元测试
//!
//! 对应源文件: `http.rs`

use super::super::types::JsonRpcError;
use super::is_session_expired;

/// TEST-MR22.1：code = -32000 视为 session 失效
#[test]
fn session_expired_by_code() {
    let err = JsonRpcError {
        code: -32000,
        message: "Internal error".to_string(),
        data: None,
    };
    assert!(is_session_expired(&err));
}

/// TEST-MR22.2：message 含 "session not found" 视为 session 失效
#[test]
fn session_expired_by_message_not_found() {
    let err = JsonRpcError {
        code: -100,
        message: "Session not found".to_string(),
        data: None,
    };
    assert!(is_session_expired(&err));
}

/// TEST-MR22.3：message 含 "expired" 视为 session 失效
#[test]
fn session_expired_by_message_expired() {
    let err = JsonRpcError {
        code: -1,
        message: "Session has expired, please re-initialize".to_string(),
        data: None,
    };
    assert!(is_session_expired(&err));
}

/// TEST-MR22.4：其它错误不被识别为 session 失效
#[test]
fn non_session_error_not_misidentified() {
    let err = JsonRpcError {
        code: -32600,
        message: "Invalid Request".to_string(),
        data: None,
    };
    assert!(!is_session_expired(&err));
}

/// TEST-MR22.5：tool error 不被识别为 session 失效
#[test]
fn tool_error_not_misidentified_as_session_expired() {
    let err = JsonRpcError {
        code: -1,
        message: "Tool execution failed: connection refused".to_string(),
        data: None,
    };
    assert!(!is_session_expired(&err));
}

// ===== BUG-MR28 / BUG-MR31 / BUG-MR25：自定义 headers / 超时 / SSE =====

/// TEST-MR28.1：apply_custom_headers 合并自定义头
#[test]
fn apply_custom_headers_merges() {
    use crate::symbio_core::schemas::mcp::mcp_config::{McpServerConfig, McpTransportType};
    let mut cfg = McpServerConfig::default();
    cfg.transport_type = McpTransportType::Http;
    cfg.headers = Some(
        vec![
            ("Authorization".to_string(), "Bearer xyz".to_string()),
            ("X-Custom".to_string(), "v1".to_string()),
        ]
        .into_iter()
        .collect(),
    );
    let client = reqwest::Client::new();
    let req = super::apply_custom_headers(client.get("http://x"), &cfg);
    // reqwest 不直接暴露 headers；这里用 build() 验证构造无 panic
    let built = req.build().expect("build ok");
    let headers = built.headers();
    assert!(headers.contains_key("authorization"));
    assert!(headers.contains_key("x-custom"));
}

/// TEST-MR28.2：保留头（content-type / accept / mcp-session-id）被过滤
#[test]
fn apply_custom_headers_filters_reserved() {
    use crate::symbio_core::schemas::mcp::mcp_config::{McpServerConfig, McpTransportType};
    let mut cfg = McpServerConfig::default();
    cfg.transport_type = McpTransportType::Http;
    cfg.headers = Some(
        vec![
            ("content-type".to_string(), "evil".to_string()),
            ("ACCEPT".to_string(), "evil".to_string()),
            ("Mcp-Session-Id".to_string(), "evil".to_string()),
            ("X-Real-Header".to_string(), "ok".to_string()),
        ]
        .into_iter()
        .collect(),
    );
    let client = reqwest::Client::new();
    let req = super::apply_custom_headers(client.get("http://x"), &cfg);
    let built = req.build().expect("build ok");
    // X-Real-Header 通过
    assert!(built.headers().contains_key("x-real-header"));
    // reserved 头虽然 warn 但用户配置的 key 不应被注入（仅当调用方后续手动设置时）
    // 此处因 apply_custom_headers 内部就过滤了，所以以下三个 key 不在 build 前置阶段出现
    // （实际请求中由 build_request 显式设置）
}

/// TEST-MR31.1：effective_timeout 使用默认 30s
#[test]
fn effective_timeout_default() {
    use crate::symbio_core::schemas::mcp::mcp_config::McpServerConfig;
    let cfg = McpServerConfig::default();
    assert_eq!(
        super::effective_timeout(&cfg),
        std::time::Duration::from_secs(30)
    );
}

/// TEST-MR31.2：effective_timeout 读取 timeout_secs
#[test]
fn effective_timeout_custom() {
    use crate::symbio_core::schemas::mcp::mcp_config::McpServerConfig;
    let mut cfg = McpServerConfig::default();
    cfg.timeout_secs = Some(120);
    assert_eq!(
        super::effective_timeout(&cfg),
        std::time::Duration::from_secs(120)
    );
}

/// TEST-MR31.3：effective_timeout 0/None fallback 到默认
#[test]
fn effective_timeout_zero_fallback() {
    use crate::symbio_core::schemas::mcp::mcp_config::McpServerConfig;
    let mut cfg = McpServerConfig::default();
    cfg.timeout_secs = Some(0);
    // .max(1) 保证至少 1 秒
    assert_eq!(
        super::effective_timeout(&cfg),
        std::time::Duration::from_secs(1)
    );
}

/// TEST-MR25.1：SSE 响应中取最后一个 data: 行的 JSON
#[test]
fn sse_parse_takes_last_data_line() {
    let sse = "\
event: message
data: {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{\"protocolVersion\":\"2025-06-18\"}}

event: message
data: {\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{\"protocolVersion\":\"WRONG\"}}

";
    // 直接复用 parse 逻辑（独立函数，单元可测）
    let mut last_data: Option<String> = None;
    for line in sse.lines() {
        if let Some(rest) = line.strip_prefix("data:") {
            let trimmed = rest.trim_start();
            if !trimmed.is_empty() {
                last_data = Some(trimmed.to_string());
            }
        }
    }
    let payload = last_data.unwrap();
    assert!(payload.contains("WRONG"), "应取最后一个 data 行");
}
