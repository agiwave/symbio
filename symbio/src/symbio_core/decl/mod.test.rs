//! 声明层的单元用例：装载 / 冻结 / 三道检查，以及两个首批视图的口径。
//!
//! 反向用例是主菜——一个只会让正确输入通过的白名单等于没有检查。

use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};

use super::super::event::{Entity, Verb};
use super::super::Seq;
use super::*;

/// 未入库事件默认 `seq = None`；视图的幕锚 / 阶段折叠读的是确切的序号，
/// 因此用例里的每条事件都按**调用顺序**分配一个递增序号（跨用例共享一个计数器
/// 不影响单个用例内的相对顺序）。
fn ev(turn: u64, kind: &str, payload: Value) -> Event {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let mut e = Event::pending(
        format!("{kind}-{turn}-{}", payload.to_string().len()),
        kind,
        Entity::Turn,
        Verb::Asserted,
        turn,
        "test",
    )
    .with_payload(payload);
    e.seq = Some(Seq::new(NEXT.fetch_add(1, Ordering::Relaxed)));
    e
}

fn ev_at(turn: u64, kind: &str, payload: Value, ts: i64) -> Event {
    ev(turn, kind, payload).with_ts(ts)
}

fn seq(e: &Event) -> u64 {
    e.seq.unwrap().value()
}

// ── 装载 ─────────────────────────────────────────────────────────────────

#[test]
fn builtin_loads_and_freezes() {
    let frozen = builtin().expect("首批声明必须能冻结");
    let facts: Vec<_> = frozen.facts().iter().map(|f| f.name).collect();
    assert_eq!(
        facts,
        vec![
            EVENT_REQUEST,
            EVENT_SENT,
            EVENT_RESPONSE,
            EVENT_FAILED,
            EVENT_RETRY,
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
            "authz.granted",
            "turn.closing",
        ]
    );
    let views: Vec<_> = frozen.views().iter().map(|v| v.name).collect();
    assert_eq!(
        views,
        vec![
            "in_flight",
            "stale_sent",
            "turn_head",
            "turn_stages",
            "turn_closed",
            "report_due",
            "reported_anchors",
        ]
    );

    // 端口是载荷字段：协议族声明它，收束 / 阶段 / 用户 / 拒绝都不带。
    for name in [
        EVENT_REQUEST,
        EVENT_SENT,
        EVENT_RESPONSE,
        EVENT_FAILED,
        EVENT_RETRY,
    ] {
        assert_eq!(frozen.fact(name).unwrap().port, Some("port"), "{name}");
    }
    assert_eq!(frozen.fact("chat.assistant.final").unwrap().port, None);

    // `ver` 是口径的显式版本：T-222a 给 `turn_head` 加幕锚 / 技能命中时升到 3，
    // `report_due` 加 `quiet_ms` / `min_rounds` 时升到 3（静默区间身份是 v2）；
    // T-223 把 `retry` 加进 `in_flight` 的关账一侧 ⇒ 升到 2。
    assert_eq!(frozen.view("in_flight").unwrap().ver, 2);
    assert_eq!(frozen.view("stale_sent").unwrap().ver, 1);
    assert_eq!(frozen.view("turn_head").unwrap().ver, 3);
    assert_eq!(frozen.view("turn_stages").unwrap().ver, 1);
    assert_eq!(frozen.view("turn_closed").unwrap().ver, 1);
    assert_eq!(frozen.view("report_due").unwrap().ver, 3);
    assert_eq!(frozen.view("reported_anchors").unwrap().ver, 1);
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
        name: EVENT_REQUEST,
        port: Some("port"),
        key: &["id"],
        expand: &[],
        fields: &["port", "id"],
    });
    b.view(ViewDecl {
        name: "probe",
        ver: 1,
        reads: &[(EVENT_REQUEST, "attmpt")],
        cost: 1,
        eval: in_flight,
    });
    assert_eq!(
        b.seal().unwrap_err(),
        vec![CheckError::UndeclaredField {
            view: "probe",
            fact: EVENT_REQUEST,
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
        vec![CheckError::DuplicateFact {
            name: EVENT_REQUEST
        }]
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
        name: EVENT_REQUEST,
        ver: 1,
        reads: &[(EVENT_REQUEST, "id")],
        cost: 1,
        eval: in_flight,
    });
    assert_eq!(
        b.seal().unwrap_err(),
        vec![CheckError::NameCollision {
            name: EVENT_REQUEST
        }]
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
        reads: &[(EVENT_SENT, "id")],
        cost: 1,
        eval: in_flight,
    });
    let frozen = b.seal().unwrap();
    let one = vec![ev(
        1,
        EVENT_SENT,
        json!({"port": "turn", "id": "a", "attempt": 1}),
    )];
    assert!(frozen.eval("probe", &one, 0).is_some());
    let two = vec![
        one[0].clone(),
        ev(
            1,
            EVENT_SENT,
            json!({"port": "turn", "id": "b", "attempt": 1}),
        ),
    ];
    assert_eq!(frozen.eval("probe", &two, 0), None);
}

