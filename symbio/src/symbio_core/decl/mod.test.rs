//! 声明层的单元用例：装载 / 冻结 / 三道检查，以及两个首批视图的口径。
//!
//! 反向用例是主菜——一个只会让正确输入通过的白名单等于没有检查。

use serde_json::{json, Value};

use super::super::event::{Entity, Verb};
use super::*;

fn ev(turn: u64, kind: &str, payload: Value) -> Event {
    Event::pending(
        format!("{kind}-{turn}-{}", payload.to_string().len()),
        kind,
        Entity::Turn,
        Verb::Asserted,
        turn,
        "test",
    )
    .with_payload(payload)
}

// ── 装载 ─────────────────────────────────────────────────────────────────

#[test]
fn builtin_loads_and_freezes() {
    let frozen = builtin().expect("首批声明必须能冻结");
    let facts: Vec<_> = frozen.facts().iter().map(|f| f.name).collect();
    assert_eq!(
        facts,
        vec![
            REQUEST,
            SENT,
            RESPONSE,
            FAILED,
            "chat.assistant.final",
            "turn.stage",
            "user.message",
            "authz.denied",
        ]
    );
    let views: Vec<_> = frozen.views().iter().map(|v| v.name).collect();
    assert_eq!(views, vec!["in_flight", "turn_head"]);

    // 端口是载荷字段：协议族声明它，收束 / 阶段 / 用户 / 拒绝都不带。
    for name in [REQUEST, SENT, RESPONSE, FAILED] {
        assert_eq!(frozen.fact(name).unwrap().port, Some("port"), "{name}");
    }
    assert_eq!(frozen.fact("chat.assistant.final").unwrap().port, None);

    // `ver` 是口径的显式版本：T-225 给 `turn_head` 加 `session.config` 时升到 2。
    assert_eq!(frozen.view("in_flight").unwrap().ver, 1);
    assert_eq!(frozen.view("turn_head").unwrap().ver, 1);
}

#[test]
fn every_declared_reference_is_a_field() {
    // `port` / `key` / `expand` 都是字段引用；声明不能自相矛盾。
    let frozen = builtin().unwrap();
    for f in frozen.facts() {
        for field in f
            .port
            .into_iter()
            .chain(f.key.iter().copied())
            .chain(f.expand.iter().copied())
        {
            assert!(
                f.fields.contains(&field),
                "{} 引用了未声明字段 {}",
                f.name,
                field
            );
        }
    }
}

// ── 三道检查 ─────────────────────────────────────────────────────────────

#[test]
fn view_cannot_read_an_undeclared_field() {
    let mut b = Builder::new();
    b.fact(FactDecl {
        name: REQUEST,
        port: Some("port"),
        key: &["id"],
        expand: &[],
        fields: &["port", "id"],
    });
    b.view(ViewDecl {
        name: "probe",
        ver: 1,
        reads: &[(REQUEST, "attmpt")],
        cost: 1,
        eval: in_flight,
    });
    assert_eq!(
        b.seal().unwrap_err(),
        vec![CheckError::UndeclaredField {
            view: "probe",
            fact: REQUEST,
            field: "attmpt",
        }]
    );
}

#[test]
fn view_cannot_read_an_undeclared_fact() {
    let mut b = Builder::new();
    b.view(ViewDecl {
        name: "probe",
        ver: 1,
        reads: &[("ghost", "id")],
        cost: 1,
        eval: in_flight,
    });
    assert_eq!(
        b.seal().unwrap_err(),
        vec![CheckError::UnknownFact {
            view: "probe",
            fact: "ghost",
        }]
    );
}

#[test]
fn duplicate_fact_is_rejected() {
    let mut b = Builder::new();
    b.fact(BUILTIN_FACTS[0]);
    b.fact(BUILTIN_FACTS[0]);
    assert_eq!(
        b.seal().unwrap_err(),
        vec![CheckError::DuplicateFact { name: REQUEST }]
    );
}

#[test]
fn duplicate_view_is_rejected() {
    let mut b = Builder::new();
    for &f in BUILTIN_FACTS {
        b.fact(f);
    }
    b.view(BUILTIN_VIEWS[0]);
    b.view(BUILTIN_VIEWS[0]);
    assert_eq!(
        b.seal().unwrap_err(),
        vec![CheckError::DuplicateView { name: "in_flight" }]
    );
}

