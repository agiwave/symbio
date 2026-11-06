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
    let nodes = transcript_nodes(&facts);
    // 角色由事实给：两轮用户发言是两条 user 消息（不是一条「用户：…」正文）。
    assert_eq!(nodes.len(), 3);
    assert_eq!(nodes[0].node_role(), Some(NodeRole::User));
    assert_eq!(text_of(&nodes[0]), "问题一");
    assert_eq!(nodes[1].node_role(), Some(NodeRole::Assistant));
    assert_eq!(text_of(&nodes[1]), "答复一");
    assert_eq!(nodes[2].node_role(), Some(NodeRole::User));
    assert_eq!(text_of(&nodes[2]), "问题二");
}

fn text_of(node: &ChatNode) -> String {
    node.content
        .as_ref()
        .map(MessageContent::to_text)
        .unwrap_or_default()
}
