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
        assert_eq!(capability_to_wire(name), name, "{name} 不该被改写");
    }
}

#[test]
fn mcp_dotted_names_become_wire_safe() {
    // MCP 的名字带 `.`，OpenAI / Anthropic 的字符集不允许——这正是收口前
    // 漏掉的那个字符（旧代码只替换 `/`，`.` 原样送给模型）。
    let wire = capability_to_wire("mcp.filesystem.read_file");
    assert_eq!(wire, "mcp__filesystem__read_file");
    assert!(
        wire.chars().all(is_wire_char),
        "线上名必须落在 function-calling 字符集内"
    );
}

#[test]
fn any_illegal_char_maps_to_the_separator() {
    assert_eq!(capability_to_wire("a/b"), "a__b");
    assert_eq!(capability_to_wire("a:b"), "a__b");
    assert_eq!(capability_to_wire("a.b.c"), "a__b__c");
    // 连续非法字符各自成段（不做压缩）——保证「一个非法字符 → 一段分隔符」
    // 这条规则简单到可以预测
    assert_eq!(capability_to_wire("a/.b"), "a____b");
}

#[test]
fn to_wire_is_idempotent_on_wire_names() {
    // 线上名再走一次编码不该继续变化（否则「编码两次」会静默产生第三个名字）
    let wire = capability_to_wire("mcp.filesystem.read_file");
    assert_eq!(capability_to_wire(&wire), wire);
}

#[test]
fn wire_projection_is_not_injective_so_assembly_must_reject_collisions() {
    // `a.b` 与 `a__b` 都映成 `a__b`。旧形状靠「字面名优先」在返回时挑一个，
    // 那不是消歧；装配期必须让这两方都不暴露（判据在 session 的 exposed_tools
    // 反例单测与 tool 插件的 resolve 歧义兜底里），本函数不负责解决。
    assert_eq!(capability_to_wire("a.b"), capability_to_wire("a__b"));
    assert_eq!(capability_to_wire("a.b"), "a__b");
}

#[test]
fn real_name_shapes_have_stable_wire_forms() {
    // 出方向是唯一的一次换算：四类真实名字形状的别名必须稳定（存量会话兼容）。
    for (canonical, wire) in [
        ("vdfs_read", "vdfs_read"),
        ("shell", "shell"),
        ("mcp.filesystem.read_file", "mcp__filesystem__read_file"),
        ("agent_reviewer_read_skill", "agent_reviewer_read_skill"),
    ] {
        assert_eq!(capability_to_wire(canonical), wire, "{canonical} 投影漂移");
    }
}
