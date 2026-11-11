//! `dedup_declared_twins` 的判据：声明的副本去掉、未声明/本名缺席的保留、判定与顺序无关。
//!
//! 反向用例的关键在「不许猜」：一个长得像前缀副本的名字，只要没有声明，就必须留下。
//! 那条用例钉的是 D-428 的形状——前缀规则归子 Agent 插件，控制面只读声明。

use super::dedup_declared_twins;
use symbio_kernel::capability::CapabilityMeta;

fn meta(name: &str) -> CapabilityMeta {
    CapabilityMeta {
        name: name.to_string(),
        ..Default::default()
    }
}

fn pair(scoped: &str, base: &str) -> (String, String) {
    (scoped.to_string(), base.to_string())
}

fn kept_names(metas: Vec<CapabilityMeta>, twins: &[(String, String)]) -> Vec<String> {
    dedup_declared_twins(metas, twins)
        .into_iter()
        .map(|m| m.name)
        .collect()
}

/// 声明了的两份副本（含双重前缀）去掉；根工具保留；子树独有工具保留前缀。
#[test]
fn declared_twins_are_dropped_and_subtree_only_tools_are_kept() {
    let metas = vec![
        meta("vdfs_read"),
        meta("agent_run"),
        meta("agent_t59-agent_vdfs_read"),
        meta("agent_t59-agent_agent_run"),
        meta("agent_t59-agent_only_in_subtree"),
    ];
    let twins = vec![
        pair("agent_t59-agent_vdfs_read", "vdfs_read"),
        pair("agent_t59-agent_agent_run", "agent_run"),
        // 子树独有工具也声明本名，但本名不在表里 ⇒ 必须保留。
        pair("agent_t59-agent_only_in_subtree", "only_in_subtree"),
    ];
    assert_eq!(
        kept_names(metas, &twins),
        ["vdfs_read", "agent_run", "agent_t59-agent_only_in_subtree"]
    );
}

/// 反向用例：**没有声明就不许删**。名字长得再像前缀副本也不算副本——
/// 控制面不解析 `agent_` 或任何切点（D-428 之前正是这里按名字猜）。
#[test]
fn undeclared_names_are_never_inferred_as_twins() {
    let metas = vec![meta("vdfs_read"), meta("agent_x_vdfs_read")];
    assert_eq!(kept_names(metas, &[]), ["vdfs_read", "agent_x_vdfs_read"]);
}

/// 反向用例：声明存在但**本名缺席**（能力被卸载）⇒ 前缀那份仍要保留，
/// 否则卸载一个插件会静默丢掉一份能力（T3 的静默边）。
#[test]
fn declared_twin_with_absent_base_is_kept() {
    let metas = vec![meta("agent_x_vdfs_read")];
    assert_eq!(
        kept_names(metas, &[pair("agent_x_vdfs_read", "vdfs_read")]),
        ["agent_x_vdfs_read"]
    );
}

/// 声明指向一个不在本轮工具表里的前缀名 ⇒ 不影响任何一行。
#[test]
fn declaration_for_an_absent_scoped_name_is_a_noop() {
    let metas = vec![meta("vdfs_read"), meta("agent_x_vdfs_read")];
    assert_eq!(
        kept_names(metas, &[pair("agent_other_vdfs_read", "vdfs_read")]),
        ["vdfs_read", "agent_x_vdfs_read"]
    );
}

/// 判定与顺序无关：同一集合换序，留下的名字集合相同。
#[test]
fn dedup_is_order_independent() {
    let twins = [pair("agent_x_vdfs_read", "vdfs_read")];
    let a = kept_names(vec![meta("vdfs_read"), meta("agent_x_vdfs_read")], &twins);
    let b = kept_names(vec![meta("agent_x_vdfs_read"), meta("vdfs_read")], &twins);
    assert_eq!(a, ["vdfs_read"]);
    assert_eq!(b, ["vdfs_read"]);
}

// ── 暴露视图反例（D-444 / D-445） ────────────────────────────────────────

use super::exposed_tools;
use std::collections::HashSet;

fn bound(names: &[&str]) -> HashSet<String> {
    names.iter().map(|n| n.to_string()).collect()
}

fn exposed_names(metas: Vec<CapabilityMeta>, bound: &HashSet<String>) -> Vec<String> {
    exposed_tools(metas, bound)
        .0
        .into_iter()
        .map(|m| m.name)
        .collect()
}

