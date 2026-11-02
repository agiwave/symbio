//! `tool.rs` 的单元测试 —— 入参的唯一校验点。

use super::{extract, IDEMPOTENCY_TOOL};
use serde_json::json;
use symbio_plugin_api::PluginError;

#[test]
fn the_tool_name_is_the_one_the_face_declares() {
    assert_eq!(IDEMPOTENCY_TOOL, "idempotent_record");
}

#[test]
fn a_complete_record_passes() {
    let args = json!({ "key": " build-42 ", "value": " compiled " });
    let (key, value) = extract(&args).expect("完整记录应通过");
    assert_eq!(key, "build-42", "两侧空白剥掉");
    assert_eq!(value, "compiled");
}

#[test]
fn missing_fields_are_rejected() {
    for args in [
        json!({ "value": "v" }),
        json!({ "key": "k" }),
        json!({ "key": 1, "value": "v" }),
        json!({ "key": "k", "value": 2 }),
    ] {
        assert!(
            matches!(extract(&args), Err(PluginError::ValidationError(_))),
            "缺字段或类型不对应被拒：{args}"
        );
    }
}

#[test]
fn blank_fields_are_rejected() {
    for args in [
        json!({ "key": "  ", "value": "v" }),
        json!({ "key": "k", "value": "\t" }),
    ] {
        assert!(
            matches!(extract(&args), Err(PluginError::ValidationError(_))),
            "空白字段应被拒：{args}"
        );
    }
}
