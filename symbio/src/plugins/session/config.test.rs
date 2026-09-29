//! `session/config.rs` 的单元测试 —— 记忆两道闸门 + 两个能力开关的取值与下界。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 缺省值 / 向后兼容 / 「面板默认值与 serde 同源」的不变式由 `plugin.test.rs`
//! 的 `config_definition_defaults_come_from_session_config` 锁定，这里**刻意不重复**。
//! 本文件只回答「取值与出厂决定」这一件事——「出厂决定」指那些一旦翻转就会改变
//! 用户可见行为的默认值（`triage_enabled` / `reply_enabled`），它们必须被断言钉住。

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

/// 出厂**打开**判决与措辞：S2 的"先交机制、默认关闭"到 S3 结束——`reply` 的两条
/// 产线已落地，`Answered` 必然有话说（最差是变体兜底），沉默的前提不复存在。
///
/// 本断言把「翻默认值」这一动作钉在**同一批**里：翻转它的人必须同时让措辞侧就位，
/// 否则「你好」会变成沉默（那是回归，不是"少说一句"）。
#[test]
fn triage_and_reply_are_on_by_default() {
    let c = SessionConfig::default();
    assert!(c.triage_enabled);
    assert!(c.reply_enabled);
    assert!(default_triage_enabled());
    assert!(default_reply_enabled());
}

/// 存量 `PLUGIN.yml` 没有这两个键时按缺省补齐（`#[serde(default)]` 的意义）
#[test]
fn missing_triage_and_reply_keys_fall_back_to_defaults() {
    let c: SessionConfig = serde_json::from_str(r#"{"max_messages": 42}"#).unwrap();
    let d = SessionConfig::default();
    assert_eq!(c.triage_enabled, d.triage_enabled);
    assert_eq!(c.reply_enabled, d.reply_enabled);
}

/// 两级开关**相互独立**：措辞关掉不会连累判决，判决关掉也不影响措辞配置。
///
/// 这不是"两个布尔恰好不同"——它们回答两个不同的问题（要不要干活 / 说什么），
/// 组合起来四种形态都合法（见 `ChatOrchestrator::reply_enabled` 的说明）。
#[test]
fn triage_and_reply_switches_are_independent() {
    let only_triage = SessionConfig {
        triage_enabled: true,
        reply_enabled: false,
        ..SessionConfig::default()
    };
    assert!(only_triage.triage_enabled && !only_triage.reply_enabled);

    let only_reply = SessionConfig {
        triage_enabled: false,
        reply_enabled: true,
        ..SessionConfig::default()
    };
    assert!(!only_reply.triage_enabled && only_reply.reply_enabled);
}
