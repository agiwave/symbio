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
    let key = DebtKey::new(7, "p", "d-1", 1);
    assert_eq!(key.next_attempt(), DebtKey::new(7, "p", "d-1", 2));
}

#[test]
fn reply_travels_on_the_closing_fact() {
    let key = DebtKey::new(7, "p", "d-1", 1);
    let fact = Fact::new(FactKind::Response, key, json!({"answer": "ok"}), 7)
        .with_reply(PortReply::default());
    assert_eq!(fact.at_ms, 7);
    assert_eq!(fact.scope, 7);
    assert!(fact.reply.is_some());
}

#[test]
fn projection_reads_one_scope_only() {
    let facts = vec![
        Fact::new(
            FactKind::Request,
            DebtKey::new(1, "p", "d", 1),
            json!({}),
            0,
        ),
        Fact::new(FactKind::Sent, DebtKey::new(1, "p", "d", 1), json!({}), 0),
        Fact::new(
            FactKind::Response,
            DebtKey::new(2, "p", "d", 1),
            json!({}),
            0,
        ),
    ];
    let view = FactView::new(&facts);
    assert_eq!(view.attempt(1, "p", "d"), 1);
    assert_eq!(view.attempt(2, "p", "d"), 1);
    assert!(view.last_sent(&DebtKey::new(2, "p", "d", 1)).is_none());
    assert!(view.closing(&DebtKey::new(1, "p", "d", 1)).is_none());
    assert_eq!(view.unresolved(1).len(), 1);
    assert!(view.unresolved(2).is_empty());
    assert_eq!(view.last_closing(2, "p").unwrap().kind, FactKind::Response);
}
