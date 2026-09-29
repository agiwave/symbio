//! `triage/config.rs` 的单元测试 —— 平凡值与存量配置的兼容。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

/// 缺省即出厂行为：规则短路**开**
#[test]
fn rule_shortcut_defaults_to_on() {
    assert!(TriageConfig::default().rule_shortcut);
    assert_eq!(
        TriageConfig::default().rule_shortcut,
        default_rule_shortcut()
    );
}

/// 存量 `PLUGIN.yml`（装配期刚补出身份键、还没有本插件业务键）必须能读成缺省
#[test]
fn missing_business_keys_fall_back_to_defaults() {
    let c: TriageConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(c.rule_shortcut, default_rule_shortcut());
}

/// 平凡值可达：`rule_shortcut: false` 是一条能被写进配置的取值
#[test]
fn plain_value_is_reachable_from_config() {
    let c: TriageConfig = serde_json::from_str(r#"{"rule_shortcut": false}"#).unwrap();
    assert!(!c.rule_shortcut);
}
