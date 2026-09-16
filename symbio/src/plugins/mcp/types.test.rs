//! MCP 协议类型 单元测试
//!
//! 对应源文件: `types.rs`

use super::*;
use serde_json::json;

fn tool(name: &str) -> McpTool {
    McpTool {
        name: name.to_string(),
        description: format!("{name} tool"),
        input_schema: json!({ "type": "object" }),
        annotations: None,
    }
}

/// TEST-M1.1：无 include / exclude 时全部通过
#[test]
fn filter_tools_none_passes_all() {
    let tools = vec![tool("a"), tool("b"), tool("c")];
    let filtered = filter_tools(tools, &None, &None);
    assert_eq!(filtered.len(), 3);
}

/// TEST-M1.2：include 白名单
#[test]
fn filter_tools_include_whitelist() {
    let tools = vec![tool("a"), tool("b"), tool("c")];
    let include = Some(vec!["a".to_string(), "c".to_string()]);
    let filtered = filter_tools(tools, &include, &None);
    let names: Vec<&str> = filtered.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, vec!["a", "c"]);
}

/// TEST-M1.3：exclude 黑名单
#[test]
fn filter_tools_exclude_blacklist() {
    let tools = vec![tool("a"), tool("b"), tool("c")];
    let exclude = Some(vec!["b".to_string()]);
    let filtered = filter_tools(tools, &None, &exclude);
    let names: Vec<&str> = filtered.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, vec!["a", "c"]);
}

/// TEST-M1.4：include + exclude 同时设置，exclude 优先
#[test]
fn filter_tools_include_then_exclude() {
    let tools = vec![tool("a"), tool("b"), tool("c")];
    let include = Some(vec!["a".to_string(), "b".to_string(), "c".to_string()]);
    let exclude = Some(vec!["b".to_string()]);
    let filtered = filter_tools(tools, &include, &exclude);
    let names: Vec<&str> = filtered.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, vec!["a", "c"]);
}

/// TEST-M1.5：include 空列表 → 全部过滤掉
#[test]
fn filter_tools_empty_include_filters_all() {
    let tools = vec![tool("a"), tool("b")];
    let include = Some(vec![]);
    let filtered = filter_tools(tools, &include, &None);
    assert!(filtered.is_empty());
}

/// TEST-M1.6：exclude 名称不存在的工具 → 不影响
#[test]
fn filter_tools_exclude_nonexistent_no_op() {
    let tools = vec![tool("a"), tool("b")];
    let exclude = Some(vec!["zzz".to_string()]);
    let filtered = filter_tools(tools, &None, &exclude);
    assert_eq!(filtered.len(), 2);
}

// ===== BUG-MR27：tool name 校验 =====

/// TEST-MR27.1：合法名称（字母/数字/下划线/连字符）
#[test]
fn validate_tool_name_accepts_valid() {
    assert!(validate_tool_name("search").is_ok());
    assert!(validate_tool_name("read_file").is_ok());
    assert!(validate_tool_name("git-commit").is_ok());
    assert!(validate_tool_name("Tool1").is_ok());
    assert!(validate_tool_name("a").is_ok());
    assert!(
        validate_tool_name(&"x".repeat(64)).is_ok(),
        "64 字符边界合法"
    );
}

/// TEST-MR27.2：空名拒绝
#[test]
fn validate_tool_name_rejects_empty() {
    let err = validate_tool_name("").unwrap_err();
    assert!(err.contains("不能为空"));
}

/// TEST-MR27.3：超长名称拒绝
#[test]
fn validate_tool_name_rejects_too_long() {
    let err = validate_tool_name(&"x".repeat(65)).unwrap_err();
    assert!(err.contains("64 字符"));
}

/// TEST-MR27.4：非法字符（空格/点/中文等）拒绝
#[test]
fn validate_tool_name_rejects_invalid_chars() {
    assert!(validate_tool_name("with space").is_err());
    assert!(validate_tool_name("with.dot").is_err());
    assert!(validate_tool_name("with/slash").is_err());
    assert!(validate_tool_name("中文").is_err());
    assert!(validate_tool_name("with😀emoji").is_err());
}

/// TEST-MR27.5：filter_valid_tool_names 保留合法 / 计数非法
#[test]
fn filter_valid_tool_names_separates() {
    let tools = vec![
        tool("search"),
        tool("read_file"),
        tool("with space"),
        tool("中文"),
        tool("git-commit"),
    ];
    let (valid, invalid) = filter_valid_tool_names(tools);
    assert_eq!(valid.len(), 3);
    assert_eq!(invalid, 2);
    let names: Vec<&str> = valid.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, vec!["search", "read_file", "git-commit"]);
}

/// TEST-MR27.6：filter_valid_tool_names 全空 / 全合法
#[test]
fn filter_valid_tool_names_edge_cases() {
    assert_eq!(filter_valid_tool_names(vec![]).0.len(), 0);
    let only_valid = vec![tool("a"), tool("b")];
    let (v, n) = filter_valid_tool_names(only_valid);
    assert_eq!(v.len(), 2);
    assert_eq!(n, 0);
}
