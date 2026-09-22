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

// ============================================================================
// MCP 报文字段名映射（camelCase ↔ snake_case）
// ============================================================================
//
// 这组用例钉的是**协议字段名**。漏掉映射不会编译失败、不会 panic，只会让
// 反序列化在运行期返回 Err——而那个 Err 经 `tracing::warn!` 输出，CLI 看不见。
// 曾经的全部后果：`initialize` 解析失败 ⇒ `tools/list` 根本不发出 ⇒
// 配了 MCP server 却一个工具都注册不上。
//
// 因此这里的输入**必须逐字取自 MCP 2025-06-18 规范**（camelCase），
// 不得为了方便改成 snake_case——那正是让这个 bug 长期潜伏的原因。

/// 规范形状的 `initialize` 响应（原文照抄规范示例）
const INITIALIZE_CAMEL: &str = r#"{
  "protocolVersion": "2025-06-18",
  "capabilities": { "tools": { "listChanged": true } },
  "serverInfo": { "name": "mock", "version": "1.0.0" },
  "instructions": "用 echo 工具回显"
}"#;

#[test]
fn initialize_response_parses_the_spec_camel_case_shape() {
    let r: McpInitializeResponse = serde_json::from_str(INITIALIZE_CAMEL).unwrap();
    assert_eq!(r.protocol_version, "2025-06-18");
    assert_eq!(r.server_info.name, "mock");
    assert_eq!(r.server_info.version.as_deref(), Some("1.0.0"));
    assert_eq!(
        r.capabilities.tools.as_ref().map(|t| t.list_changed),
        Some(true)
    );
    assert_eq!(r.instructions.as_deref(), Some("用 echo 工具回显"));
}

#[test]
fn initialize_response_requires_protocol_version_in_camel_case() {
    // 反面：只有 snake_case 时**必须**失败——证明映射真的在起作用，
    // 而不是「两种写法恰好都能过」
    let snake = r#"{"protocol_version":"2025-06-18","server_info":{"name":"mock"}}"#;
    assert!(serde_json::from_str::<McpInitializeResponse>(snake).is_err());
}

/// 规范形状的 `tools/list` 响应
const TOOLS_LIST_CAMEL: &str = r#"{
  "tools": [
    {
      "name": "echo",
      "description": "回显",
      "inputSchema": { "type": "object", "properties": { "text": { "type": "string" } } },
      "annotations": { "readOnlyHint": true, "destructiveHint": false, "title": "Echo" }
    },
    { "name": "bare", "inputSchema": { "type": "object" } }
  ],
  "nextCursor": "page2"
}"#;

#[test]
fn tools_list_parses_the_spec_camel_case_shape() {
    let r: ListToolsResult = serde_json::from_str(TOOLS_LIST_CAMEL).unwrap();
    assert_eq!(r.tools.len(), 2);
    assert_eq!(r.tools[0].name, "echo");
    assert_eq!(r.tools[0].description, "回显");
    assert_eq!(r.tools[0].input_schema["type"], "object");
    // 分页游标也是 camelCase——漏了它翻页永远停在第一页（且不报错）
    assert_eq!(r.next_cursor.as_deref(), Some("page2"));
}

#[test]
fn tool_description_is_optional() {
    // 规范里 description 可选；缺它不得把整份 tools/list 拖垮
    let r: ListToolsResult = serde_json::from_str(TOOLS_LIST_CAMEL).unwrap();
    assert_eq!(r.tools[1].name, "bare");
    assert_eq!(r.tools[1].description, "");
}

#[test]
fn tool_annotations_use_camel_case() {
    let r: ListToolsResult = serde_json::from_str(TOOLS_LIST_CAMEL).unwrap();
    let a = r.tools[0]
        .annotations
        .as_ref()
        .expect("annotations 应被解析");
    assert!(a.read_only_hint);
    assert!(!a.destructive_hint);
    assert_eq!(a.title.as_deref(), Some("Echo"));
}

#[test]
fn tool_call_response_is_error_uses_camel_case() {
    let r: McpToolCallResponse =
        serde_json::from_str(r#"{"content":[{"type":"text","text":"x"}],"isError":true}"#).unwrap();
    assert_eq!(r.is_error, Some(true));
    assert_eq!(r.text(), "x");
}

// ---- 以下是 `tools/call` 的**载荷**语义 -------------------------------------
//
// 曾经这里按 JSON-RPC **信封**（`{result, error, isError}`）建模，却拿
// `CallToolResult` **载荷**（`{content, isError}`）去反序列化：三个字段全可选，
// 于是"解析成功"、内容是 `Value::Null`、正文永远渲染成字符串 `"null"`，且全程
// 不报错。下面这组用例把"读的是载荷"这件事钉死。

#[test]
fn call_tool_result_reads_the_payload_not_the_envelope() {
    // 规范 `CallToolResult` 载荷（逐字取自 MCP 规范示例形状）
    let r: McpToolCallResponse =
        serde_json::from_str(r#"{"content":[{"type":"text","text":"mcp-echo:hello"}]}"#).unwrap();
    assert_eq!(r.text(), "mcp-echo:hello");
    assert_eq!(r.is_error, None);
}

#[test]
fn call_tool_result_text_joins_multiple_blocks() {
    let r: McpToolCallResponse = serde_json::from_str(
        r#"{"content":[{"type":"text","text":"a"},{"type":"text","text":"b"}]}"#,
    )
    .unwrap();
    assert_eq!(r.text(), "a\nb");
}

#[test]
fn call_tool_result_text_keeps_non_text_blocks_as_json() {
    // 非 text 块（image / resource / audio）不丢——原样保留 JSON 交给上层判断
    let r: McpToolCallResponse = serde_json::from_str(
        r#"{"content":[{"type":"image","data":"AA==","mimeType":"image/png"}]}"#,
    )
    .unwrap();
    assert!(r.text().contains("\"image\""));
    assert!(r.text().contains("image/png"));
}

#[test]
fn call_tool_result_falls_back_to_structured_content() {
    // content 为空时退到 structuredContent，而不是交白卷
    let r: McpToolCallResponse =
        serde_json::from_str(r#"{"content":[],"structuredContent":{"total":3}}"#).unwrap();
    assert_eq!(r.text(), r#"{"total":3}"#);
}

#[test]
fn call_tool_result_empty_is_empty_not_null() {
    // 空结果渲染成 ""，绝不能是 "null"
    let r: McpToolCallResponse = serde_json::from_str("{}").unwrap();
    assert_eq!(r.text(), "");
    assert!(!r.text().contains("null"));
}
