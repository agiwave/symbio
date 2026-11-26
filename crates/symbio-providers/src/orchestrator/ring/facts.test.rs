use super::*;
use serde_json::json;
use symbio_plugin_api::port::PortReply;

#[test]
fn five_kind_names_round_trip() {
    for kind in FactKind::ALL {
        assert_eq!(FactKind::parse(kind.as_str()), Some(kind));
    }
    assert_eq!(FactKind::parse("something-else"), None);
}

#[test]
fn only_response_and_failed_close_a_debt() {
    assert!(FactKind::Response.closes());
    assert!(FactKind::Failed.closes());
    assert!(!FactKind::Request.closes());
    assert!(!FactKind::Sent.closes());
    assert!(!FactKind::Retry.closes());
}

#[test]
fn next_attempt_keeps_identity_and_advances_lease() {
    let key = DebtKey::new("p", "d-1", 1);
    assert_eq!(key.next_attempt(), DebtKey::new("p", "d-1", 2));
}

#[test]
fn reply_travels_on_the_closing_fact() {
    let key = DebtKey::new("p", "d-1", 1);
    let fact = Fact::new(FactKind::Response, key, json!({"answer": "ok"}), 7)
        .with_reply(PortReply::default());
    assert_eq!(fact.at_ms, 7);
    assert!(fact.reply.is_some());
}

#[test]
fn projection_reads_the_session_facts() {
    let facts = vec![
        Fact::new(FactKind::Request, DebtKey::new("p", "d", 1), json!({}), 0),
        Fact::new(FactKind::Sent, DebtKey::new("p", "d", 1), json!({}), 0),
        Fact::new(FactKind::Response, DebtKey::new("p", "d", 1), json!({}), 0),
    ];
    let view = FactView::new(&facts);
    assert_eq!(view.attempt("p", "d"), 1);
    assert!(view.last_sent(&DebtKey::new("p", "d", 1)).is_some());
    assert!(view.closing(&DebtKey::new("p", "d", 1)).is_some());
    assert!(view.unresolved().is_empty());
    assert_eq!(view.last_closing("p").unwrap().kind, FactKind::Response);
}
