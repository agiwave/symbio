//! `task/config.rs` 的单元测试 —— 出厂容量、存量配置兼容、可覆盖。

use super::*;

#[test]
fn readyset_cap_defaults_to_the_shared_factory_value() {
    assert_eq!(TaskConfig::default().readyset_cap, default_readyset_cap());
    assert_eq!(default_readyset_cap(), symbio_plugin_api::TASK_CAP);
}

#[test]
fn missing_keys_fall_back_to_default() {
    let c: TaskConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(c.readyset_cap, default_readyset_cap());
}

#[test]
fn cap_is_reachable_from_plugin_yml() {
    let c: TaskConfig = serde_json::from_str(r#"{"readyset_cap": 3}"#).unwrap();
    assert_eq!(c.readyset_cap, 3);
}