#[test]
fn fact_and_view_share_one_name_space() {
    let mut b = Builder::new();
    b.fact(BUILTIN_FACTS[0]);
    b.view(ViewDecl {
        name: REQUEST,
        ver: 1,
        reads: &[(REQUEST, "id")],
        cost: 1,
        eval: in_flight,
    });
    assert_eq!(
        b.seal().unwrap_err(),
        vec![CheckError::NameCollision { name: REQUEST }]
    );
}

// ── 视图的两条隔离 + 首批口径 ─────────────────────────────────────────────

#[test]
fn eval_takes_only_events_and_now() {
    // 签名即隔离：视图拿不到 `&Store`（写口）也拿不到别的视图。
    let eval: fn(&[Event], i64) -> Value = in_flight;
    assert_eq!(eval(&[], 0), json!([]));
    assert_eq!(
        builtin().unwrap().eval("in_flight", &[], 0),
        Some(json!([]))
    );
    assert_eq!(builtin().unwrap().eval("ghost", &[], 0), None);
}

#[test]
fn eval_respects_declared_cost() {
    // `cost` 是声明的扫描上界；超界返回 `None`，不静默算一半。
    let mut b = Builder::new();
    b.fact(BUILTIN_FACTS[1]);
    b.view(ViewDecl {
        name: "probe",
        ver: 1,
        reads: &[(SENT, "id")],
        cost: 1,
        eval: in_flight,
    });
    let frozen = b.seal().unwrap();
    let one = vec![ev(
        1,
        SENT,
        json!({"port": "turn", "id": "a", "attempt": 1}),
    )];
    assert!(frozen.eval("probe", &one, 0).is_some());
    let two = vec![
        one[0].clone(),
        ev(1, SENT, json!({"port": "turn", "id": "b", "attempt": 1})),
    ];
    assert_eq!(frozen.eval("probe", &two, 0), None);
}

#[test]
fn in_flight_holds_sent_until_response_or_failure() {
    let events = vec![
        ev(1, SENT, json!({"port": "turn", "id": "a", "attempt": 1})),
        ev(1, SENT, json!({"port": "turn", "id": "b", "attempt": 1})),
        ev(
            1,
            RESPONSE,
            json!({"port": "turn", "id": "a", "attempt": 1}),
        ),
        ev(2, SENT, json!({"port": "turn", "id": "c", "attempt": 2})),
        ev(2, FAILED, json!({"port": "turn", "id": "c", "why": "boom"})),
    ];
    assert_eq!(
        builtin().unwrap().eval("in_flight", &events, 0),
        Some(json!([{"id": "b", "port": "turn", "attempt": 1}]))
    );
}

#[test]
fn in_flight_is_attempt_scoped() {
    // 旧尝试的 `response` 不能收清新尝试的 `sent`（T-223 判据 8 的声明侧）。
    let events = vec![
        ev(1, SENT, json!({"port": "turn", "id": "a", "attempt": 1})),
        ev(1, SENT, json!({"port": "turn", "id": "a", "attempt": 2})),
        ev(
            1,
            RESPONSE,
            json!({"port": "turn", "id": "a", "attempt": 1}),
        ),
    ];
    assert_eq!(
        builtin().unwrap().eval("in_flight", &events, 0),
        Some(json!([{"id": "a", "port": "turn", "attempt": 2}]))
    );
}

#[test]
fn turn_head_reads_round_and_done_from_facts() {
    let events = vec![
        ev(1, "user.message", json!({"text": "hi"})),
        ev(1, "turn.stage", json!({"stage": "recall", "round": 0})),
        ev(1, "turn.stage", json!({"stage": "decide", "round": 0})),
    ];
    assert_eq!(
        turn_head(&events, 0),
        json!({"turn": 1, "round": 0, "has_utterance": true, "done": true})
    );

    // 下一轮的阶段事实出现后，轮头跟着走；`done` 只认**当前 round** 的判决格。
    let events = vec![
        ev(1, "user.message", json!({"text": "hi"})),
        ev(1, "turn.stage", json!({"stage": "decide", "round": 0})),
        ev(2, "user.message", json!({"text": "again"})),
        ev(2, "turn.stage", json!({"stage": "recall", "round": 1})),
    ];
    assert_eq!(
        turn_head(&events, 0),
        json!({"turn": 2, "round": 1, "has_utterance": true, "done": false})
    );
}

#[test]
fn turn_head_is_empty_without_facts() {
    assert_eq!(turn_head(&[], 0), Value::Null);
}
