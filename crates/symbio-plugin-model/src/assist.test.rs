use super::*;
use serde_json::json;
use symbio_plugin_api::chat_node::NodeRole;

#[test]
fn transcript_projects_one_node_per_role() {
    let facts = vec![
        PortFact {
            id: "u-1".into(),
            port: String::new(),
            kind: EVENT_USER_MESSAGE.into(),
            payload: json!({ "text": "问题一" }),
        },
        PortFact {
            id: "chat.assistant.final:u-1".into(),
            port: "model".into(),
            kind: EVENT_CHAT_ASSISTANT_FINAL.into(),
            payload: json!({ "text": "答复一" }),
        },
        PortFact {
            id: "u-2".into(),
            port: String::new(),
            kind: EVENT_USER_MESSAGE.into(),
            payload: json!({ "text": "问题二" }),
        },
    ];
    let nodes = transcript_nodes(&facts, None).expect("无切片时应读整份");
    // 角色由事实给：两轮用户发言是两条 user 消息（不是一条「用户：…」正文）。
    assert_eq!(nodes.len(), 3);
    assert_eq!(nodes[0].node_role(), Some(NodeRole::User));
    assert_eq!(text_of(&nodes[0]), "问题一");
    assert_eq!(nodes[1].node_role(), Some(NodeRole::Assistant));
    assert_eq!(text_of(&nodes[1]), "答复一");
    assert_eq!(nodes[2].node_role(), Some(NodeRole::User));
    assert_eq!(text_of(&nodes[2]), "问题二");
}

/// 切片是**硬边界**：只在两端事实之间取，端点缺失 ⇒ 响亮失败（不退回整份）。
#[test]
fn slice_projects_only_the_declared_window() {
    let facts = vec![
        PortFact {
            id: "u-1".into(),
            port: String::new(),
            kind: EVENT_USER_MESSAGE.into(),
            payload: json!({ "text": "问题一" }),
        },
        PortFact {
            id: "chat.assistant.final:u-1".into(),
            port: "model".into(),
            kind: EVENT_CHAT_ASSISTANT_FINAL.into(),
            payload: json!({ "text": "答复一" }),
        },
        PortFact {
            id: "u-2".into(),
            port: String::new(),
            kind: EVENT_USER_MESSAGE.into(),
            payload: json!({ "text": "问题二" }),
        },
    ];
    let slice = TranscriptSlice::from_payload(&json!({
        "slice": { "from": "u-1", "to": "chat.assistant.final:u-1" }
    }))
    .expect("切片载荷可读")
    .expect("应带切片");
    let nodes = transcript_nodes(&facts, Some(&slice)).expect("端点都在读面");
    assert_eq!(nodes.len(), 2, "只取端点之间的两条");
    assert_eq!(text_of(&nodes[0]), "问题一");
    assert_eq!(text_of(&nodes[1]), "答复一");

    let missing = TranscriptSlice::from_payload(&json!({
        "slice": { "from": "u-404", "to": "chat.assistant.final:u-1" }
    }))
    .expect("切片载荷可读")
    .expect("应带切片");
    assert!(
        transcript_nodes(&facts, Some(&missing)).is_err(),
        "端点不在读面必须失败，而不是退回整份"
    );
}

/// 极端单条：按字符预算取头尾，中间留省略标记；正文长度不随原长增长。
#[test]
fn oversized_node_is_split_head_and_tail() {
    let long = "长".repeat(2_000);
    let facts = vec![PortFact {
        id: "u-1".into(),
        port: String::new(),
        kind: EVENT_USER_MESSAGE.into(),
        payload: json!({ "text": long }),
    }];
    let slice = TranscriptSlice::from_payload(&json!({
        "slice": {
            "from": "u-1",
            "to": "u-1",
            "truncate": { "u-1": { "head_chars": 60, "tail_chars": 40 } }
        }
    }))
    .expect("切片载荷可读")
    .expect("应带切片");
    let nodes = transcript_nodes(&facts, Some(&slice)).expect("端点都在读面");
    assert_eq!(nodes.len(), 1);
    let text = text_of(&nodes[0]);
    assert_eq!(
        text.chars().count(),
        60 + "…中间省略…\n".chars().count() + 40
    );
    assert!(text.contains("…中间省略…"));
}

fn text_of(node: &ChatNode) -> String {
    node.content
        .as_ref()
        .map(MessageContent::to_text)
        .unwrap_or_default()
}
