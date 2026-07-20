//! McpToolCapability 单元测试
//!
//! 对应源文件: `capability.rs`

use super::*;
use crate::symbio_core::schemas::mcp::mcp_config::McpServerConfig;
use crate::symbio_core::CapabilityCategory;
use serde_json::json;

fn make_capability(server: &str, tool_name: &str) -> McpToolCapability {
    let tool = McpTool {
        name: tool_name.to_string(),
        description: format!("{tool_name} desc"),
        input_schema: json!({ "type": "object", "properties": { "q": { "type": "string" } } }),
        annotations: None,
    };
    let config = McpServerConfig::default();
    let manager = McpManager::new();
    McpToolCapability::new(server.to_string(), tool, config, Arc::new(manager))
}

/// TEST-M7.1：三段式命名 `mcp.<server>.<tool>`
#[test]
fn namespaced_name_three_part() {
    assert_eq!(
        McpToolCapability::namespaced_name("filesystem", "read"),
        "mcp.filesystem.read"
    );
}

/// TEST-M7.2：meta 名称遵循三段式
#[test]
fn meta_uses_namespaced_name() {
    let cap = make_capability("git", "commit");
    let m = cap.meta();
    assert_eq!(m.name, "mcp.git.commit");
}

/// TEST-M7.3：meta 描述前缀标记 server
#[test]
fn meta_description_prefixes_server() {
    let cap = make_capability("git", "commit");
    let m = cap.meta();
    assert!(m.description.starts_with("[MCP:git] "));
    assert!(m.description.contains("commit desc"));
}

/// TEST-M7.4：meta 分类为 Mcp
#[test]
fn meta_category_is_mcp() {
    let cap = make_capability("git", "commit");
    let m = cap.meta();
    assert_eq!(m.category, Some(CapabilityCategory::Mcp));
}

/// TEST-M7.5：input_schema 透传
#[test]
fn meta_input_schema_passthrough() {
    let cap = make_capability("git", "commit");
    let m = cap.meta();
    assert_eq!(
        m.input_schema,
        json!({ "type": "object", "properties": { "q": { "type": "string" } } })
    );
}

/// TEST-M7.6：同 server 不同 tool 名不冲突
#[test]
fn namespaced_name_avoids_collision() {
    let a = McpToolCapability::namespaced_name("server1", "search");
    let b = McpToolCapability::namespaced_name("server2", "search");
    assert_ne!(a, b);
    assert_eq!(a, "mcp.server1.search");
    assert_eq!(b, "mcp.server2.search");
}
