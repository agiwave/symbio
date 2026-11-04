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
fn spec_builder_carries_identity_as_data() {
    let spec = DebtSpec::new("row", "demo", "demo:1", json!({"q": 1}))
        .after("demo:0")
        .lease(1_000, 2);
    assert_eq!(spec.port, "demo");
    assert_eq!(spec.id, "demo:1");
    assert_eq!(spec.after.as_deref(), Some("demo:0"));
    assert_eq!(spec.lease_ms, 1_000);
    assert_eq!(spec.max_attempts, 2);
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
