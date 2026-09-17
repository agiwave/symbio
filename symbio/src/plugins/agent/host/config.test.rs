//! `agent/host/config.rs` 的单元测试 —— 缺省值、向后兼容与闸门下界。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

#[test]
fn defaults_are_the_shipped_behaviour() {
    let c = AgentConfig::default();
    assert_eq!(c.item_max_bytes, 32 * 1024);
    assert_eq!(c.memory_max_bytes, 16 * 1024);
    assert_eq!(c.memory_inject_max_bytes, 4 * 1024);
}

/// 存量 / 手写的 `PLUGIN.yml` 缺字段时按缺省补齐（`#[serde(default)]` 的意义）
#[test]
fn missing_fields_fall_back_to_defaults() {
    let d = AgentConfig::default();
    let c: AgentConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(c.item_max_bytes, d.item_max_bytes);
    assert_eq!(c.memory_max_bytes, d.memory_max_bytes);
    assert_eq!(c.memory_inject_max_bytes, d.memory_inject_max_bytes);

    let c: AgentConfig = serde_json::from_str(r#"{"item_max_bytes": 128}"#).unwrap();
    assert_eq!(c.item_max_bytes, 128);
}

#[test]
fn zero_limits_are_clamped_to_one() {
    let c = AgentConfig {
        item_max_bytes: 0,
        memory_max_bytes: 0,
        memory_inject_max_bytes: 0,
    };
    assert_eq!(c.effective_item_max_bytes(), 1);
    assert_eq!(c.effective_memory_max_bytes(), 1);
    assert_eq!(c.effective_memory_inject_bytes(), 1);
}

#[test]
fn roundtrip_is_lossless() {
    let c = AgentConfig {
        item_max_bytes: 11,
        memory_max_bytes: 7,
        memory_inject_max_bytes: 3,
    };
    let json = serde_json::to_string(&c).unwrap();
    let back: AgentConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(back.item_max_bytes, 11);
    assert_eq!(back.memory_max_bytes, 7);
    assert_eq!(back.memory_inject_max_bytes, 3);
}
