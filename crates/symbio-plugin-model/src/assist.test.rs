use super::*;
use serde_json::json;

#[test]
fn transcript_reads_user_and_final_facts() {
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
    ];
    assert_eq!(transcript(&facts), "用户：问题一\n助手：答复一\n");
}
