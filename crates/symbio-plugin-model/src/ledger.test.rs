//! 两本账的单元测试：分币种 / 按主体 / as-of / 「没测量」与「测到零」。
//!
//! ## 夹具为什么按**章**而不按端口名给数
//!
//! 两本账认的是债行盖的章（`stamp.usage` / `stamp.spend`），不是端口名字面量
//! （`symbio_sdk::port_stamp` 的原文：读侧只认章与载荷）。夹具因此显式给章——
//! 按 `port == "model"` 推章会把「账认端口」这条错误口径写进用例，
//! 而生产里真正决定「进不进这本账」的是写那条债行的插件声明。

use super::{eval_spend, eval_tokens};
use serde_json::json;
use symbio_kernel::event::{Entity, Event, Verb};

/// 一条收束应答。`stamps` 是要盖的章（`usage` / `spend`），空 = 不盖。
fn response(
    id: &str,
    principal: &str,
    usage: Option<(u64, u64)>,
    spend: Option<(u64, &str)>,
    stamps: &[&str],
    ts: i64,
) -> Event {
    let mut stamp = serde_json::Map::new();
    stamp.insert("role".into(), json!("reader"));
    for s in stamps {
        stamp.insert((*s).to_string(), json!(true));
    }
    let mut payload = json!({
        "port": "model",
        "id": id,
        "attempt": 1,
        "principal": principal,
        "stamp": serde_json::Value::Object(stamp),
    });
    if let Some((input, output)) = usage {
        payload["usage"] = json!({ "input": input, "output": output });
    }
    if let Some((amount_nano, currency)) = spend {
        payload["spend"] = json!({ "amount_nano": amount_nano, "currency": currency });
    }
    Event::pending(id, "response", Entity::Scope, Verb::Asserted, "system")
        .with_produced_by(1)
        .with_ts(ts)
        .with_payload(payload)
}

fn field(value: &serde_json::Value, path: &[&str]) -> serde_json::Value {
    let mut cur = value;
    for key in path {
        cur = &cur[*key];
    }
    cur.clone()
}

// ── 金额 ──────────────────────────────────────────────────────────────────

#[test]
fn amounts_add_up_per_currency_and_per_principal() {
    let events = vec![
        response(
            "m-1",
            "session/main",
            None,
            Some((150_000_000, "usd")),
            &["spend"],
            10,
        ),
        response(
            "m-2",
            "session/main",
            None,
            Some((50_000_000, "USD")),
            &["spend"],
            20,
        ),
        response(
            "m-3",
            "sub/worker",
            None,
            Some((1_000_000, "USD")),
            &["spend"],
            30,
        ),
        response(
            "m-4",
            "session/main",
            None,
            Some((7, "cny")),
            &["spend"],
            40,
        ),
    ];
    let got = eval_spend(&events, i64::MAX);

    assert_eq!(
        field(&got, &["by_currency", "USD", "amount_nano"]),
        json!(201_000_000)
    );
    assert_eq!(field(&got, &["by_currency", "USD", "charges"]), json!(3));
    assert_eq!(
        field(
            &got,
            &[
                "by_currency",
                "USD",
                "by_principal",
                "session/main",
                "amount_nano"
            ]
        ),
        json!(200_000_000)
    );
    assert_eq!(
        field(
            &got,
            &[
                "by_currency",
                "USD",
                "by_principal",
                "session/main",
                "charges"
            ]
        ),
        json!(2)
    );
    assert_eq!(
        field(
            &got,
            &[
                "by_currency",
                "USD",
                "by_principal",
                "sub/worker",
                "amount_nano"
            ]
        ),
        json!(1_000_000)
    );
    assert_eq!(
        field(&got, &["by_currency", "CNY", "amount_nano"]),
        json!(7),
        "`cny` 与 `CNY` 是同一个桶"
    );
    let keys: Vec<&String> = got["by_currency"]
        .as_object()
        .expect("按币种摊开")
        .keys()
        .collect();
    assert_eq!(keys, vec!["CNY", "USD"], "币种字典序（BTreeMap）");
}

#[test]
fn the_stamp_decides_the_ledger_not_the_port_name() {
    // 三条同形应答：只有盖章的那条进账。端口名一律 `model`，差别只在章。
    let events = vec![
        response("m-1", "session/main", None, Some((9, "USD")), &[], 10),
        response(
            "m-2",
            "session/main",
            None,
            Some((9, "USD")),
            &["usage"],
            20,
        ),
        response(
            "m-3",
            "session/main",
            None,
            Some((9, "USD")),
            &["spend"],
            30,
        ),
    ];
    let got = eval_spend(&events, i64::MAX);
    assert_eq!(
        field(&got, &["by_currency", "USD", "amount_nano"]),
        json!(9)
    );
    assert_eq!(field(&got, &["by_currency", "USD", "charges"]), json!(1));
}

