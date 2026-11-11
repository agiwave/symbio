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
fn an_empty_inventory_seals_as_ok_and_is_stable() {
    let first = reports().expect("本 crate 的测试进程还没有报表登记");
    let second = reports().expect("同一进程第二次读应给同一结论");
    assert!(first.is_empty());
    assert_eq!(first.as_ptr(), second.as_ptr());
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

// ── 出口对账（`D-465`，`D-463` 第三级）─────────────────────────────────────

#[test]
fn owner_short_strips_the_crate_path_not_the_name() {
    assert_eq!(owner_short("crates/symbio-plugin-model"), "model");
    assert_eq!(owner_short("crates/symbio-plugin-event_bus"), "event_bus");
    // 短名本来就只有一种拼写：原样通过，不给它编一条规则。
    assert_eq!(owner_short("model"), "model");
    assert_eq!(owner_short("kernel"), "kernel");
}

#[test]
fn a_report_reading_another_owners_view_is_rejected() {
    let a = decl("cost", "cost", 40, &["model.usage"]);
    let errors = check_view_owners(&[&a], |name| match name {
        "model.usage" => Some("crates/symbio-plugin-model"),
        _ => None,
    });
    assert_eq!(errors.len(), 1, "只该有一条诊断：{errors:?}");
    for must in [
        "`cost`",
        "`model.usage`",
        "crates/symbio-plugin-model",
        "折这条账",
    ] {
        assert!(errors[0].contains(must), "诊断里缺 `{must}`：{}", errors[0]);
    }
}

#[test]
fn a_view_nobody_declared_is_rejected_rather_than_read_as_null() {
    let a = decl("cost", "cost", 40, &["cost.ledgr"]);
    let errors = check_view_owners(&[&a], |_| None);
    assert_eq!(errors.len(), 1, "只该有一条诊断：{errors:?}");
    for must in ["`cost.ledgr`", "冻结声明表"] {
        assert!(errors[0].contains(must), "诊断里缺 `{must}`：{}", errors[0]);
    }
}

#[test]
fn same_owner_in_two_spellings_passes_the_reconciliation() {
    let a = decl("model", "usage", 41, &["model.usage"]);
    let b = decl("model", "spend", 42, &["model.spend"]);
    let errors = check_view_owners(&[&a, &b], |name| match name {
        "model.usage" | "model.spend" => Some("crates/symbio-plugin-model"),
        _ => None,
    });
    assert!(
        errors.is_empty(),
        "同一 owner 的两种拼写不该被对账判红：{errors:?}"
    );
}

#[test]
fn an_empty_view_name_is_left_to_the_pure_validation() {
    // 空名归 `EmptyViewName`（`validate` 已判）：对账跳过后不再报第二遍，
    // 否则同一处错误会被数两次，两条判据的红条数对不上。
    let a = decl("cost", "cost", 40, &[""]);
    assert!(check_view_owners(&[&a], |_| None).is_empty());
    assert_eq!(
        validate(&[&a]),
        vec![ReportError::EmptyViewName {
            owner: "cost",
            slot: "cost",
            index: 0,
        }]
    );
}
