//! `conation/config.rs` 的单元测试 —— 出厂关、关掉可写、存量配置兼容。

use super::*;

#[test]
fn conation_enabled_defaults_to_off() {
    assert!(!ConationConfig::default().conation_enabled);
    assert!(!default_conation_enabled());
}

#[test]
fn missing_keys_fall_back_to_default() {
    let c: ConationConfig = serde_json::from_str("{}").unwrap();
    assert!(!c.conation_enabled);
}

#[test]
fn enabled_is_reachable_from_plugin_yml() {
    let c: ConationConfig = serde_json::from_str(r#"{"conation_enabled": true}"#).unwrap();
    assert!(c.conation_enabled);
}
