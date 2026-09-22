//! `tool_name` 模块的单元测试。
//!
//! `tool_name.rs` 只保留生产代码，测试全部放本文件。

use super::*;

#[test]
fn safe_names_pass_through_unchanged() {
    // 今天全部能力名都在协议字符集内 → 线上形态 = 原名。
    // 这条是**存量兼容**的根据：名字没变，落库的 ToolCall.name 也就没变。
    for name in [
        "vdfs_read",
        "shell",
        "ask_user",
        "todo_write",
        "codebase_search",
        "content_search",
        "web_search",
        "web_fetch",
        "http_request",
        "heartbeat",
        "agent_run",
        "agent_com_acme_code-reviewer_read_skill",
    ] {
        assert_eq!(to_wire(name), name, "{name} 不该被改写");
    }
}

#[test]
fn mcp_dotted_names_become_wire_safe() {
    // MCP 的名字带 `.`，OpenAI / Anthropic 的字符集不允许——这正是收口前
    // 漏掉的那个字符（旧代码只替换 `/`，`.` 原样送给模型）。
    let wire = to_wire("mcp.filesystem.read_file");
    assert_eq!(wire, "mcp__filesystem__read_file");
    assert!(
        wire.chars().all(is_wire_char),
        "线上名必须落在 function-calling 字符集内"
    );
}

#[test]
fn any_illegal_char_maps_to_the_separator() {
    assert_eq!(to_wire("a/b"), "a__b");
    assert_eq!(to_wire("a:b"), "a__b");
    assert_eq!(to_wire("a.b.c"), "a__b__c");
    // 连续非法字符各自成段（不做压缩）——保证「一个非法字符 → 一段分隔符」
    // 这条规则简单到可以预测
    assert_eq!(to_wire("a/.b"), "a____b");
}

#[test]
fn to_wire_is_idempotent_on_wire_names() {
    // 线上名再走一次编码不该继续变化（否则「编码两次」会静默产生第三个名字）
    let wire = to_wire("mcp.filesystem.read_file");
    assert_eq!(to_wire(&wire), wire);
}

#[test]
fn resolve_prefers_the_literal_name() {
    // `a__b` 既可能是 `a.b` 的线上形态，也可能本身就是 `a__b`。
    // 字面名赢——歧义有确定答案，而不是「看注册顺序」。
    let known = ["a.b", "a__b"];
    assert_eq!(resolve("a__b", known), Some("a__b"));

    // 反过来，只有投影候选时也能命中
    let known = ["a.b"];
    assert_eq!(resolve("a__b", known), Some("a.b"));
}

#[test]
fn resolve_is_order_independent_for_literal_matches() {
    // 字面名排在投影候选**之后**也要赢（不是「先到先得」）
    let known = ["a.b", "c", "a__b"];
    assert_eq!(resolve("a__b", known), Some("a__b"));
}

#[test]
fn resolve_returns_none_for_unknown_names() {
    // 解析失败必须是 None——调用方据此报错，而不是回退到字符串反演
    assert_eq!(resolve("nope", ["shell", "vdfs_read"]), None);
    assert_eq!(resolve("", ["shell"]), None);
}

#[test]
fn resolve_round_trips_every_real_name_shape() {
    // 出方向 → 入方向往返：四类真实名字形状都要能回到原名
    let known = [
        "vdfs_read",
        "shell",
        "mcp.filesystem.read_file",
        "agent_reviewer_read_skill",
    ];
    for name in known {
        let wire = to_wire(name);
        assert_eq!(resolve(&wire, known), Some(name), "{name} 往返失败");
    }
}