#[test]
fn in_flight_holds_sent_until_response_or_failure() {
    let events = vec![
        ev(
            1,
            EVENT_SENT,
            json!({"port": "turn", "id": "a", "attempt": 1}),
        ),
        ev(
            1,
            EVENT_SENT,
            json!({"port": "turn", "id": "b", "attempt": 1}),
        ),
        ev(
            1,
            EVENT_RESPONSE,
            json!({"port": "turn", "id": "a", "attempt": 1}),
        ),
        ev(
            2,
            EVENT_SENT,
            json!({"port": "turn", "id": "c", "attempt": 2}),
        ),
        ev(
            2,
            EVENT_FAILED,
            json!({"port": "turn", "id": "c", "why": "boom"}),
        ),
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
        ev(
            1,
            EVENT_SENT,
            json!({"port": "turn", "id": "a", "attempt": 1}),
        ),
        ev(
            1,
            EVENT_SENT,
            json!({"port": "turn", "id": "a", "attempt": 2}),
        ),
        ev(
            1,
            EVENT_RESPONSE,
            json!({"port": "turn", "id": "a", "attempt": 1}),
        ),
    ];
    assert_eq!(
        builtin().unwrap().eval("in_flight", &events, 0),
        Some(json!([{"id": "a", "port": "turn", "attempt": 2}]))
    );
}

#[test]
fn in_flight_drops_the_attempt_retry_voided() {
    // `retry` 只关它那一次尝试：没有它，被作废的那次会永远留在在途里。
    let events = vec![
        ev(
            1,
            EVENT_SENT,
            json!({"port": "turn", "id": "a", "attempt": 1}),
        ),
        ev(
            1,
            EVENT_RETRY,
            json!({"port": "turn", "id": "a", "attempt": 1}),
        ),
        ev(
            1,
            EVENT_SENT,
            json!({"port": "turn", "id": "a", "attempt": 2}),
        ),
    ];
    assert_eq!(
        builtin().unwrap().eval("in_flight", &events, 0),
        Some(json!([{"id": "a", "port": "turn", "attempt": 2}]))
    );
}

/// 一条带租约的派发事实（T-223 的 `sent` 形状）。
fn sent_at(id: &str, attempt: u64, lease_ms: u64, cap: u64, ts: i64) -> Event {
    ev_at(
        1,
        EVENT_SENT,
        json!({
            "port": "turn",
            "id": id,
            "attempt": attempt,
            "lease_ms": lease_ms,
            "attempt_cap": cap,
        }),
        ts,
    )
}

#[test]
fn stale_sent_folds_the_lease_and_the_attempt_cap() {
    let events = vec![sent_at("a", 1, 1000, 3, 1_000)];
    let at = |now: i64| builtin().unwrap().eval("stale_sent", &events, now).unwrap();

    // 租约未到：不算卡住，但它在途（有实例，`stale:false`）。
    assert_eq!(
        at(1_999),
        json!([{"port": "turn", "id": "a", "attempt": 1, "age_ms": 999,
               "stale": false, "exhausted": false}])
    );
    // 到期即卡住（>= 而不是 >：边界算到期）。
    assert_eq!(
        at(2_000),
        json!([{"port": "turn", "id": "a", "attempt": 1, "age_ms": 1000,
               "stale": true, "exhausted": false}])
    );

    // 尝试到顶：同一个实例上 `exhausted` 为真——到期但要判死，不是再重试。
    let events = vec![sent_at("a", 3, 1000, 3, 1_000)];
    assert_eq!(
        builtin().unwrap().eval("stale_sent", &events, 9_000),
        Some(
            json!([{"port": "turn", "id": "a", "attempt": 3, "age_ms": 8000,
                    "stale": true, "exhausted": true}])
        )
    );
}

