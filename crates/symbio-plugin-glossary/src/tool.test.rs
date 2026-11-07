//! 工具入参的判据：`term` / `definition` 都必须是非空字符串且在声明上界内。

use super::extract_pair;
use serde_json::json;
use symbio_plugin_api::PluginError;

#[test]
fn a_valid_pair_is_accepted_and_trimmed() {
    let args = json!({ "term": "  债  ", "definition": "  尚未闭合的一笔义务  " });
    assert_eq!(extract_pair(&args).unwrap(), ("债", "尚未闭合的一笔义务"));
}

#[test]
fn a_missing_term_is_rejected() {
    let args = json!({ "definition": "尚未闭合的一笔义务" });
    assert!(matches!(
        extract_pair(&args),
        Err(PluginError::ValidationError(_))
    ));
}

#[test]
fn a_blank_definition_is_rejected() {
    let args = json!({ "term": "债", "definition": "   " });
    assert!(matches!(
        extract_pair(&args),
        Err(PluginError::ValidationError(_))
    ));
}

#[test]
fn a_non_string_term_is_rejected() {
    let args = json!({ "term": 42, "definition": "尚未闭合的一笔义务" });
    assert!(matches!(
        extract_pair(&args),
        Err(PluginError::ValidationError(_))
    ));
}

#[test]
fn an_overlong_definition_is_rejected() {
    let long = "字".repeat(super::MAX_DEFINITION_CHARS + 1);
    let args = json!({ "term": "债", "definition": long });
    assert!(matches!(
        extract_pair(&args),
        Err(PluginError::ValidationError(_))
    ));
}
