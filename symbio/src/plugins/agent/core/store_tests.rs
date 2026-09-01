//! store.rs 单元测试
//!
//! 对应源文件: `store.rs`

use super::*;

#[test]
fn test_cosine_similarity_basic() {
    assert!((cosine_similarity(&[1.0, 0.0], &[1.0, 0.0]) - 1.0).abs() < 1e-6);
    assert!((cosine_similarity(&[1.0, 0.0], &[0.0, 1.0]) - 0.0).abs() < 1e-6);
}

#[test]
fn test_cosine_similarity_empty_returns_zero() {
    assert_eq!(cosine_similarity(&[], &[]), 0.0);
    assert_eq!(cosine_similarity(&[1.0], &[]), 0.0);
}

#[test]
fn test_cosine_similarity_length_mismatch() {
    assert_eq!(cosine_similarity(&[1.0, 0.0], &[1.0]), 0.0);
}

#[test]
fn test_page_request_helpers() {
    let p = PageRequest::first(10);
    assert_eq!(p.offset, 0);
    assert_eq!(p.limit, 10);

    let p = PageRequest::new(20, 5);
    assert_eq!(p.offset, 20);
    assert_eq!(p.limit, 5);
}

#[test]
fn test_filter_starts_with() {
    let f = FilterExpr::StartsWith {
        key: "id".to_string(),
        prefix: "test::".to_string(),
    };
    match f {
        FilterExpr::StartsWith { key, prefix } => {
            assert_eq!(key, "id");
            assert_eq!(prefix, "test::");
        }
        _ => panic!("expected StartsWith"),
    }
}

#[test]
fn test_filter_combinators() {
    let expr = FilterExpr::and(vec![
        FilterExpr::is_a("rule"),
        FilterExpr::eq("level", serde_json::json!("sys")),
    ]);
    match expr {
        // is_a 现在是 Relation { key: "is_a", value: "rule" }
        FilterExpr::And(items) => assert_eq!(items.len(), 2),
        _ => panic!("expected And"),
    }
}