#[test]
fn stale_sent_only_sees_calls_nobody_closed() {
    // 三个出口各关一条：被应答 / 被判死 / 被作废的都不该再算卡住。
    let events = vec![
        sent_at("answered", 1, 1000, 3, 0),
        ev_at(
            1,
            EVENT_RESPONSE,
            json!({"port": "turn", "id": "answered", "attempt": 1}),
            1,
        ),
        sent_at("dead", 1, 1000, 3, 0),
        ev_at(
            1,
            EVENT_FAILED,
            json!({"port": "turn", "id": "dead", "why": "x"}),
            1,
        ),
        sent_at("retried", 1, 1000, 3, 0),
        ev_at(
            1,
            EVENT_RETRY,
            json!({"port": "turn", "id": "retried", "attempt": 1}),
            1,
        ),
    ];
    assert_eq!(
        builtin().unwrap().eval("stale_sent", &events, 100_000),
        Some(json!([]))
    );
}

#[test]
fn stale_sent_never_expires_a_call_without_a_lease() {
    // 旧日志里的 `sent` 没有 `lease_ms`：0 = 没有租约，永不被判成卡住。
    let events = vec![ev(
        1,
        EVENT_SENT,
        json!({"port": "turn", "id": "a", "attempt": 1}),
    )];
    assert_eq!(
        builtin().unwrap().eval("stale_sent", &events, 1_000_000),
        Some(
            json!([{"port": "turn", "id": "a", "attempt": 1, "age_ms": 1_000_000,
                    "stale": false, "exhausted": false}])
        )
    );
}

fn flags(classify: bool, compress: bool, persistent: bool) -> Value {
    json!({
        "classify_enabled": classify,
        "auto_compress": compress,
        "is_persistent": persistent,
        "progress_enabled": false,
        "progress_interval_ms": 0,
        "progress_min_rounds": 0,
        "progress_max_per_turn": 0,
    })
}

#[test]
fn turn_head_reads_round_and_done_from_facts() {
    let events = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(1, "session.config", flags(true, false, true)),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
        ev(
            1,
            "turn.stage",
            json!({"stage": "recall", "round": 0, "result": "hit"}),
        ),
        ev(1, "turn.stage", json!({"stage": "decide", "round": 0})),
    ];
    assert_eq!(
        turn_head(&events, 0),
        json!({
            "turn": 1,
            "round": 0,
            "anchor": seq(&events[0]),
            "has_utterance": true,
            "resumed": false,
            "classify_enabled": true,
            "classify_done": true,
            "auto_compress": false,
            "compress_done": false,
            "is_persistent": true,
            "recall_done": true,
            "skill_hit": true
        })
    );

    // 下一轮的事实出现后，轮头跟着走；`done` 只认**本幕**的阶段格。
    let events = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
        ev(1, "turn.stage", json!({"stage": "decide", "round": 0})),
        ev(2, "user.message", json!({"text": "again", "id": "u-2"})),
        ev(2, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    let head = turn_head(&events, 0);
    assert_eq!(head["turn"], json!(2));
    assert_eq!(head["round"], json!(0));
    assert_eq!(head["anchor"], json!(seq(&events[3])));
    assert_eq!(head["has_utterance"], json!(true));
    assert_eq!(head["classify_done"], json!(false));
    assert_eq!(head["recall_done"], json!(false));
}

#[test]
fn turn_head_resume_drive_starts_from_the_resume_anchor() {
    // 续写幕复用同一轮号：首幕的 recall / decide / final 都不能算到它头上。
    let events = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(1, "session.config", flags(false, false, true)),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
        ev(
            1,
            "turn.stage",
            json!({"stage": "recall", "round": 0, "result": "hit"}),
        ),
        ev(1, "turn.stage", json!({"stage": "decide", "round": 0})),
        ev(1, "chat.assistant.final", json!({"text": "done"})),
        ev(
            1,
            "turn.stage",
            json!({"stage": "resume", "round": 0, "result": "approve"}),
        ),
        ev(1, "session.config", flags(true, true, true)),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    let head = turn_head(&events, 0);
    assert_eq!(head["resumed"], json!(true));
    assert_eq!(head["has_utterance"], json!(false), "续写幕不是用户新发言");
    assert_eq!(head["anchor"], json!(seq(&events[6])));
    assert_eq!(head["classify_done"], json!(false));
    assert_eq!(head["recall_done"], json!(false));
    assert_eq!(head["skill_hit"], json!(false));
    assert_eq!(head["auto_compress"], json!(true), "配置按幕取最新一份");
    assert_eq!(turn_closed(&events, 0), json!([]), "首幕的终态不算本幕收束");

    let mut closed = events.clone();
    closed.push(ev(1, "chat.assistant.final", json!({"text": "resumed"})));
    assert_eq!(turn_closed(&closed, 0), json!([{"turn": 1}]));
}

