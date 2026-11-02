//! `tool.rs` 的单元测试 —— 入参的唯一校验点。
//!
//! 抽成纯函数的理由就在这里：判据不必构造 `ExecEnv` / 上下文，直接问「这套参数行不行」。

use super::{extract, EXPLAIN_TOOL};
use serde_json::json;
use symbio_plugin_api::PluginError;

#[test]
fn the_tool_name_is_the_one_the_face_declares() {
    assert_eq!(EXPLAIN_TOOL, "explain_record");
}

#[test]
fn a_complete_record_passes() {
    let args = json!({ "decision": " 切换到增量编译 ", "why": "全量编译超过预算", "evidence": "构建日志" });
    let (decision, why, evidence) = extract(&args).expect("完整记录应通过");
    assert_eq!(decision, "切换到增量编译", "两侧空白剥掉");
    assert_eq!(why, "全量编译超过预算");
    assert_eq!(evidence, "构建日志");
}

#[test]
fn evidence_is_optional() {
    let args = json!({ "decision": "a", "why": "b" });
    let (_, _, evidence) = extract(&args).expect("缺省证据");
    assert_eq!(evidence, "");
}

#[test]
fn missing_fields_are_rejected() {
    for args in [
        json!({ "why": "b" }),
        json!({ "decision": "a" }),
        json!({ "decision": 1, "why": "b" }),
        json!({ "decision": "a", "why": 2 }),
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
        json!({ "decision": "  ", "why": "b" }),
        json!({ "decision": "a", "why": "\t" }),
    ] {
        assert!(
            matches!(extract(&args), Err(PluginError::ValidationError(_))),
            "空白字段应被拒：{args}"
        );
    }
}

/// 证据不是字符串时拒绝，不静默丢弃——「填错类型」与「没填」是两件事。
#[test]
fn a_non_string_evidence_is_rejected() {
    assert!(matches!(
        extract(&json!({ "decision": "a", "why": "b", "evidence": 1 })),
        Err(PluginError::ValidationError(_))
    ));
}
