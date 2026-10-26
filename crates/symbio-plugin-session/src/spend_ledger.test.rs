//! `spend_ledger` 的单元测试：分币种、按主体、as-of 与三种「空」。

use super::spend_ledger;
use symbio_core::{Entity, Event, Verb};

fn response(id: &str, port: &str, principal: &str, spend: Option<(u64, &str)>, ts: i64) -> Event {
    let mut payload = serde_json::json!({
        "port": port,
        "id": id,
        "attempt": 1,
        "principal": principal,
    });
    if let Some((amount_nano, currency)) = spend {
        payload["spend"] = serde_json::json!({
            "amount_nano": amount_nano,
            "currency": currency,
        });
    }
    Event::pending(id, "response", Entity::Turn, Verb::Asserted, 0, "system")
        .with_produced_by(1)
        .with_ts(ts)
        .with_payload(payload)
}

#[test]
fn amounts_add_up_per_currency_and_per_principal() {
    let events = vec![
        response(
            "m-1",
            "model",
            "session/main",
            Some((150_000_000, "usd")),
            10,
        ),
        response(
            "m-2",
            "model",
            "session/main",
            Some((50_000_000, "USD")),
            20,
        ),
        response("m-3", "model", "sub/worker", Some((1_000_000, "USD")), 30),
        response("m-4", "model", "session/main", Some((7, "cny")), 40),
    ];
    let got = spend_ledger().read_all(&events).value;

    let usd = &got.by_currency["USD"];
    assert_eq!(usd.amount_nano, 201_000_000);
    assert_eq!(usd.charges, 3, "三条带金额的应答");
    assert_eq!(usd.by_principal["session/main"].amount_nano, 200_000_000);
    assert_eq!(usd.by_principal["session/main"].charges, 2);
    assert_eq!(usd.by_principal["sub/worker"].amount_nano, 1_000_000);

    let cny = &got.by_currency["CNY"];
    assert_eq!(cny.amount_nano, 7, "`cny` 与 `CNY` 是同一个桶");
    assert_eq!(cny.charges, 1);
    assert_eq!(
        got.by_currency.keys().collect::<Vec<_>>(),
        vec!["CNY", "USD"],
        "币种字典序（BTreeMap）"
    );
}

#[test]
fn only_model_responses_with_spend_enter_the_ledger() {
    let events = vec![
        response("m-1", "model", "session/main", None, 10),
        response("c-1", "tool", "session/main", Some((999, "USD")), 20),
        response("m-2", "model", "session/main", Some((9, "USD")), 30),
    ];
    let got = spend_ledger().read_all(&events).value;
    assert_eq!(got.by_currency["USD"].amount_nano, 9);
    assert_eq!(got.by_currency["USD"].charges, 1);
}

#[test]
fn as_of_hides_later_responses() {
    let events = vec![
        response("m-1", "model", "session/main", Some((100, "USD")), 10),
        response("m-2", "model", "session/main", Some((200, "USD")), 20),
    ];
    let view = spend_ledger().apply(&events, 10, symbio_core::Budget::generous());
    assert_eq!(view.value.by_currency["USD"].amount_nano, 100);
}

#[test]
fn zero_amount_is_a_charge_not_a_missing_entry() {
    let events = vec![response(
        "m-1",
        "model",
        "session/main",
        Some((0, "USD")),
        10,
    )];
    let got = spend_ledger().read_all(&events).value;
    assert_eq!(got.by_currency["USD"].amount_nano, 0);
    assert_eq!(got.by_currency["USD"].charges, 1, "明确免费 ≠ 没定价");
}

#[test]
fn malformed_spend_is_skipped_instead_of_inventing_a_bucket() {
    let mut missing_currency = response("m-1", "model", "session/main", None, 10);
    missing_currency.payload["spend"] = serde_json::json!({ "amount_nano": 5 });
    let mut blank_currency = response("m-2", "model", "session/main", None, 20);
    blank_currency.payload["spend"] = serde_json::json!({ "amount_nano": 5, "currency": "  " });
    let mut negative = response("m-3", "model", "session/main", None, 30);
    negative.payload["spend"] = serde_json::json!({ "amount_nano": -5, "currency": "USD" });

    let got = spend_ledger()
        .read_all(&[missing_currency, blank_currency, negative])
        .value;
    assert!(
        got.by_currency.is_empty(),
        "形状不对的账跳过，不猜币种、不收负数：{:?}",
        got.by_currency
    );
}
