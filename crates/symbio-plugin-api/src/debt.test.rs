use super::*;
use serde_json::json;

#[test]
fn facts_merge_pairing_and_attempts_by_id() {
    let mut facts = DebtFacts::default();
    facts.note_attempt("d", 1);
    facts.note_attempt("d", 3);
    facts.note_closing("d", Closing::Ok, json!({"answer": "ok"}));

    assert_eq!(facts.attempt("d"), 3);
    assert_eq!(facts.attempt("missing"), 0);
    let (closing, payload) = facts.closing("d").unwrap();
    assert_eq!(closing, Closing::Ok);
    assert_eq!(payload, &json!({"answer": "ok"}));
}

#[test]
fn fact_identities_are_read_by_kind_without_counting() {
    let mut facts = DebtFacts::default();
    assert_eq!(facts.latest_fact_id("user.message"), None);
    facts.note_fact("user.message", "u-1");
    facts.note_fact("user.message", "u-2");
    facts.note_fact("response", "r-1");
    assert_eq!(facts.latest_fact_id("user.message"), Some("u-2"));
    assert_eq!(facts.latest_fact_id("response"), Some("r-1"));
}

#[test]
fn spec_builder_carries_identity_as_data() {
    let spec = DebtSpec::new("row", "demo", "demo:1", json!({"q": 1}))
        .after("demo:0")
        .unless("turn.done")
        .lease(1_000, 2);
    assert_eq!(spec.port, "demo");
    assert_eq!(spec.id, "demo:1");
    assert_eq!(spec.after.as_deref(), Some("demo:0"));
    assert_eq!(spec.unless, Some(Unless::Kind("turn.done".into())));
    assert_eq!(spec.lease_ms, 1_000);
    assert_eq!(spec.max_attempts, 2);
}

#[test]
fn identity_level_unless_names_both_the_kind_and_the_id() {
    let spec =
        DebtSpec::new("row", "demo", "demo:1", json!({})).unless_id("turn.done", "turn.done:u-1");
    assert_eq!(
        spec.unless,
        Some(Unless::Id {
            kind: "turn.done".into(),
            id: "turn.done:u-1".into(),
        })
    );
}

#[test]
fn debt_rows_are_registered_under_the_debt_family() {
    struct Once;
    impl DebtSource for Once {
        fn rows(&self, _request: &DriveRequest, _facts: &DebtFacts) -> Vec<DebtSpec> {
            vec![DebtSpec::new("row", "demo", "demo:1", json!({}))]
        }
    }
    let row = debt_row("demo", Arc::new(Once));
    assert!(row.payload::<DebtTable>().is_some());
}
