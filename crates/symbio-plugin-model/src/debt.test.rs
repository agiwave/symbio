use super::*;

fn request(provider: Option<&str>) -> DriveRequest {
    DriveRequest {
        scope: 7,
        session_id: "s".into(),
        provider: provider.map(str::to_string),
    }
}

#[test]
fn no_user_input_means_no_model_debt() {
    let rows = ModelDebts.rows(&request(Some("p")), &DebtFacts::default());
    assert!(rows.is_empty(), "没有输入事实就没有可派的债");
}

#[test]
fn user_turn_debt_derives_its_identity_from_the_input_fact() {
    let mut facts = DebtFacts::default();
    facts.note_fact(EVENT_USER_MESSAGE, "u-1");

    let rows = ModelDebts.rows(&request(Some("p")), &facts);
    assert_eq!(rows.len(), 1, "没有工具产物时只有用户输入那一行");

    let reply = &rows[0];
    assert_eq!(reply.name, "model-reply");
    assert_eq!(reply.port, MODEL_PORT);
    assert_eq!(reply.id, "model:u-1");
    assert_eq!(reply.when.as_deref(), Some(EVENT_USER_MESSAGE));
    assert_eq!(
        reply.unless,
        Some(symbio_plugin_api::drive::Unless::Id {
            kind: EVENT_CHAT_ASSISTANT_FINAL.into(),
            id: "chat.assistant.final:u-1".into(),
        })
    );
    assert_eq!(reply.payload["root_id"], "u-1");
    assert_eq!(reply.payload["provider"], "p");
}

#[test]
fn tool_result_adds_a_second_row_with_its_own_identity() {
    let mut facts = DebtFacts::default();
    facts.note_fact(EVENT_USER_MESSAGE, "u-1");
    facts.note_fact(EVENT_ARTIFACT_ADDED, "a-1");

    let rows = ModelDebts.rows(&request(None), &facts);
    assert_eq!(rows.len(), 2);

    let after_tools = &rows[1];
    assert_eq!(after_tools.name, "model-after-tools");
    assert_eq!(after_tools.id, "model:after:a-1");
    assert_eq!(after_tools.when_no_open, Some(TOOL_PORT));
    assert!(after_tools.when.is_none(), "工具轮之后只看端口是否还有未还债");
}

#[test]
fn latest_input_fact_is_the_turn_identity() {
    let mut facts = DebtFacts::default();
    facts.note_fact(EVENT_USER_MESSAGE, "u-1");
    facts.note_fact(EVENT_USER_MESSAGE, "u-2");

    let rows = ModelDebts.rows(&request(None), &facts);
    assert_eq!(rows[0].id, "model:u-2");
    assert_eq!(rows[0].payload["root_id"], "u-2");
}
