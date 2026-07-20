//! memory/save.rs 单元测试
//!
//! 对应源文件: `memory/save.rs`

use super::*;

// ── top_level_cu_field 单元测试 ──

#[test]
fn top_level_cu_field_detects_id() {
    let obj = json!({"id": "x", "items": []}).as_object().unwrap().clone();
    assert_eq!(top_level_cu_field(&obj), Some("id"));
}

#[test]
fn top_level_cu_field_detects_name() {
    let obj = json!({"name": "李四", "items": []})
        .as_object()
        .unwrap()
        .clone();
    assert_eq!(top_level_cu_field(&obj), Some("name"));
}

#[test]
fn top_level_cu_field_detects_is_a() {
    let obj = json!({"is_a": ["fact"], "items": []})
        .as_object()
        .unwrap()
        .clone();
    assert_eq!(top_level_cu_field(&obj), Some("is_a"));
}

#[test]
fn top_level_cu_field_clean_for_proper_call() {
    let obj = json!({"items": [{"id": "cu_1", "name": "X"}]})
        .as_object()
        .unwrap()
        .clone();
    assert_eq!(top_level_cu_field(&obj), None);
}

#[test]
fn top_level_cu_field_clean_for_empty_obj() {
    let obj = serde_json::Map::new();
    assert_eq!(top_level_cu_field(&obj), None);
}

#[test]
fn top_level_cu_field_ignores_non_cu_fields() {
    // 顶层可以有非 CU 字段（如未来的扩展参数），不应被误判
    let obj = json!({"items": [{"id": "cu_1"}], "extra_meta": "x"})
        .as_object()
        .unwrap()
        .clone();
    assert_eq!(top_level_cu_field(&obj), None);
}
