//! McpToolCapability 单元测试
//!
//! 对应源文件: `capability.rs`

use super::*;
use crate::schemas::mcp_config::McpServerConfig;
use serde_json::json;
use symbio_plugin_api::CapabilityCategory;

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

/// 三段式命名 `mcp.<server>.<tool>`
#[test]
fn namespaced_name_three_part() {
    assert_eq!(
        McpToolCapability::namespaced_name("filesystem", "read"),
        "mcp.filesystem.read"
    );
}

/// meta 名称遵循三段式
#[test]
fn meta_uses_namespaced_name() {
    let cap = make_capability("git", "commit");
    let m = cap.meta();
    assert_eq!(m.name, "mcp.git.commit");
}

/// meta 描述前缀标记 server
#[test]
fn meta_description_prefixes_server() {
    let cap = make_capability("git", "commit");
    let m = cap.meta();
    assert!(m.description.starts_with("[MCP:git] "));
    assert!(m.description.contains("commit desc"));
}

/// meta 分类为 Mcp
#[test]
fn meta_category_is_mcp() {
    let cap = make_capability("git", "commit");
    let m = cap.meta();
    assert_eq!(m.category, Some(CapabilityCategory::Mcp));
}

/// input_schema 透传
#[test]
fn meta_input_schema_passthrough() {
    let cap = make_capability("git", "commit");
    let m = cap.meta();
    assert_eq!(
        m.input_schema,
        json!({ "type": "object", "properties": { "q": { "type": "string" } } })
    );
}

/// 同 server 不同 tool 名不冲突
#[test]
fn namespaced_name_avoids_collision() {
    let a = McpToolCapability::namespaced_name("server1", "search");
    let b = McpToolCapability::namespaced_name("server2", "search");
    assert_ne!(a, b);
    assert_eq!(a, "mcp.server1.search");
    assert_eq!(b, "mcp.server2.search");
}

/// 嵌套对象里的 path / url / command 按键名投影，未知字符串仍可作为参数事实约束。
#[test]
fn facts_project_nested_semantic_arguments() {
    let facts = intent_facts(
        "mcp.generic.run",
        &json!({
            "request": {
                "filePath": r"C:\Repo\docs\a.md",
                "target": { "endpoint": "HTTPS://Example.COM:443/a/../b?q=1#frag" },
                "steps": [
                    { "shellCommand": "  echo ok  " },
                    { "directory": "docs\\api" },
                    "free text"
                ]
            }
        }),
    );

    assert_eq!(facts[0], SandboxFact::new("mcp_tool", "mcp.generic.run"));
    assert!(facts.contains(&SandboxFact::new("vdfs_path", "c:/Repo/docs/a.md")));
    assert!(facts.contains(&SandboxFact::new("vdfs_path", "docs/api")));
    assert!(facts.contains(&SandboxFact::new("url", "https://example.com/b?q=1")));
    assert!(facts.contains(&SandboxFact::new("shell_command", "echo ok")));

    let arguments: Vec<&str> = facts
        .iter()
        .filter(|fact| fact.kind == "mcp_argument")
        .map(|fact| fact.value.as_str())
        .collect();
    assert_eq!(arguments.len(), 5);
    assert!(arguments.contains(&"free text"));
    assert!(arguments.contains(&"  echo ok  "));
}

/// 数组元素继承 key 的语义；非字符串值不产生参数事实。
#[test]
fn facts_cover_arrays_and_normalize_each_string() {
    let facts = intent_facts(
        "mcp.generic.batch",
        &json!({
            "paths": [r"C:\A\one.txt", "./two.txt"],
            "urls": ["https://a.test/x#frag", "https://b.test/y"],
            "commands": [" ls ", "pwd"],
            "mixed": [1, true, null, { "path": "nested/file" }]
        }),
    );

    assert!(facts.contains(&SandboxFact::new("vdfs_path", "c:/A/one.txt")));
    assert!(facts.contains(&SandboxFact::new("vdfs_path", "two.txt")));
    assert!(facts.contains(&SandboxFact::new("vdfs_path", "nested/file")));
    assert!(facts.contains(&SandboxFact::new("url", "https://a.test/x")));
    assert!(facts.contains(&SandboxFact::new("url", "https://b.test/y")));
    assert!(facts.contains(&SandboxFact::new("shell_command", "ls")));
    assert!(facts.contains(&SandboxFact::new("shell_command", "pwd")));
    assert_eq!(
        facts
            .iter()
            .filter(|fact| fact.kind == "mcp_argument")
            .count(),
        7
    );
}

/// 敏感 key 命中时连子树一起跳过，值不会进入任何可观察事实或语义投影。
#[test]
fn facts_skip_sensitive_fields_and_subtrees() {
    let facts = intent_facts(
        "mcp.generic.auth",
        &json!({
            "password": "pw-value",
            "passwd": "passwd-value",
            "cookie": "cookie-value",
            "authorization": "Bearer auth-value",
            "auth": {
                "access_token": "token-value",
                "nested": [{ "clientSecret": "secret-value" }]
            },
            "apiKey": "key-value",
            "tokenBlob": { "path": "/hidden/token", "value": "hidden-value" },
            "secretPath": "/hidden/secret",
            "safe": "visible",
            "items": [{ "key": "nested-key-value", "file": "safe/file.txt" }]
        }),
    );

    for secret in [
        "pw-value",
        "passwd-value",
        "cookie-value",
        "auth-value",
        "token-value",
        "secret-value",
        "key-value",
        "/hidden/token",
        "hidden-value",
        "/hidden/secret",
        "nested-key-value",
    ] {
        assert!(
            facts.iter().all(|fact| !fact.value.contains(secret)),
            "敏感值泄露到事实: {secret}"
        );
    }
    assert!(facts.contains(&SandboxFact::new("mcp_argument", "visible")));
    assert!(facts.contains(&SandboxFact::new("mcp_argument", "safe/file.txt")));
    assert!(facts.contains(&SandboxFact::new("vdfs_path", "safe/file.txt")));
    assert_eq!(
        facts
            .iter()
            .filter(|fact| fact.kind == "mcp_argument")
            .count(),
        2
    );
}

#[test]
fn embedded_url_credentials_are_redacted_before_projection() {
    let facts = intent_facts(
        "mcp.generic.fetch",
        &json!({"endpoint": "https://alice:secret@example.com/private?q=1"}),
    );
    assert!(facts.contains(&SandboxFact::new("url", "https://example.com/private?q=1")));
    assert!(facts
        .iter()
        .all(|fact| { !fact.value.contains("alice") && !fact.value.contains("secret") }));
}

/// 没有可识别参数时仍保留稳定的 server/tool 身份，供 blocked_tools 等规则约束。
#[test]
fn facts_always_include_tool_identity() {
    for args in [
        json!({}),
        json!([]),
        Value::Null,
        json!({ "count": 1, "enabled": true }),
    ] {
        assert_eq!(
            intent_facts("mcp.generic.ping", &args),
            vec![SandboxFact::new("mcp_tool", "mcp.generic.ping")]
        );
    }
}
