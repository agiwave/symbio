use super::*;
use serde_json::json;

fn render(_views: &[Value], _ctx: &ReportCtx<'_>) -> Value {
    Value::Null
}

fn decl(
    owner: &'static str,
    slot: &'static str,
    order: u8,
    views: &'static [&'static str],
) -> ReportDecl {
    ReportDecl {
        owner,
        slot,
        order,
        views,
        fleet: None,
        render,
        fleet_render: None,
        panel: None,
    }
}

#[test]
fn valid_registration_has_no_errors() {
    let a = decl("cost", "cost", 40, &["cost.ledger"]);
    let b = decl("skill", "calibration", 50, &["skill.calibration"]);
    let refs = [&a, &b];
    assert!(validate(&refs).is_empty());
}

#[test]
fn duplicate_slot_is_rejected() {
    let a = decl("cost", "cost", 40, &["cost.ledger"]);
    let b = decl("reputation", "cost", 50, &["reputation.ledger"]);
    let refs = [&a, &b];
    assert_eq!(
        validate(&refs),
        vec![ReportError::DuplicateSlot {
            slot: "cost",
            first: "cost",
            second: "reputation",
        }]
    );
}

#[test]
fn same_view_in_two_reports_is_rejected() {
    let a = decl("slo", "tiers", 10, &["slo.latency", "slo.fallback"]);
    let b = decl("slo", "span", 20, &["slo.latency"]);
    let refs = [&a, &b];
    assert_eq!(
        validate(&refs),
        vec![ReportError::DuplicateView {
            view: "slo.latency",
            first_slot: "tiers",
            second_slot: "span",
        }]
    );
}

#[test]
fn empty_slot_is_rejected() {
    let a = decl("cost", "", 40, &["cost.ledger"]);
    assert_eq!(
        validate(&[&a]),
        vec![ReportError::EmptySlot { owner: "cost" }]
    );
}

#[test]
fn empty_views_is_rejected() {
    let a = decl("cost", "cost", 40, &[]);
    assert_eq!(
        validate(&[&a]),
        vec![ReportError::EmptyViews {
            owner: "cost",
            slot: "cost",
        }]
    );
}

#[test]
fn empty_view_name_is_rejected() {
    let a = decl("cost", "cost", 40, &[""]);
    assert_eq!(
        validate(&[&a]),
        vec![ReportError::EmptyViewName {
            owner: "cost",
            slot: "cost",
            index: 0,
        }]
    );
}

#[test]
fn empty_fleet_is_rejected() {
    let mut a = decl("cost", "cost", 40, &["cost.ledger"]);
    a.fleet = Some("");
    assert_eq!(
        validate(&[&a]),
        vec![ReportError::EmptyFleet {
            owner: "cost",
            slot: "cost",
        }]
    );
}

#[test]
fn order_ties_break_by_slot_lexicographically() {
    let a = decl("skill", "calibration", 40, &["skill.calibration"]);
    let b = decl("cost", "cost", 40, &["cost.ledger"]);
    let sorted = sorted(vec![&a, &b]);
    let slots: Vec<&str> = sorted.iter().map(|r| r.slot).collect();
    assert_eq!(slots, vec!["calibration", "cost"]);
}

#[test]
fn same_registration_serializes_byte_identically_twice() {
    let a = decl("skill", "calibration", 40, &["skill.calibration"]);
    let b = decl("cost", "cost", 40, &["cost.ledger"]);
    let c = decl("conation", "conation", 10, &["conation.intents"]);

    let encode = || {
        let rows: Vec<(&str, u8, &str)> = sorted(vec![&a, &b, &c])
            .iter()
            .map(|r| (r.slot, r.order, r.owner))
            .collect();
        serde_json::to_vec(&rows).expect("夹具应能序列化")
    };

    assert_eq!(encode(), encode());
}

#[test]
fn readable_is_distinguished_from_a_failed_read() {
    fn empty_when_unreadable(_views: &[Value], ctx: &ReportCtx<'_>) -> Value {
        if ctx.readable {
            json!({ "by_skill": {} })
        } else {
            json!({})
        }
    }

    let unreadable = ReportCtx {
        principal: "p",
        cap: 4,
        readable: false,
    };
    let readable = ReportCtx {
        readable: true,
        ..unreadable
    };
    assert_eq!(empty_when_unreadable(&[], &unreadable), json!({}));
    assert_eq!(
        empty_when_unreadable(&[], &readable),
        json!({ "by_skill": {} })
    );
}