#[test]
fn as_of_hides_later_responses() {
    let events = vec![
        response(
            "m-1",
            "session/main",
            None,
            Some((100, "USD")),
            &["spend"],
            10,
        ),
        response(
            "m-2",
            "session/main",
            None,
            Some((200, "USD")),
            &["spend"],
            20,
        ),
    ];
    let got = eval_spend(&events, 10);
    assert_eq!(
        field(&got, &["by_currency", "USD", "amount_nano"]),
        json!(100)
    );
}

#[test]
fn zero_amount_is_a_charge_not_a_missing_entry() {
    let events = vec![response(
        "m-1",
        "session/main",
        None,
        Some((0, "USD")),
        &["spend"],
        10,
    )];
    let got = eval_spend(&events, i64::MAX);
    assert_eq!(
        field(&got, &["by_currency", "USD", "amount_nano"]),
        json!(0)
    );
    assert_eq!(
        field(&got, &["by_currency", "USD", "charges"]),
        json!(1),
        "明确免费 ≠ 没定价"
    );
}

#[test]
fn malformed_spend_is_skipped_instead_of_inventing_a_bucket() {
    let mut missing_currency = response("m-1", "session/main", None, None, &["spend"], 10);
    missing_currency.payload["spend"] = json!({ "amount_nano": 5 });
    let mut blank_currency = response("m-2", "session/main", None, None, &["spend"], 20);
    blank_currency.payload["spend"] = json!({ "amount_nano": 5, "currency": "  " });
    let mut negative = response("m-3", "session/main", None, None, &["spend"], 30);
    negative.payload["spend"] = json!({ "amount_nano": -5, "currency": "USD" });

    let got = eval_spend(&[missing_currency, blank_currency, negative], i64::MAX);
    assert_eq!(
        got["by_currency"].as_object().map(|m| m.len()),
        Some(0),
        "形状不对的账跳过，不猜币种、不收负数：{got}"
    );
}

// ── 用量 ──────────────────────────────────────────────────────────────────

#[test]
fn tokens_add_up_and_count_only_measured_responses() {
    let events = vec![
        response(
            "m-1",
            "session/main",
            Some((1000, 100)),
            None,
            &["usage"],
            10,
        ),
        response(
            "m-2",
            "session/main",
            Some((500, 100)),
            None,
            &["usage"],
            20,
        ),
        response("m-3", "sub/worker", Some((7, 3)), None, &["usage"], 30),
        // 没给 usage：章在、读数不在 —— 不进 measured、不补零。
        response("m-4", "session/main", None, None, &["usage"], 40),
        // 没盖章：usage 在也不算。
        response("m-5", "session/main", Some((9999, 9999)), None, &[], 50),
    ];
    let got = eval_tokens(&events, i64::MAX);

    assert_eq!(field(&got, &["calls"]), json!(3), "只有带实测的三条");
    assert_eq!(field(&got, &["input"]), json!(1507));
    assert_eq!(field(&got, &["output"]), json!(203));
    assert_eq!(
        field(&got, &["by_principal", "session/main", "measured"]),
        json!(2)
    );
    assert_eq!(
        field(&got, &["by_principal", "sub/worker", "output"]),
        json!(3)
    );
}

#[test]
fn no_measurement_at_all_reads_as_zero_calls_not_zero_tokens() {
    let events = vec![response("m-1", "session/main", None, None, &["usage"], 10)];
    let got = eval_tokens(&events, i64::MAX);
    assert_eq!(field(&got, &["calls"]), json!(0));
    assert_eq!(field(&got, &["input"]), json!(0));
    assert_eq!(
        got["by_principal"].as_object().map(|m| m.len()),
        Some(0),
        "没有实测就没有主体行：{got}"
    );
}

#[test]
fn zero_tokens_are_a_measurement_not_a_missing_one() {
    let events = vec![response(
        "m-1",
        "session/main",
        Some((0, 0)),
        None,
        &["usage"],
        10,
    )];
    let got = eval_tokens(&events, i64::MAX);
    assert_eq!(field(&got, &["calls"]), json!(1), "测到零 ≠ 没测量");
    assert_eq!(
        field(&got, &["by_principal", "session/main", "measured"]),
        json!(1)
    );
}
