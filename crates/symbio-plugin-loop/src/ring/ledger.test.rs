use super::*;
use crate::ring::facts::{DebtKey, Fact};
use serde_json::json;

fn fact(kind: FactKind, id: &str, attempt: u32, at_ms: u64) -> Fact {
    Fact::new(kind, DebtKey::new("p", id, attempt), json!({}), at_ms)
}

#[test]
fn duplicate_delivery_is_absorbed() {
    let mut ledger = Ledger::new();
    assert!(ledger.append(fact(FactKind::Request, "d", 1, 0)).is_ok());
    assert_eq!(
        ledger.append(fact(FactKind::Request, "d", 1, 0)),
        Err(AppendError::Duplicate)
    );
    assert_eq!(ledger.facts().len(), 1);
}

#[test]
fn closing_finds_response_or_failed() {
    let mut ledger = Ledger::new();
    ledger.append(fact(FactKind::Request, "d", 1, 0)).unwrap();
    ledger.append(fact(FactKind::Response, "d", 1, 1)).unwrap();
    let key = DebtKey::new("p", "d", 1);
    assert_eq!(ledger.closing(&key).map(|f| f.kind), Some(FactKind::Response));
    assert!(ledger.closing(&DebtKey::new("p", "d", 2)).is_none());
}

#[test]
fn attempt_is_the_max_seen_number() {
    let mut ledger = Ledger::new();
    ledger.append(fact(FactKind::Request, "d", 1, 0)).unwrap();
    ledger.append(fact(FactKind::Retry, "d", 2, 1)).unwrap();
    assert_eq!(ledger.attempt("p", "d"), 2);
    assert_eq!(ledger.attempt("p", "other"), 0);
}

#[test]
fn dispatch_count_reads_sent_facts() {
    let mut ledger = Ledger::new();
    ledger.append(fact(FactKind::Sent, "a", 1, 0)).unwrap();
    ledger.append(fact(FactKind::Sent, "b", 1, 0)).unwrap();
    ledger.append(fact(FactKind::Sent, "a", 2, 0)).unwrap();
    assert_eq!(ledger.dispatch_count(), 3);
}

#[test]
fn unresolved_lists_only_open_requests() {
    let mut ledger = Ledger::new();
    ledger.append(fact(FactKind::Request, "a", 1, 0)).unwrap();
    ledger.append(fact(FactKind::Request, "b", 1, 0)).unwrap();
    ledger.append(fact(FactKind::Failed, "b", 1, 1)).unwrap();
    assert_eq!(ledger.unresolved(), vec![DebtKey::new("p", "a", 1)]);
}

#[test]
fn rebuilding_from_facts_keeps_one_copy_of_each() {
    let facts = vec![
        fact(FactKind::Request, "a", 1, 0),
        fact(FactKind::Request, "a", 1, 0),
        fact(FactKind::Sent, "a", 1, 0),
    ];
    let ledger = Ledger::from_facts(facts);
    assert_eq!(ledger.facts().len(), 2);
}
