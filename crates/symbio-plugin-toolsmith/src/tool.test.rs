//! `tool.rs` 的单元测试 —— 入参的唯一校验点与工具名形状。

use super::{extract, valid_name, TOOLSMITH_TOOL};
use serde_json::json;
use symbio_plugin_api::PluginError;

#[test]
fn the_tool_name_is_the_one_the_face_declares() {
    assert_eq!(TOOLSMITH_TOOL, "toolsmith_define");
}

#[test]
fn a_complete_definition_passes() {
    let args = json!({
        "name": " summarize_ticket ",
        "description": " 把工单压成一句话 ",
        "input_schema": { "type": "object" },
    });
    let (name, description, schema) = extract(&args).expect("完整定义应通过");
    assert_eq!(name, "summarize_ticket", "两侧空白剥掉");
    assert_eq!(description, "把工单压成一句话");
    assert_eq!(schema["type"], "object");
}

/// 名字规则：小写字母开头、只含 `[a-z0-9_]`、3–40 字符。
#[test]
fn the_name_shape_is_enforced() {
    for good in ["a_b", "summarize_ticket", "tool_2"] {
        assert!(valid_name(good), "{good} 应合法");
    }
    for bad in ["", "ab", "_abc", "Tool", "tool-name", "tool name", "工具"] {
        assert!(!valid_name(bad), "{bad} 应不合法");
    }
    assert!(!valid_name(&"a".repeat(41)), "超过 40 字符应不合法");
}

#[test]
fn an_invalid_name_is_rejected() {
    let args = json!({
        "name": "Bad-Name",
        "description": "d",
        "input_schema": { "type": "object" },
    });
    assert!(matches!(
        extract(&args),
        Err(PluginError::ValidationError(_))
    ));
}

#[test]
fn missing_or_blank_fields_are_rejected() {
    for args in [
        json!({ "description": "d", "input_schema": {} }),
        json!({ "name": "good_name", "input_schema": {} }),
        json!({ "name": "good_name", "description": "d" }),
        json!({ "name": "good_name", "description": "  ", "input_schema": {} }),
        json!({ "name": 1, "description": "d", "input_schema": {} }),
    ] {
        assert!(
            matches!(extract(&args), Err(PluginError::ValidationError(_))),
            "缺字段或类型不对应被拒：{args}"
        );
    }
}

/// schema 必须是对象：字符串或数组不是可用的入参 schema。
#[test]
fn a_non_object_schema_is_rejected() {
    for schema in [json!("object"), json!([1]), json!(null)] {
        let args = json!({ "name": "good_name", "description": "d", "input_schema": schema });
        assert!(
            matches!(extract(&args), Err(PluginError::ValidationError(_))),
            "非对象 schema 应被拒：{schema}"
        );
    }
}

/// 入参里夹带 owner 不影响结果：这个字段根本不在 schema 里，也不被读取。
#[test]
fn an_owner_argument_is_ignored() {
    let args = json!({
        "name": "good_name",
        "description": "d",
        "input_schema": {},
        "owner": "vdfs",
    });
    let (name, _, _) = extract(&args).expect("多余字段应被忽略");
    assert_eq!(name, "good_name");
}
