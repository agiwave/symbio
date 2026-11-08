//! `tool.rs` 的单元测试 —— 入参的唯一校验点。
//!
//! 抽成纯函数的理由就在这里：判据不必构造 `ExecEnv` / 上下文，直接问「这套参数行不行」。

use super::{extract, WORLD_TOOL};
use serde_json::json;
use symbio_plugin_api::PluginError;

#[test]
fn the_tool_name_is_the_one_the_face_declares() {
    assert_eq!(WORLD_TOOL, "world_observe");
}

#[test]
fn a_complete_observation_passes() {
    let args = json!({
        "entity": " 服务器 ",
        "attribute": "状态",
        "value": "离线",
        "confidence": 80,
    });
    let (entity, attribute, value, confidence) = extract(&args).expect("完整观察应通过");
    assert_eq!(entity, "服务器", "两侧空白剥掉");
    assert_eq!(attribute, "状态");
    assert_eq!(value, "离线");
    assert_eq!(confidence, 80);
}

#[test]
fn confidence_is_optional() {
    let args = json!({ "entity": "a", "attribute": "b", "value": "c" });
    let (_, _, _, confidence) = extract(&args).expect("缺省置信度");
    assert_eq!(confidence, 50);
}

#[test]
fn confidence_above_hundred_is_clamped() {
    let args = json!({ "entity": "a", "attribute": "b", "value": "c", "confidence": 1000 });
    let (_, _, _, confidence) = extract(&args).expect("超界收窄");
    assert_eq!(confidence, 100);
}

#[test]
fn missing_fields_are_rejected() {
    for args in [
        json!({ "attribute": "b", "value": "c" }),
        json!({ "entity": "a", "value": "c" }),
        json!({ "entity": "a", "attribute": "b" }),
        json!({ "entity": 1, "attribute": "b", "value": "c" }),
        json!({ "entity": "a", "attribute": "b", "value": 3 }),
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
        json!({ "entity": "  ", "attribute": "b", "value": "c" }),
        json!({ "entity": "a", "attribute": "\t", "value": "c" }),
        json!({ "entity": "a", "attribute": "b", "value": "  " }),
    ] {
        assert!(
            matches!(extract(&args), Err(PluginError::ValidationError(_))),
            "空白字段应被拒：{args}"
        );
    }
}

/// 置信度不是整数时拒绝，不静默取缺省——「填错了」与「没填」是两件事。
#[test]
fn a_non_integer_confidence_is_rejected() {
    assert!(matches!(
        extract(&json!({
            "entity": "a",
            "attribute": "b",
            "value": "c",
            "confidence": "high",
        })),
        Err(PluginError::ValidationError(_))
    ));
}
