//! `dedup_subtree_twins` 的判据：前缀副本去掉、子树独有工具保留、判定与顺序无关。

use super::dedup_subtree_twins;
use symbio_kernel::capability::CapabilityMeta;

fn meta(name: &str) -> CapabilityMeta {
    CapabilityMeta {
        name: name.to_string(),
        ..Default::default()
    }
}

/// 前缀副本（含 `agent_` 双重前缀）被去掉；根工具保留；子树独有工具保留前缀。
#[test]
fn prefixed_twins_are_dropped_and_subtree_only_tools_are_kept() {
    let kept: Vec<String> = dedup_subtree_twins(vec![
        meta("vdfs_read"),
        meta("agent_run"),
        meta("agent_t59-agent_vdfs_read"),
        meta("agent_t59-agent_agent_run"),
        meta("agent_t59-agent_only_in_subtree"),
    ])
    .into_iter()
    .map(|m| m.name)
    .collect();
    assert_eq!(
        kept,
        ["vdfs_read", "agent_run", "agent_t59-agent_only_in_subtree"]
    );
}

/// 判定与顺序无关：同一集合换序，留下的名字集合相同。
#[test]
fn dedup_is_order_independent() {
    let a: Vec<String> = dedup_subtree_twins(vec![meta("vdfs_read"), meta("agent_x_vdfs_read")])
        .into_iter()
        .map(|m| m.name)
        .collect();
    let b: Vec<String> = dedup_subtree_twins(vec![meta("agent_x_vdfs_read"), meta("vdfs_read")])
        .into_iter()
        .map(|m| m.name)
        .collect();
    assert_eq!(a, ["vdfs_read"]);
    assert_eq!(b, ["vdfs_read"]);
}
