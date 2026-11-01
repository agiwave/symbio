//! `tool.rs` 的单元测试 —— 入参的唯一校验点。
//!
//! 抽成纯函数的理由就在这里：判据不必构造 `ExecEnv` / 上下文，直接问「这套参数行不行」。

use super::{extract, CAUSAL_TOOL};
use serde_json::json;
use symbio_plugin_api::PluginError;

#[test]
fn the_tool_name_is_the_one_the_face_declares() {
    assert_eq!(CAUSAL_TOOL, "causal_assert");
}

#[test]
fn a_complete_triple_passes() {
    let args = json!({ "cause": " 内存不足 ", "effect": "任务超时", "confidence": 70 });
    let (cause, effect, confidence) = extract(&args).expect("完整三元组应通过");
    assert_eq!(cause, "内存不足", "两侧空白剥掉");
    assert_eq!(effect, "任务超时");
    assert_eq!(confidence, 70);
}

#[test]
fn confidence_is_optional() {
    let args = json!({ "cause": "a", "effect": "b" });
    let (_, _, confidence) = extract(&args).expect("缺省置信度");
    assert_eq!(confidence, 50);
}

#[test]
fn confidence_above_hundred_is_clamped() {
    let args = json!({ "cause": "a", "effect": "b", "confidence": 1000 });
    let (_, _, confidence) = extract(&args).expect("超界收窄");
    assert_eq!(confidence, 100);
}

#[test]
fn missing_sides_are_rejected() {
    for args in [
        json!({ "effect": "b" }),
        json!({ "cause": "a" }),
        json!({ "cause": 1, "effect": "b" }),
    ] {
        assert!(
            matches!(extract(&args), Err(PluginError::ValidationError(_))),
            "缺侧或类型不对应被拒：{args}"
        );
    }
}

#[test]
fn blank_sides_are_rejected() {
    for args in [
        json!({ "cause": "  ", "effect": "b" }),
        json!({ "cause": "a", "effect": "\t" }),
    ] {
        assert!(
            matches!(extract(&args), Err(PluginError::ValidationError(_))),
            "空白侧应被拒：{args}"
        );
    }
}

/// 自环直接拒：它在图里没有信息量，接受它只会让每个读方多一条要滤的边。
#[test]
fn a_self_loop_is_rejected() {
    assert!(matches!(
        extract(&json!({ "cause": "a", "effect": " a " })),
        Err(PluginError::ValidationError(_))
    ));
}

/// 置信度不是整数时拒绝，不静默取缺省——「填错了」与「没填」是两件事。
#[test]
fn a_non_integer_confidence_is_rejected() {
    assert!(matches!(
        extract(&json!({ "cause": "a", "effect": "b", "confidence": "high" })),
        Err(PluginError::ValidationError(_))
    ));
}
