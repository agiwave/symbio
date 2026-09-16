//! `session/config.rs` 的单元测试 —— 记忆两道闸门的取值与下界。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 缺省值 / 向后兼容 / 「面板默认值与 serde 同源」的不变式由 `plugin.test.rs`
//! 的 `config_definition_defaults_come_from_session_config` 锁定，这里**刻意不重复**。
//! 本文件只回答「闸门取值」这一件事。

use super::*;

/// 缺省即出厂行为：写入 16 KiB、注入 4 KiB（与 work 插件同口径）
#[test]
fn memory_gate_defaults_are_the_shipped_behaviour() {
    let c = SessionConfig::default();
    assert_eq!(c.memory_max_bytes, 16 * 1024);
    assert_eq!(c.memory_inject_max_bytes, 4 * 1024);
}

/// 两道闸门**独立**：写入上限通常远大于注入预算，改一个不该动另一个
#[test]
fn the_two_gates_are_independent() {
    let c = SessionConfig {
        memory_max_bytes: 4096,
        memory_inject_max_bytes: 512,
        ..SessionConfig::default()
    };
    assert_eq!(c.effective_memory_max_bytes(), 4096);
    assert_eq!(c.effective_memory_inject_bytes(), 512);
}

/// 配成 0 不能让一切写入都失败却看不出原因——下界兜到 1
#[test]
fn zero_limits_are_clamped_to_one() {
    let c = SessionConfig {
        memory_max_bytes: 0,
        memory_inject_max_bytes: 0,
        ..SessionConfig::default()
    };
    assert_eq!(c.effective_memory_max_bytes(), 1);
    assert_eq!(c.effective_memory_inject_bytes(), 1);
}

/// 存量 `PLUGIN.yml` 缺这两个键时按缺省补齐（`#[serde(default)]` 的意义）
#[test]
fn missing_memory_keys_fall_back_to_defaults() {
    let c: SessionConfig = serde_json::from_str(r#"{"max_messages": 42}"#).unwrap();
    assert_eq!(c.max_messages, 42);
    assert_eq!(
        c.memory_max_bytes,
        SessionConfig::default().memory_max_bytes
    );
    assert_eq!(
        c.memory_inject_max_bytes,
        SessionConfig::default().memory_inject_max_bytes
    );
}