/// 孤儿描述行（有说明、无有效绑定）不得进清单：清单 = 显式暴露 ∩ 有效绑定。
#[test]
fn orphan_description_row_is_not_exposed() {
    let metas = vec![meta("vdfs_read"), meta("ghost_tool")];
    assert_eq!(exposed_names(metas, &bound(&["vdfs_read"])), ["vdfs_read"]);
    // 空绑定集 = 空工具集：仍要能正常装配，不报错、不猜工具、不造别名或碰撞。
    let (empty, aliases, collisions) = exposed_tools(vec![meta("vdfs_read")], &bound(&[]));
    assert!(empty.is_empty());
    assert!(aliases.is_empty());
    assert!(collisions.is_empty());
}

/// wire 碰撞（`a.b` 与 `a__b` 都映成 `a__b`）必须在**请求成形前**拒绝：
/// 冲突两方都不暴露，别名表里没有这个 wire，碰撞单独报因（不靠返回时优先匹配）。
#[test]
fn wire_collision_is_rejected_before_the_request_is_shaped() {
    let metas = vec![meta("a.b"), meta("a__b"), meta("shell")];
    let (exposed, aliases, collisions) = exposed_tools(metas, &bound(&["a.b", "a__b", "shell"]));
    assert_eq!(
        exposed.into_iter().map(|m| m.name).collect::<Vec<_>>(),
        ["shell"]
    );
    assert!(!aliases.contains_key("a.b"), "碰撞方不得进别名表");
    assert!(!aliases.contains_key("a__b"), "碰撞方不得进别名表");
    assert_eq!(aliases.get("shell"), Some(&serde_json::json!("shell")));
    assert_eq!(collisions.len(), 1);
    assert_eq!(collisions[0]["wire"], "a__b");
    assert_eq!(collisions[0]["names"], serde_json::json!(["a.b", "a__b"]));
}

/// 无碰撞时别名表与清单同源、且保持注册顺序（D-057）。
#[test]
fn aliases_come_from_the_same_exposure_set_in_registration_order() {
    let metas = vec![meta("mcp.fs.read"), meta("shell")];
    let (exposed, aliases, collisions) = exposed_tools(metas, &bound(&["mcp.fs.read", "shell"]));
    assert_eq!(
        exposed.into_iter().map(|m| m.name).collect::<Vec<_>>(),
        ["mcp.fs.read", "shell"]
    );
    assert_eq!(
        aliases.get("mcp.fs.read"),
        Some(&serde_json::json!("mcp__fs__read"))
    );
    assert_eq!(aliases.get("shell"), Some(&serde_json::json!("shell")));
    assert!(collisions.is_empty());
}

/// MCP 工具集变化后，**上一轮的 wire 名不得绑到新工具**（D-447 ③ 的第七类反例）。
///
/// 误绑的形状：别名表按名字缓存、或派发侧按前缀匹配，于是 `mcp.fs.read` 下线、
/// `mcp.fs.read_all` 上线之后，模型拿着上一轮那个 `mcp__fs__read` 仍能调到工具——
/// 而账、风险档、审批全按**旧名**算。正确行为：别名表每轮按本轮暴露集合重算，
/// 旧名不在表里，派发侧因此落失败事实（`plugin-tool` 的
/// `resolve_only_accepts_this_rounds_exposure_aliases` 钉那一半）。
#[test]
fn a_changed_mcp_tool_set_never_rebinds_last_rounds_wire_name() {
    let before = vec![meta("mcp.fs.read"), meta("mcp.fs.write")];
    let (_, aliases_before, _) = exposed_tools(before, &bound(&["mcp.fs.read", "mcp.fs.write"]));
    assert_eq!(
        aliases_before.get("mcp.fs.read"),
        Some(&serde_json::json!("mcp__fs__read"))
    );

    // 上游 MCP 服务换了：`read` 下线、`read_all` 上线。
    let after = vec![meta("mcp.fs.read_all"), meta("mcp.fs.write")];
    let (_, aliases_after, _) = exposed_tools(after, &bound(&["mcp.fs.read_all", "mcp.fs.write"]));

    assert!(
        !aliases_after.contains_key("mcp.fs.read"),
        "下线的工具不得留在本轮别名表里：{aliases_after:?}"
    );
    assert_eq!(
        aliases_after.get("mcp.fs.read_all"),
        Some(&serde_json::json!("mcp__fs__read_all")),
        "新工具拿到的是自己的名字，不是旧的"
    );
    assert!(
        aliases_after
            .values()
            .all(|wire| wire != &serde_json::json!("mcp__fs__read")),
        "上一轮的 wire 名不得被任何本轮身份占用：{aliases_after:?}"
    );
}
