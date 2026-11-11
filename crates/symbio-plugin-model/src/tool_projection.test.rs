//! `tool_projection` 的判据：字符集换算 + 本轮暴露投影（碰撞整组退出）。
//!
//! 碰撞反例钉的是 `D-444` ②：两个身份映到同一 wire 时，**请求成形前**就让整组退出，
//! 并留下报因；不靠返回时「精确匹配优先」挑一个。

use super::*;

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|name| name.to_string()).collect()
}

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
fn wire_projection_is_not_injective_so_projection_must_reject_collisions() {
    // `a.b` 与 `a__b` 都映成 `a__b`。不靠「字面名优先」在返回时挑一个——
    // 碰撞整组退出的判据见下面的投影反例。
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

/// wire 碰撞（`a.b` 与 `a__b` 都映成 `a__b`）必须在**请求成形前**拒绝：
/// 冲突两方都不暴露，别名表里没有这个 wire，碰撞单独报因。
#[test]
fn collision_rejects_the_whole_group_and_reports_the_reason() {
    let projection = project(&names(&["a.b", "a__b", "shell"]));

    assert_eq!(projection.exposed, ["shell"]);
    assert!(
        !projection.aliases.contains_key("a.b"),
        "碰撞方不得进别名表"
    );
    assert!(
        !projection.aliases.contains_key("a__b"),
        "碰撞方不得进别名表"
    );
    assert_eq!(
        projection.aliases.get("shell").map(String::as_str),
        Some("shell")
    );
    assert_eq!(projection.collisions.len(), 1);
    assert_eq!(projection.collisions[0].wire, "a__b");
    assert_eq!(projection.collisions[0].names, ["a.b", "a__b"]);
}

/// 无碰撞时别名表与清单同源、且保持注册顺序（D-057）。
#[test]
fn aliases_come_from_the_same_exposure_set_in_registration_order() {
    let projection = project(&names(&["mcp.fs.read", "shell"]));

    assert_eq!(projection.exposed, ["mcp.fs.read", "shell"]);
    assert_eq!(
        projection.aliases.get("mcp.fs.read").map(String::as_str),
        Some("mcp__fs__read")
    );
    assert_eq!(
        projection.aliases.get("shell").map(String::as_str),
        Some("shell")
    );
    assert!(projection.collisions.is_empty());
}

/// 工具集变化后，**上一轮的 wire 名不得绑到新工具**（D-447 ③ 的第七类反例）。
///
/// 误绑的形状：别名表按名字缓存、或派发侧按前缀匹配，于是 `mcp.fs.read` 下线、
/// `mcp.fs.read_all` 上线之后，模型拿着上一轮那个 `mcp__fs__read` 仍能调到工具——
/// 而账、风险档、审批全按**旧名**算。正确行为：投影每轮按本轮候选重算，
/// 旧名不在表里，派发侧因此落失败事实（`plugin-tool` 的
/// `resolve_only_accepts_this_rounds_exposure_aliases` 钉那一半）。
#[test]
fn a_changed_tool_set_never_rebinds_last_rounds_wire_name() {
    let before = project(&names(&["mcp.fs.read", "mcp.fs.write"]));
    assert_eq!(
        before.aliases.get("mcp.fs.read").map(String::as_str),
        Some("mcp__fs__read")
    );

    // 上游 MCP 服务换了：`read` 下线、`read_all` 上线。
    let after = project(&names(&["mcp.fs.read_all", "mcp.fs.write"]));

    assert!(
        !after.aliases.contains_key("mcp.fs.read"),
        "下线的工具不得留在本轮别名表里：{:?}",
        after.aliases
    );
    assert_eq!(
        after.aliases.get("mcp.fs.read_all").map(String::as_str),
        Some("mcp__fs__read_all"),
        "新工具拿到的是自己的名字，不是旧的"
    );
    assert!(
        after.aliases.values().all(|wire| wire != "mcp__fs__read"),
        "上一轮的 wire 名不得被任何本轮身份占用：{:?}",
        after.aliases
    );
}

/// 空候选 = 空工具集：正常装配，不造别名或碰撞。
#[test]
fn empty_input_projects_to_an_empty_view() {
    let projection = project(&[]);
    assert!(projection.exposed.is_empty());
    assert!(projection.aliases.is_empty());
    assert!(projection.collisions.is_empty());
}
