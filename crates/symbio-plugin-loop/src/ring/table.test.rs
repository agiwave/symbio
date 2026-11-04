use super::*;
use crate::ring::facts::{Fact, FactKind};
use serde_json::json;

#[test]
fn table_returns_rows_in_order() {
    let table = Table::new(vec![
        DebtRow::new("a", "p", "d-a", json!({"q": 1}))
            .after(DebtKey::new(1, "p", "d-0", 1)),
        DebtRow::new("b", "p", "d-b", json!({"q": 2})),
    ]);
    let facts: Vec<Fact> = Vec::new();
    let view = FactView::new(&facts);
    let rows = table.rows(&view, 1);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].name, "a");
    assert_eq!(rows[0].id, "d-a");
    assert_eq!(rows[1].payload, json!({"q": 2}));
    assert!(rows[1].after.is_none());
}

#[test]
fn after_records_the_prerequisite_identity() {
    let key = DebtKey::new(1, "p", "d-a", 1);
    let row = DebtRow::new("b", "p", "d-b", json!({})).after(key.clone());
    assert_eq!(row.after, Some(key));
}

#[test]
fn lease_builder_clamps_to_at_least_one_attempt() {
    let row = DebtRow::new("a", "p", "d", json!({})).lease(5, 0);
    assert_eq!(row.lease_ms, 5);
    assert_eq!(row.max_attempts, 1);
}

#[test]
fn source_can_derive_rows_from_scoped_facts() {
    struct Once;
    impl DebtSource for Once {
        fn rows(&self, facts: &FactView<'_>, scope: u64) -> Vec<DebtRow> {
            if facts
                .all()
                .iter()
                .any(|f| f.scope == scope && f.kind == FactKind::Response)
            {
                Vec::new()
            } else {
                vec![DebtRow::new("a", "p", "d", json!({}))]
            }
        }
    }
    assert_eq!(Once.rows(&FactView::new(&[]), 1).len(), 1);
    let done = [Fact::new(
        FactKind::Response,
        DebtKey::new(1, "p", "d", 1),
        json!({}),
        0,
    )];
    assert!(Once.rows(&FactView::new(&done), 1).is_empty());
    assert_eq!(Once.rows(&FactView::new(&done), 2).len(), 1);
}
