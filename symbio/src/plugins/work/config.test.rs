//! `work/config.rs` 的单元测试 —— 缺省值、向后兼容与闸门下界。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

#[test]
fn defaults_are_the_shipped_behaviour() {
    let c = WorkConfig::default();
    assert!(c.memory_enabled);
    assert_eq!(c.memory_max_bytes, 16 * 1024);
    assert_eq!(c.memory_inject_max_bytes, 4 * 1024);
}

/// 存量 / 手写的 `PLUGIN.yml` 缺字段时按缺省补齐（`#[serde(default)]` 的意义）
#[test]
fn missing_fields_fall_back_to_defaults() {
    let c: WorkConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(c.memory_enabled, WorkConfig::default().memory_enabled);
    assert_eq!(c.memory_max_bytes, WorkConfig::default().memory_max_bytes);

    // 只写一个字段：其余照旧
    let c: WorkConfig = serde_json::from_str(r#"{"memory_max_bytes": 512}"#).unwrap();
    assert_eq!(c.memory_max_bytes, 512);
    assert_eq!(c.memory_inject_max_bytes, 4 * 1024);
}

/// 配成 0 不能让一切写入都失败却看不出原因——下界兜到 1
#[test]
fn zero_limits_are_clamped_to_one() {
    let c = WorkConfig {
        memory_max_bytes: 0,
        memory_inject_max_bytes: 0,
        ..WorkConfig::default()
    };
    assert_eq!(c.effective_max_bytes(), 1);
    assert_eq!(c.effective_inject_bytes(), 1);
}

#[test]
fn roundtrip_is_lossless() {
    let c = WorkConfig {
        memory_enabled: false,
        memory_max_bytes: 7,
        memory_inject_max_bytes: 3,
    };
    let json = serde_json::to_string(&c).unwrap();
    let back: WorkConfig = serde_json::from_str(&json).unwrap();
    assert!(!back.memory_enabled);
    assert_eq!(back.memory_max_bytes, 7);
    assert_eq!(back.memory_inject_max_bytes, 3);
}
