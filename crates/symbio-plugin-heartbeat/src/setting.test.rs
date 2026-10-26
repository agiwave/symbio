//! `setting` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）。
//!
//! 这里锁的是 `Session.metadata.heartbeat` 那块数据的**读侧回落**：会话存储只按键
//! 浅合并，不认识格子里有什么，所以「缺哪格算什么」只有本文件这一处定义。

use super::*;

/// 没有 `heartbeat` 键 ⇒ 默认（未启用）配置，而不是报错：清单里绝大多数会话
/// 从没配过心跳，那是一棵子树的常态。
#[test]
fn from_metadata_missing_returns_disabled_default() {
    let m = serde_json::json!({ "title": "x" });
    let hb = HeartbeatConfig::from_metadata(&m);
    assert!(!hb.enabled);
    assert_eq!(hb.interval_seconds, 300);
    assert!(hb.include_history);
    assert!(hb.prompt.is_empty());
}

#[test]
fn from_metadata_parses_full_config() {
    let m = serde_json::json!({
        "heartbeat": {
            "enabled": true,
            "interval_seconds": 120,
            "prompt": "请检查待办",
            "include_history": false
        }
    });
    let hb = HeartbeatConfig::from_metadata(&m);
    assert!(hb.enabled);
    assert_eq!(hb.interval_seconds, 120);
    assert_eq!(hb.prompt, "请检查待办");
    assert!(!hb.include_history);
}

/// 单格缺失各自回落默认，**不受其他格影响**：`include_history` 缺省是 `true`，
/// 所以「只写了 enabled」的会话照样携带历史。
#[test]
fn from_metadata_missing_fields_use_defaults() {
    let m = serde_json::json!({ "heartbeat": { "enabled": true } });
    let hb = HeartbeatConfig::from_metadata(&m);
    assert!(hb.enabled);
    assert_eq!(hb.interval_seconds, 300);
    assert!(hb.include_history);
}

/// `view()` 与 `from_metadata` 是**同一份四格**：设置工具的每次回执回这一份，
/// 调度侧下一次读的是它写进去的那份——写出去的读回来必须一模一样。
#[test]
fn view_round_trips_through_metadata() {
    let hb = HeartbeatConfig {
        enabled: true,
        interval_seconds: 45,
        prompt: "巡检".to_string(),
        include_history: false,
    };
    let m = serde_json::json!({ "heartbeat": hb.view() });
    let back = HeartbeatConfig::from_metadata(&m);
    assert_eq!(back.enabled, hb.enabled);
    assert_eq!(back.interval_seconds, hb.interval_seconds);
    assert_eq!(back.prompt, hb.prompt);
    assert_eq!(back.include_history, hb.include_history);
    assert_eq!(back.view(), hb.view(), "回执与落盘必须是同一份视图");
}

/// `view()` 的**键集合固定为四格**（顺序无关）：前端按这张表预填子表单，
/// 多一格或少一格都是契约变了。
#[test]
fn view_has_exactly_the_four_slots() {
    let keys: Vec<String> = {
        let mut k: Vec<String> = HeartbeatConfig::default()
            .view()
            .as_object()
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default();
        k.sort();
        k
    };
    assert_eq!(
        keys,
        vec![
            "enabled".to_string(),
            "include_history".to_string(),
            "interval_seconds".to_string(),
            "prompt".to_string()
        ]
    );
}
