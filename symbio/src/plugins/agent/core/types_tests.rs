//! types.rs 单元测试
//!
//! 对应源文件: `types.rs`

use super::*;
use serde_json::json;

// ── OperationResult 单元测试 ──

#[test]
fn test_operation_result_success_constructor() {
    let r = OperationResult::success(json!({"key": "value"}));
    assert!(r.success);
    assert!(r.data.is_some());
    assert!(r.error.is_none());
    assert_eq!(r.data.unwrap(), json!({"key": "value"}));
}

#[test]
fn test_operation_result_error_constructor() {
    let r: OperationResult = OperationResult::error("something went wrong");
    assert!(!r.success);
    assert!(r.data.is_none());
    assert_eq!(r.error.unwrap(), "something went wrong");
}

// ── now_secs 单元测试 ──

#[test]
fn test_now_secs_returns_positive() {
    let now = now_secs();
    // 应该大于 2020-01-01
    assert!(now > 1577836800, "now_secs() 应返回合理的时间戳");
}

// ── cu_fields 常量测试 ──

#[test]
fn test_cu_fields_constants() {
    assert_eq!(cu_fields::ID, "id");
    assert_eq!(cu_fields::NAME, "name");
    assert_eq!(cu_fields::DESCRIPTION, "description");
    assert_eq!(cu_fields::CONTENT, "content");
    assert_eq!(cu_fields::CONFIDENCE, "confidence");
    assert_eq!(cu_fields::IS_A, "is_a");
    assert_eq!(cu_fields::PRIORITY, "priority");
    assert_eq!(cu_fields::EXPIRES_AT, "_ext_expires_at");
}

// ── generate_short_id 单元测试 ──

#[test]
fn test_generate_short_id_returns_8_chars() {
    let id = generate_short_id();
    assert_eq!(id.len(), 8, "短 id 应为 8 字符");
}

#[test]
fn test_generate_short_id_is_unique() {
    let a = generate_short_id();
    let b = generate_short_id();
    assert_ne!(a, b, "两次生成的 id 应不同");
}

// ── parse_cu_ref 单元测试 ──

#[test]
fn test_parse_cu_ref_with_name() {
    let r = parse_cu_ref("MyName::abc123");
    assert_eq!(r.id, "abc123");
    assert_eq!(r.name, Some("MyName".to_string()));
}

#[test]
fn test_parse_cu_ref_bare_id() {
    let r = parse_cu_ref("abc123");
    assert_eq!(r.id, "abc123");
    assert_eq!(r.name, None);
}

#[test]
fn test_parse_cu_ref_with_slash_rejected() {
    let r = parse_cu_ref("Name::path/to/id");
    // 包含 `/` 的 id 走 fallback，整个串当成 id
    assert_eq!(r.id, "Name::path/to/id");
    assert_eq!(r.name, None);
}

#[test]
fn test_parse_cu_ref_with_backslash_rejected() {
    let r = parse_cu_ref("Name::path\\to\\id");
    assert_eq!(r.id, "Name::path\\to\\id");
    assert_eq!(r.name, None);
}

#[test]
fn test_parse_cu_ref_empty_after_colon() {
    let r = parse_cu_ref("Name::");
    // 空 id 走 fallback
    assert_eq!(r.id, "Name::");
    assert_eq!(r.name, None);
}

#[test]
fn test_parse_cu_ref_leading_colon() {
    let r = parse_cu_ref("::abc");
    // 开头 `::` → colon_pos = 0，不满足 `colon_pos > 0`
    assert_eq!(r.id, "::abc");
    assert_eq!(r.name, None);
}

#[test]
fn test_parse_cu_owned_delegates_to_parse_cu_ref() {
    let r = parse_cu_owned("X::y");
    assert_eq!(r.id, "y");
    assert_eq!(r.name, Some("X".to_string()));
}

// ── new_cognitive_unit 单元测试 ──

#[test]
fn test_new_cognitive_unit_creates_unit_with_id() {
    let cu = new_cognitive_unit();
    assert!(!cu.id().is_empty());
    assert_eq!(cu.id().len(), 8);
}

// ── unit_with_id 单元测试 ──

#[test]
fn test_unit_with_id_preserves_existing_id() {
    let cu = CognitiveUnit::new("existing_id");
    let result = unit_with_id(&cu);
    assert_eq!(result.id(), "existing_id");
}

#[test]
fn test_unit_with_id_generates_id_when_empty() {
    let cu = CognitiveUnit::new("");
    let result = unit_with_id(&cu);
    assert!(!result.id().is_empty());
    assert_eq!(result.id().len(), 8);
}

// ── cu_from_json 单元测试 ──

#[test]
fn test_cu_from_json_valid_value() {
    let v = json!({
        "id": "test_cu",
        "is_a": ["fact"],
        "name": "Test",
        "description": "A test CU"
    });
    let cu = cu_from_json(v);
    assert_eq!(cu.id(), "test_cu");
    assert!(cu.is_type("fact"));
}

#[test]
fn test_cu_from_json_missing_id_uses_fallback() {
    // 没有 id 的 value 走 fallback 路径（不会 panic）
    let v = json!({
        "is_a": ["rule"],
        "description": "no id"
    });
    let cu = cu_from_json(v);
    assert!(!cu.id().is_empty());
    // 原始数据应被保留
    assert_eq!(cu.description(), Some("no id"));
}

// ── truncate_chars 单元测试 ──

#[test]
fn test_truncate_chars_short_text_unchanged() {
    let s = "hello";
    assert_eq!(truncate_chars(s, 10), "hello");
}

#[test]
fn test_truncate_chars_exact_length() {
    let s = "hello";
    assert_eq!(truncate_chars(s, 5), "hello");
}

#[test]
fn test_truncate_chars_truncates() {
    let s = "hello world";
    assert_eq!(truncate_chars(s, 5), "hello...");
}

#[test]
fn test_truncate_chars_handles_cjk() {
    // 5 个中文字符 → 截 3 个 + "..."
    let s = "你好世界你好";
    assert_eq!(truncate_chars(s, 3), "你好世...");
}

#[test]
fn test_truncate_chars_empty() {
    assert_eq!(truncate_chars("", 10), "");
    assert_eq!(truncate_chars("", 0), "");
}

#[test]
fn test_truncate_chars_zero_max() {
    let s = "hello";
    // max=0 → 截 0 字符 + "..." → "..."
    assert_eq!(truncate_chars(s, 0), "...");
}
