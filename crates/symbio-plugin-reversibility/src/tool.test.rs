//! `tool.rs` 的单元测试 —— 入参的唯一校验点。
//!
//! 抽成纯函数的理由就在这里：判据不必构造 `ExecEnv` / 上下文，直接问「这套参数行不行」。

use super::{extract, REVERSIBILITY_TOOL};
use serde_json::json;
use symbio_plugin_api::PluginError;

#[test]
fn the_tool_name_is_the_one_the_face_declares() {
    assert_eq!(REVERSIBILITY_TOOL, "reversibility_annotate");
}

#[test]
fn a_reversible_action_passes() {
    let args = json!({ "action": " 删除 /tmp/build ", "level": "reversible" });
    let (action, level, undo) = extract(&args).expect("可逆动作应通过");
    assert_eq!(action, "删除 /tmp/build", "两侧空白剥掉");
    assert_eq!(level, "reversible");
    assert_eq!(undo, "", "没给补偿做法时为空串");
}

#[test]
fn a_compensable_action_needs_its_undo() {
    let missing = json!({ "action": "改配置", "level": "compensable" });
    assert!(
        matches!(extract(&missing), Err(PluginError::ValidationError(_))),
        "compensable 缺 undo 应被拒"
    );
    let blank = json!({ "action": "改配置", "level": "compensable", "undo": "  " });
    assert!(
        matches!(extract(&blank), Err(PluginError::ValidationError(_))),
        "compensable 的 undo 不能是空白"
    );
    let ok = json!({ "action": "改配置", "level": "compensable", "undo": "把旧配置写回" });
    assert_eq!(extract(&ok).expect("有补偿做法应通过").2, "把旧配置写回");
}

#[test]
fn an_unknown_level_is_rejected() {
    for level in ["maybe", "REVERSIBLE", ""] {
        let args = json!({ "action": "x", "level": level });
        assert!(
            matches!(extract(&args), Err(PluginError::ValidationError(_))),
            "未知等级应被拒：{level:?}"
        );
    }
}

#[test]
fn missing_or_blank_action_is_rejected() {
    for args in [
        json!({ "level": "reversible" }),
        json!({ "action": "  ", "level": "reversible" }),
        json!({ "action": 1, "level": "reversible" }),
        json!({ "action": "x" }),
    ] {
        assert!(
            matches!(extract(&args), Err(PluginError::ValidationError(_))),
            "动作缺失或类型不对应被拒：{args}"
        );
    }
}

#[test]
fn a_non_string_undo_is_rejected() {
    assert!(matches!(
        extract(&json!({ "action": "x", "level": "reversible", "undo": 1 })),
        Err(PluginError::ValidationError(_))
    ));
}
