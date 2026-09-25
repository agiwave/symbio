//! `symbio/src/symbio_core/capability.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

/// serde 往返：声明字段序列化进 tools/list JSON；未声明字段不出现且旧 JSON 兼容
#[test]
fn context_retention_serde_roundtrip() {
    let meta = CapabilityMeta {
        name: "todo_write".into(),
        description: "d".into(),
        input_schema: serde_json::json!({"type": "object"}),
        context_retention: Some(CapabilityToolContextRetention::LastOnly),
        ..Default::default()
    };
    let v = serde_json::to_value(&meta).unwrap();
    assert_eq!(v["context_retention"], serde_json::json!("last_only"));
    let back: CapabilityMeta = serde_json::from_value(v).unwrap();
    assert_eq!(
        back.context_retention,
        Some(CapabilityToolContextRetention::LastOnly)
    );

    // 旧 JSON（无该字段）反序列化兼容，且序列化时不输出空字段
    let old: CapabilityMeta = serde_json::from_value(serde_json::json!({
        "name": "x", "description": "d", "parameters": {}
    }))
    .unwrap();
    assert_eq!(old.context_retention, None);
    let ser = serde_json::to_value(&old).unwrap();
    assert!(ser.get("context_retention").is_none());
}

/// keep_count：All → u32::MAX，LastN(n) → n.max(1)，LastOnly → 1
#[test]
fn keep_count_semantics() {
    assert_eq!(CapabilityToolContextRetention::All.keep_count(), u32::MAX);
    assert_eq!(CapabilityToolContextRetention::LastN(0).keep_count(), 1);
    assert_eq!(CapabilityToolContextRetention::LastN(3).keep_count(), 3);
    assert_eq!(CapabilityToolContextRetention::LastOnly.keep_count(), 1);
}
