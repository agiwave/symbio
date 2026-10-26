//! `crates/symbio-plugin-web/src/net_guard.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 第三道闸（Agent 声明的出站白名单）的反向用例全在这里：**没有选中 Agent 时恒过、
//! 选中了而清单为空时恒拒**——这两条是该字段的语义本身，写反任何一条，
//! 「声明要成为安全边界」就不成立。

use super::*;
use serde_json::json;
use std::path::PathBuf;

fn scope(domains: &[&str]) -> AgentScope {
    AgentScope {
        id: "reviewer".to_string(),
        root: PathBuf::from("/nowhere/agent/reviewer"),
        // 经线格式构造（与 manifest 解析同一条路）：不深引 core 私有模块取子类型，
        // 也顺带钉住「声明就是 manifest 里那个形状」。
        permissions: serde_json::from_value(json!({ "network": { "domains": domains } }))
            .expect("声明可解析"),
    }
}

/// 没有选中 Agent ⇒ 宿主态，第三道闸恒过（没有 manifest 可声明）。
#[test]
fn no_agent_scope_means_host_mode() {
    assert!(agent_network_allowed("anything.example.org", None).is_ok());
}

/// 选中了而清单为空 ⇒ **一律拒**（grant 语义：空清单即没有授权）。
/// 这一条是该字段的语义本身——放行它，「不声明就能出网」就让整份声明失去意义。
#[test]
fn an_empty_domain_list_denies_every_host() {
    let s = scope(&[]);
    for host in ["example.com", "api.github.com", "127.0.0.1"] {
        let err = agent_network_allowed(host, Some(&s)).unwrap_err();
        assert!(
            err.to_string().contains("permissions.network.domains"),
            "拒绝理由须点名是哪个字段：{err}"
        );
    }
}

/// 清单内的主机放行；清单外的拒，且**拒绝发生在发请求之前**（本函数在建连前被调用）。
#[test]
fn only_declared_hosts_pass() {
    let s = scope(&["api.github.com", "*.example.com"]);
    assert!(agent_network_allowed("api.github.com", Some(&s)).is_ok());
    assert!(agent_network_allowed("cdn.example.com", Some(&s)).is_ok());
    // 标签边界：字符串后缀比较会把这两个也放行。
    assert!(agent_network_allowed("evil-api.github.com", Some(&s)).is_err());
    assert!(agent_network_allowed("notexample.com", Some(&s)).is_err());
}
