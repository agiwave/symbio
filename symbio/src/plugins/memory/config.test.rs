//! `config.rs` 的单元测试 —— 缺省值、向后兼容与闸门下界。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

#[test]
fn defaults_are_the_shipped_behaviour() {
    let c = MemoryConfig::default();
    // 智能体作用域
    assert_eq!(c.memory_max_bytes, 16 * 1024);
    assert_eq!(c.memory_inject_max_bytes, 4 * 1024);
    // 工作区作用域（开关默认开——原 work 插件的出厂行为）
    assert!(c.workspace_enabled);
    assert_eq!(c.workspace_max_bytes, 16 * 1024);
    assert_eq!(c.workspace_inject_max_bytes, 4 * 1024);
}

/// 存量 / 手写的 `PLUGIN.yml` 缺字段时按缺省补齐（`#[serde(default)]` 的意义）。
///
/// 两个作用域**互不补位**：只写智能体的闸门，工作区照走缺省，反之亦然。
#[test]
fn missing_fields_fall_back_to_defaults() {
    let c: MemoryConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(c.memory_max_bytes, MemoryConfig::default().memory_max_bytes);
    assert_eq!(
        c.workspace_max_bytes,
        MemoryConfig::default().workspace_max_bytes
    );
    assert!(c.workspace_enabled);

    // 只写一个字段：其余照旧
    let c: MemoryConfig = serde_json::from_str(r#"{"memory_max_bytes": 512}"#).unwrap();
    assert_eq!(c.memory_max_bytes, 512);
    assert_eq!(c.memory_inject_max_bytes, 4 * 1024);
    assert_eq!(c.workspace_max_bytes, 16 * 1024);

    let c: MemoryConfig =
        serde_json::from_str(r#"{"workspace_max_bytes": 256, "workspace_enabled": false}"#)
            .unwrap();
    assert_eq!(c.workspace_max_bytes, 256);
    assert!(!c.workspace_enabled);
    assert_eq!(
        c.memory_max_bytes,
        16 * 1024,
        "工作区字段不波及智能体作用域"
    );
}

/// 配成 0 不能让一切写入都失败却看不出原因——下界兜到 1（两个作用域同规）
#[test]
fn zero_limits_are_clamped_to_one() {
    let c = MemoryConfig {
        memory_max_bytes: 0,
        memory_inject_max_bytes: 0,
        workspace_enabled: true,
        workspace_max_bytes: 0,
        workspace_inject_max_bytes: 0,
    };
    assert_eq!(c.effective_memory_max_bytes(), 1);
    assert_eq!(c.effective_memory_inject_bytes(), 1);
    assert_eq!(c.effective_workspace_max_bytes(), 1);
    assert_eq!(c.effective_workspace_inject_bytes(), 1);
}

#[test]
fn roundtrip_is_lossless() {
    let c = MemoryConfig {
        memory_max_bytes: 7,
        memory_inject_max_bytes: 3,
        workspace_enabled: false,
        workspace_max_bytes: 11,
        workspace_inject_max_bytes: 5,
    };
    let json = serde_json::to_string(&c).unwrap();
    let back: MemoryConfig = serde_json::from_str(&json).unwrap();
    assert_eq!(back.memory_max_bytes, 7);
    assert_eq!(back.memory_inject_max_bytes, 3);
    assert!(!back.workspace_enabled);
    assert_eq!(back.workspace_max_bytes, 11);
    assert_eq!(back.workspace_inject_max_bytes, 5);
}