#[test]
fn turn_stages_only_sees_the_current_drive() {
    let events = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
        ev(1, "turn.stage", json!({"stage": "recall", "round": 0})),
        ev(
            1,
            "turn.stage",
            json!({"stage": "resume", "round": 0, "result": "approve"}),
        ),
        ev(1, "session.config", flags(false, false, true)),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    assert_eq!(
        turn_stages(&events, 0),
        json!([
            {"turn": 1, "stage": "resume", "round": 0},
            {"turn": 1, "stage": "round", "round": 0},
        ])
    );
}

#[test]
fn turn_head_is_empty_without_facts() {
    assert_eq!(turn_head(&[], 0), Value::Null);
    assert_eq!(turn_stages(&[], 0), json!([]));
    assert_eq!(turn_closed(&[], 0), json!([]));
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
        "progress_min_rounds": 0,
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
        json!({
            "turn": 1,
            "anchor": seq(&events[1]),
            "due": true,
            "quiet_ms": 1500,
            "min_rounds": 0,
            "reports": 0,
            "quota": 2
        })
    );
    // 同一份切片、只换 now：不到阈值 ⇒ 不到期。
    assert_eq!(
        report_due(&events, 1000),
        json!({
            "turn": 1,
            "anchor": seq(&events[1]),
            "due": false,
            "quiet_ms": 980,
            "min_rounds": 0,
            "reports": 0,
            "quota": 2
        })
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
    // 最近一句助手文本在 900 ⇒ 静默 200ms，未到 1000ms；静默区间的身份跟着前移。
    assert_eq!(
        report_due(&events, 1100),
        json!({
            "turn": 1,
            "anchor": seq(&events[2]),
            "due": false,
            "quiet_ms": 200,
            "min_rounds": 0,
            "reports": 1,
            "quota": 2
        })
    );
}

#[test]
fn report_due_resets_quiet_on_resume() {
    // 审批 / 回填发生在请求开始时：静默从那一刻起算，而不是从几天前那句用户话。
    let events = vec![
        ev_at(1, "session.config", cfg(1000, 2), 10),
        ev_at(1, "user.message", json!({"text": "hi"}), 20),
        ev_at(
            1,
            "turn.stage",
            json!({"stage": "resume", "round": 0, "result": "approve"}),
            9_000,
        ),
    ];
    let due = report_due(&events, 9_500);
    assert_eq!(due["anchor"], json!(seq(&events[2])));
    assert_eq!(due["due"], json!(false));
}

#[test]
fn reported_anchors_close_one_quiet_interval_each() {
    // 每条汇报关掉它**之前**那段静默；汇报自己成为下一段的起点。
    let events = vec![
        ev_at(1, "session.config", cfg(1000, 3), 10),
        ev_at(1, "user.message", json!({"text": "hi"}), 20),
        ev_at(1, "chat.assistant.reported", json!({"text": "一"}), 900),
        ev_at(1, "chat.assistant.reported", json!({"text": "二"}), 1900),
    ];
    assert_eq!(
        reported_anchors(&events, 0),
        json!([
            {"turn": 1, "anchor": seq(&events[1])},
            {"turn": 1, "anchor": seq(&events[2])},
        ])
    );
    // 第二段已关；第三段自第二条汇报起算，到期时是一笔新账。
    let due = report_due(&events, 3_000);
    assert_eq!(due["anchor"], json!(seq(&events[3])));
    assert_eq!(due["due"], json!(true));
    assert_eq!(due["reports"], json!(2));
}

#[test]
fn reported_anchors_need_a_sequence() {
    // 未入库的汇报不是事实：没有 seq 就没有可关的区间。
    let mut report = ev(1, "chat.assistant.reported", json!({"text": "x"}));
    report.seq = None;
    assert_eq!(reported_anchors(&[report], 0), json!([]));
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
fn report_due_quota_is_per_drive() {
    // 配额是请求作用域的量：续写幕重新开账，首幕报过不影响本幕的额度。
    let events = vec![
        ev_at(1, "session.config", cfg(1000, 1), 10),
        ev_at(1, "user.message", json!({"text": "hi"}), 20),
        ev_at(1, "chat.assistant.reported", json!({"text": "报过"}), 30),
        ev_at(
            1,
            "turn.stage",
            json!({"stage": "resume", "round": 0, "result": "approve"}),
            9_000,
        ),
        ev_at(1, "session.config", cfg(1000, 1), 9_001),
    ];
    let due = report_due(&events, 10_500);
    assert_eq!(due["reports"], json!(0));
    assert_eq!(due["due"], json!(true));
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
