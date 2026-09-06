//! McpConfig / McpServerConfig 单元测试
//!
//! 对应源文件: `mcp_config.rs`

use super::*;

/// TEST-F1.1：McpServerConfig 序列化时 transport_type 序列化为 "type"
#[test]
fn mcp_server_config_serializes_transport_type_as_type() {
    let cfg = McpServerConfig {
        transport_type: McpTransportType::Http,
        command: None,
        args: None,
        env: None,
        url: Some("http://example.com/mcp".to_string()),
        headers: None,
        include_tools: None,
        exclude_tools: None,
        timeout_secs: None,
        enabled: true,
    };
    let v: serde_json::Value = serde_json::to_value(&cfg).unwrap();
    assert_eq!(v["type"], "http");
    assert_eq!(v["url"], "http://example.com/mcp");
}

/// TEST-F1.2：McpServerConfig 缺省字段在序列化时被省略
#[test]
fn mcp_server_config_omits_none_fields() {
    let cfg = McpServerConfig::default(); // 全 None + stdio + enabled=true
    let v: serde_json::Value = serde_json::to_value(&cfg).unwrap();
    assert!(v.get("command").is_none());
    assert!(v.get("args").is_none());
    assert!(v.get("env").is_none());
    assert!(v.get("url").is_none());
    assert!(v.get("include_tools").is_none());
    assert!(v.get("exclude_tools").is_none());
}

/// TEST-F1.3：McpServerConfig 反序列化时 transport_type 缺省为 stdio
#[test]
fn mcp_server_config_deserializes_missing_type_as_stdio() {
    let json = r#"{"command": "echo", "args": ["hi"]}"#;
    let cfg: McpServerConfig = serde_json::from_str(json).unwrap();
    assert_eq!(cfg.transport_type, McpTransportType::Stdio);
    assert_eq!(cfg.command, Some("echo".to_string()));
}

/// TEST-F1.4：McpServerConfig 反序列化时 enabled 缺省为 true
#[test]
fn mcp_server_config_enabled_defaults_true() {
    let json = r#"{"command": "echo"}"#;
    let cfg: McpServerConfig = serde_json::from_str(json).unwrap();
    assert!(cfg.enabled);
}

/// TEST-F1.5：McpServerConfig 反序列化所有字段名使用 snake_case
#[test]
fn mcp_server_config_deserializes_all_fields_snake_case() {
    let json = r#"{
        "type": "http",
        "url": "http://x",
        "include_tools": ["a", "b"],
        "exclude_tools": ["c"],
        "enabled": false
    }"#;
    let cfg: McpServerConfig = serde_json::from_str(json).unwrap();
    assert_eq!(cfg.transport_type, McpTransportType::Http);
    assert_eq!(cfg.url, Some("http://x".to_string()));
    assert_eq!(
        cfg.include_tools,
        Some(vec!["a".to_string(), "b".to_string()])
    );
    assert_eq!(cfg.exclude_tools, Some(vec!["c".to_string()]));
    assert!(!cfg.enabled);
}

/// TEST-F1.6：McpConfig 反序列化为空时为默认空 HashMap
#[test]
fn mcp_config_default_is_empty() {
    let cfg = McpConfig::default();
    assert!(cfg.servers.is_empty());
    let v = serde_json::to_value(&cfg).unwrap();
    assert_eq!(v, serde_json::json!({ "servers": {} }));
}
