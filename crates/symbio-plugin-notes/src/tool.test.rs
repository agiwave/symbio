//! 工具入参的判据：`text` 必须是非空字符串。

use super::extract_text;
use serde_json::json;
use symbio_plugin_api::PluginError;

#[test]
fn a_non_empty_text_is_accepted_and_trimmed() {
    assert_eq!(
        extract_text(&json!({ "text": "  买牛奶  " })).unwrap(),
        "买牛奶"
    );
}

#[test]
fn a_missing_text_is_rejected() {
    assert!(matches!(
        extract_text(&json!({})),
        Err(PluginError::ValidationError(_))
    ));
}

#[test]
fn a_blank_text_is_rejected() {
    assert!(matches!(
        extract_text(&json!({ "text": "   " })),
        Err(PluginError::ValidationError(_))
    ));
}

#[test]
fn a_non_string_text_is_rejected() {
    assert!(matches!(
        extract_text(&json!({ "text": 42 })),
        Err(PluginError::ValidationError(_))
    ));
}
