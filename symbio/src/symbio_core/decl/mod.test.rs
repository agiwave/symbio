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

fn ev_at(turn: u64, kind: &str, payload: Value, ts: i64) -> Event {
    ev(turn, kind, payload).with_ts(ts)
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
            "chat.assistant.fallback",
            "chat.assistant.reported",
            "chat.assistant.opening",
            "chat.assistant.progress",
            "turn.stage",
            "user.message",
            "turn.supplemented",
            "session.config",
            "turn.dispatch",
            "authz.denied",
        ]
    );
    let views: Vec<_> = frozen.views().iter().map(|v| v.name).collect();
    assert_eq!(views, vec!["in_flight", "turn_head", "report_due"]);

    // 端口是载荷字段：协议族声明它，收束 / 阶段 / 用户 / 拒绝都不带。
    for name in [REQUEST, SENT, RESPONSE, FAILED] {
        assert_eq!(frozen.fact(name).unwrap().port, Some("port"), "{name}");
    }
    assert_eq!(frozen.fact("chat.assistant.final").unwrap().port, None);

    // `ver` 是口径的显式版本：T-225 给 `turn_head` 加 `session.config` 时升到 2。
    assert_eq!(frozen.view("in_flight").unwrap().ver, 1);
    assert_eq!(frozen.view("turn_head").unwrap().ver, 2);
    assert_eq!(frozen.view("report_due").unwrap().ver, 1);
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
        json!({
            "turn": 1,
            "round": 0,
            "has_utterance": true,
            "classify_enabled": false,
            "done": true
        })
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
        json!({
            "turn": 2,
            "round": 1,
            "has_utterance": true,
            "classify_enabled": false,
            "done": false
        })
    );
}

#[test]
fn turn_head_is_empty_without_facts() {
    assert_eq!(turn_head(&[], 0), Value::Null);
}

#[test]
fn turn_head_reads_classify_switch_from_config_fact() {
    // 配置是**一轮一条事实**：判决读的是那一轮落下的快照，不是进程里的活值。
    let events = vec![
        ev_at(1, "session.config", json!({"classify_enabled": false}), 1),
        ev_at(1, "session.config", json!({"classify_enabled": true}), 2),
        ev_at(1, "user.message", json!({"text": "hi"}), 3),
    ];
    assert_eq!(
        turn_head(&events, 5)["classify_enabled"],
        json!(true),
        "同一轮多份配置时取最后一份（恢复幕会追加新快照）"
    );
}

// ── report_due：只用事实 + now ──────────────────────────────────────────

fn cfg(interval: i64, quota: u64) -> Value {
    json!({
        "progress_enabled": true,
        "progress_interval_ms": interval,
        "progress_max_per_turn": quota,
    })
}

#[test]
fn report_due_needs_log_and_now_only() {
    let events = vec![
        ev_at(1, "session.config", cfg(1000, 2), 10),
        ev_at(1, "user.message", json!({"text": "hi"}), 20),
    ];
    // 静默 1500ms > 1000ms，配额未用完 ⇒ 到期。
    assert_eq!(
        report_due(&events, 1520),
        json!({"turn": 1, "due": true, "reports": 0, "quota": 2})
    );
    // 同一份切片、只换 now：不到阈值 ⇒ 不到期。
    assert_eq!(
        report_due(&events, 1000),
        json!({"turn": 1, "due": false, "reports": 0, "quota": 2})
    );
}

#[test]
fn report_due_resets_quiet_on_assistant_speech() {
    let events = vec![
        ev_at(1, "session.config", cfg(1000, 2), 10),
        ev_at(1, "user.message", json!({"text": "hi"}), 20),
        ev_at(
            1,
            "chat.assistant.reported",
            json!({"text": "先报一句"}),
            900,
        ),
    ];
    // 最近一句助手文本在 900 ⇒ 静默 200ms，未到 1000ms。
    assert_eq!(
        report_due(&events, 1100),
        json!({"turn": 1, "due": false, "reports": 1, "quota": 2})
    );
}

#[test]
fn report_due_respects_quota_and_switch() {
    let events = vec![
        ev_at(1, "session.config", cfg(1000, 1), 10),
        ev_at(1, "user.message", json!({"text": "hi"}), 20),
        ev_at(1, "chat.assistant.reported", json!({"text": "报过"}), 30),
    ];
    // 配额 1 已用完 ⇒ 静默再久也不到期。
    assert_eq!(report_due(&events, 60_000)["due"], json!(false));

    // 开关关掉时其余量一律不作数。
    let off = vec![
        ev_at(
            1,
            "session.config",
            json!({
                "progress_enabled": false,
                "progress_interval_ms": 1,
                "progress_max_per_turn": 9,
            }),
            10,
        ),
        ev_at(1, "user.message", json!({"text": "hi"}), 20),
    ];
    assert_eq!(report_due(&off, 60_000)["due"], json!(false));
}

#[test]
fn report_due_is_empty_without_facts() {
    assert_eq!(report_due(&[], 1_000), Value::Null);
}

#[test]
fn views_are_pure_same_slice_same_now_same_value() {
    // ADR-115 决策 1：`now` 是入参。同一份事实切片求值两次逐字相同；
    // 不同 `now` 只通过比较影响 `due`——视图自己不读时钟。
    let events = vec![
        ev_at(1, "session.config", cfg(1000, 3), 10),
        ev_at(1, "user.message", json!({"text": "hi"}), 20),
        ev_at(
            1,
            "turn.stage",
            json!({"stage": "recall", "round": 0, "result": "hit"}),
            30,
        ),
    ];
    assert_eq!(report_due(&events, 5_000), report_due(&events, 5_000));
    assert_eq!(turn_head(&events, 5_000), turn_head(&events, 5_000));
    assert_eq!(report_due(&events, 5_000)["due"], json!(true));
    assert_eq!(report_due(&events, 500)["due"], json!(false));
}
