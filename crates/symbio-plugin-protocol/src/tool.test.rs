//! `tool.rs` 的单元测试 —— 入参的唯一校验点与名字形状。

use super::{extract_join, extract_proposal, messages_of, valid_name};
use crate::decl::{PROTOCOL_JOIN_TOOL, PROTOCOL_PROPOSE_TOOL};
use serde_json::json;
use symbio_plugin_api::PluginError;

#[test]
fn the_tool_names_are_the_ones_the_face_declares() {
    assert_eq!(PROTOCOL_PROPOSE_TOOL, "protocol_propose");
    assert_eq!(PROTOCOL_JOIN_TOOL, "protocol_join");
}

#[test]
fn a_complete_proposal_passes() {
    let args = json!({
        "name": " handoff ",
        "title": " 任务交接 ",
        "purpose": " 把一个任务交给另一个代理 ",
        "messages": [" request ", "accept", "done"],
    });
    let (name, title, purpose, messages) = extract_proposal(&args).expect("完整提案应通过");
    assert_eq!(name, "handoff", "两侧空白剥掉");
    assert_eq!(title, "任务交接");
    assert_eq!(purpose, "把一个任务交给另一个代理");
    assert_eq!(messages, vec!["request", "accept", "done"]);
}

/// 名字规则：小写字母开头、只含 `[a-z0-9_]`、3–40 字符。
#[test]
fn the_name_shape_is_enforced() {
    for good in ["a_b", "handoff", "proto_2"] {
        assert!(valid_name(good), "{good} 应合法");
    }
    for bad in ["", "ab", "_abc", "Handoff", "hand-off", "hand off", "协议"] {
        assert!(!valid_name(bad), "{bad} 应不合法");
    }
    assert!(!valid_name(&"a".repeat(41)), "超过 40 字符应不合法");
}

#[test]
fn an_invalid_name_is_rejected_on_both_tools() {
    let proposal = json!({
        "name": "Bad-Name",
        "title": "t",
        "purpose": "p",
        "messages": ["a"],
    });
    assert!(matches!(
        extract_proposal(&proposal),
        Err(PluginError::ValidationError(_))
    ));

    let join = json!({ "name": "Bad-Name", "participant": "reviewer" });
    assert!(matches!(
        extract_join(&join),
        Err(PluginError::ValidationError(_))
    ));
}

#[test]
fn missing_or_blank_proposal_fields_are_rejected() {
    for args in [
        json!({ "title": "t", "purpose": "p", "messages": ["a"] }),
        json!({ "name": "handoff", "purpose": "p", "messages": ["a"] }),
        json!({ "name": "handoff", "title": "t", "messages": ["a"] }),
        json!({ "name": "handoff", "title": "t", "purpose": "p" }),
        json!({ "name": "handoff", "title": "  ", "purpose": "p", "messages": ["a"] }),
        json!({ "name": 1, "title": "t", "purpose": "p", "messages": ["a"] }),
    ] {
        assert!(
            matches!(
                extract_proposal(&args),
                Err(PluginError::ValidationError(_))
            ),
            "缺字段或类型不对应被拒：{args}"
        );
    }
}

/// 消息序列至少一条、每条非空字符串、去空白去重。
#[test]
fn the_message_sequence_is_validated_and_normalized() {
    for bad in [
        json!([]),
        json!(["a", ""]),
        json!(["a", "  "]),
        json!(["a", 1]),
        json!("a"),
    ] {
        let args = json!({
            "name": "handoff",
            "title": "t",
            "purpose": "p",
            "messages": bad,
        });
        assert!(
            matches!(
                extract_proposal(&args),
                Err(PluginError::ValidationError(_))
            ),
            "非法消息序列应被拒：{bad}"
        );
    }

    let args = json!({
        "name": "handoff",
        "title": "t",
        "purpose": "p",
        "messages": [" a ", "a", "b"],
    });
    let (_, _, _, messages) = extract_proposal(&args).expect("合法消息序列应通过");
    assert_eq!(messages, vec!["a", "b"]);
}

/// `messages_of` 是工具与行为共用的唯一口径（测试它等于钉住两侧一致）。
#[test]
fn messages_of_is_the_single_reader_of_the_sequence() {
    assert_eq!(
        messages_of(&json!({ "messages": ["a"] })).unwrap(),
        vec!["a"]
    );
    assert!(messages_of(&json!({})).is_err());
}

#[test]
fn a_complete_join_passes() {
    let args = json!({ "name": " handoff ", "participant": " reviewer " });
    let (name, participant) = extract_join(&args).expect("完整加入应通过");
    assert_eq!(name, "handoff");
    assert_eq!(participant, "reviewer");
}

#[test]
fn a_blank_or_overlong_participant_is_rejected() {
    for participant in [json!(""), json!("  "), json!("x".repeat(65)), json!(1)] {
        let args = json!({ "name": "handoff", "participant": participant });
        assert!(
            matches!(extract_join(&args), Err(PluginError::ValidationError(_))),
            "非法参与者应被拒：{participant}"
        );
    }
    assert!(extract_join(&json!({ "name": "handoff", "participant": "x".repeat(64) })).is_ok());
}

/// 入参里夹带别的字段不影响结果：它们不在 schema 里，也不被读取。
#[test]
fn extra_arguments_are_ignored() {
    let args = json!({
        "name": "handoff",
        "title": "t",
        "purpose": "p",
        "messages": ["a"],
        "owner": "vdfs",
    });
    let (name, _, _, _) = extract_proposal(&args).expect("多余字段应被忽略");
    assert_eq!(name, "handoff");
}
