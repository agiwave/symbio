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

// ── 候选暴露集合（D-445 / D-468） ────────────────────────────────────────

use super::candidate_tools;
use std::collections::HashSet;

fn bound(names: &[&str]) -> HashSet<String> {
    names.iter().map(|n| n.to_string()).collect()
}

fn candidate_names(metas: Vec<CapabilityMeta>, bound: &HashSet<String>) -> Vec<String> {
    candidate_tools(metas, bound)
        .into_iter()
        .map(|m| m.name)
        .collect()
}

/// 孤儿描述行（有说明、无有效绑定）不得进候选：候选 = 显式暴露 ∩ 有效绑定。
///
/// 协议命名与碰撞裁决在模型插件的投影里（`tool_projection.test.rs`，D-469）；
/// 本用例只钉会话侧那一半：它不猜工具，也不替投影造别名。
#[test]
fn orphan_description_row_is_not_exposed() {
    let metas = vec![meta("vdfs_read"), meta("ghost_tool")];
    assert_eq!(
        candidate_names(metas, &bound(&["vdfs_read"])),
        ["vdfs_read"]
    );
    // 空绑定集 = 空候选：仍要能正常装配，不报错、不猜工具。
    assert!(candidate_names(vec![meta("vdfs_read")], &bound(&[])).is_empty());
}
