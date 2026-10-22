//! 行表与旧控制面的**逐行对拍**（T-222 判据 7 / 10）。
//!
//! 每个用例都从同一份日志切片出发：旧 `Registry::decide` 拿 `LoopState` 判，新
//! `owed` 拿视图判。**只差一个条件**的用例是主菜——只验「全满足时命中」在
//! 「恒返回 Some」的实现下也通过。
//!
//! 对拍的边界写在 `loop_rows` 模块头：汇报的 `round > 0 ∧ 工具轮数 ≥ min_rounds`
//! 是触发点的前提（日志里没有当前工具轮数那一格），本文件对的是「静默 + 配额」
//! 这一段；执行行是 T-222 的临时形态。

use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use super::super::chat_loop::ProgressPolicy;
use super::super::loop_registry::{builtin, LoopState};
use super::ROWS;
use crate::symbio_core::ring::{check_rows, drive, owed, Decl, Gap};
use crate::symbio_core::{builtin as declarations, Frozen};
use crate::symbio_core::{Entity, Event, Seq, Verb};

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

fn frozen() -> Arc<Frozen> {
    declarations().expect("内置声明必须能冻结")
}

/// 新表里这一行此刻是否可还（`owes` 命中且未被 `until` / `unless` 关掉）。
fn owes(name: &str, events: &[Event], now: i64) -> bool {
    owed(ROWS, &frozen(), events, now)
        .iter()
        .any(|d| d.decl.name == name && !d.blocked)
}

fn old(state: &LoopState) -> Option<&'static str> {
    builtin().decide(state).hit().map(|h| h.capsule.name())
}

fn copy(decl: &Decl) -> Decl {
    Decl {
        name: decl.name,
        prio: decl.prio,
        by: decl.by,
        tier: decl.tier,
        owes: decl.owes,
        until: decl.until,
        unless: decl.unless,
    }
}

/// 只开判决、其余策略关掉的配置。
fn plain() -> Value {
    json!({
        "classify_enabled": false,
        "auto_compress": false,
        "is_persistent": false,
        "progress_enabled": false,
        "progress_interval_ms": 0,
        "progress_min_rounds": 0,
        "progress_max_per_turn": 0,
    })
}

// ── 五条策略行逐条对拍 ───────────────────────────────────────────────────

#[test]
fn classify_row_matches_policy_classify() {
    let open = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(
            1,
            "session.config",
            json!({"classify_enabled": true, "auto_compress": false}),
        ),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    assert_eq!(
        old(&LoopState::turn_head(0, true, true, false)),
        Some("decide")
    );
    assert!(owes("policy.classify", &open, 0));

    // 没有发言（用户格 `id: null`）。
    let silent = vec![
        ev(1, "user.message", json!({"text": "", "id": null})),
        ev(
            1,
            "session.config",
            json!({"classify_enabled": true, "auto_compress": false}),
        ),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    assert_eq!(old(&LoopState::turn_head(0, false, true, false)), None);
    assert!(!owes("policy.classify", &silent, 0));

    // 开关关掉。
    let off = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(
            1,
            "session.config",
            json!({"classify_enabled": false, "auto_compress": false}),
        ),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    assert_eq!(old(&LoopState::turn_head(0, true, false, false)), None);
    assert!(!owes("policy.classify", &off, 0));

    // 不是轮首。
    let later = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(
            1,
            "session.config",
            json!({"classify_enabled": true, "auto_compress": false}),
        ),
        ev(1, "turn.stage", json!({"stage": "round", "round": 1})),
    ];
    assert_eq!(old(&LoopState::turn_head(1, true, true, false)), None);
    assert!(!owes("policy.classify", &later, 0));

    // 已经判过。
    let mut done = open.clone();
    done.push(ev(1, "turn.stage", json!({"stage": "decide", "round": 0})));
    assert_eq!(old(&LoopState::turn_head(0, true, true, true)), None);
    assert!(!owes("policy.classify", &done, 0));
}

#[test]
fn compress_row_matches_policy_compress() {
    let on = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(
            1,
            "session.config",
            json!({"classify_enabled": false, "auto_compress": true}),
        ),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    assert_eq!(old(&LoopState::compress(true, false)), Some("compress"));
    assert!(owes("policy.compress", &on, 0));

    let off = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(
            1,
            "session.config",
            json!({"classify_enabled": false, "auto_compress": false}),
        ),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    assert_eq!(old(&LoopState::compress(false, false)), None);
    assert!(!owes("policy.compress", &off, 0));

    let mut done = on.clone();
    done.push(ev(
        1,
        "turn.stage",
        json!({"stage": "compress", "round": 0}),
    ));
    assert_eq!(old(&LoopState::compress(true, true)), None);
    assert!(!owes("policy.compress", &done, 0));
}

#[test]
fn recall_row_matches_policy_recall() {
    let ready = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(
            1,
            "session.config",
            json!({"classify_enabled": false, "is_persistent": true}),
        ),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    assert_eq!(
        old(&LoopState::turn_prepare(0, true, false)),
        Some("recall")
    );
    assert!(owes("policy.recall", &ready, 0));

    // 临时会话：没有事实源。
    let transient = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(
            1,
            "session.config",
            json!({"classify_enabled": false, "is_persistent": false}),
        ),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    assert_eq!(old(&LoopState::turn_prepare(0, false, false)), None);
    assert!(!owes("policy.recall", &transient, 0));

    let mut done = ready.clone();
    done.push(ev(
        1,
        "turn.stage",
        json!({"stage": "recall", "round": 0, "result": "miss"}),
    ));
    assert_eq!(old(&LoopState::turn_prepare(0, true, true)), None);
    assert!(!owes("policy.recall", &done, 0));

    // 不是轮首。
    let later = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(
            1,
            "session.config",
            json!({"classify_enabled": false, "is_persistent": true}),
        ),
        ev(1, "turn.stage", json!({"stage": "round", "round": 1})),
    ];
    assert_eq!(old(&LoopState::turn_prepare(1, true, false)), None);
    assert!(!owes("policy.recall", &later, 0));
}

#[test]
fn reflex_row_matches_policy_reflex() {
    let hit = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(
            1,
            "session.config",
            json!({"classify_enabled": false, "is_persistent": true}),
        ),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
        ev(
            1,
            "turn.stage",
            json!({"stage": "recall", "round": 0, "result": "hit"}),
        ),
    ];
    assert_eq!(old(&LoopState::reflex(true, true)), Some("reflex"));
    assert!(owes("policy.reflex", &hit, 0));

    // 召回没命中。
    let miss = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(
            1,
            "session.config",
            json!({"classify_enabled": false, "is_persistent": true}),
        ),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
        ev(
            1,
            "turn.stage",
            json!({"stage": "recall", "round": 0, "result": "miss"}),
        ),
    ];
    assert_eq!(old(&LoopState::reflex(true, false)), None);
    assert!(!owes("policy.reflex", &miss, 0));

    // 续写幕：同一轮号、恢复格成为新幕锚 ⇒ 召回命中也不走快路。
    let mut resumed = hit.clone();
    resumed.push(ev(
        1,
        "turn.stage",
        json!({"stage": "resume", "round": 0, "result": "approve"}),
    ));
    resumed.push(ev(
        1,
        "session.config",
        json!({"classify_enabled": false, "is_persistent": true}),
    ));
    resumed.push(ev(1, "turn.stage", json!({"stage": "round", "round": 0})));
    assert_eq!(old(&LoopState::reflex(false, true)), None);
    assert!(!owes("policy.reflex", &resumed, 0));
}

fn policy(interval_ms: i64, max_per_turn: u32) -> ProgressPolicy {
    ProgressPolicy {
        enabled: true,
        interval_ms,
        min_rounds: 0,
        max_per_turn,
    }
}

#[test]
fn report_row_matches_policy_report_on_the_loggable_part() {
    let quiet = vec![
        ev_at(
            1,
            "session.config",
            json!({
                "progress_enabled": true,
                "progress_interval_ms": 1000,
                "progress_max_per_turn": 2,
            }),
            10,
        ),
        ev_at(1, "user.message", json!({"text": "hi", "id": "u-1"}), 20),
    ];
    assert_eq!(
        old(&LoopState::round_boundary(3, 60_000, 0, policy(1000, 2))),
        Some("report")
    );
    assert!(owes("policy.report", &quiet, 60_000));

    // 刚说过一句：旧侧静默归零，新侧静默锚前移。
    let mut spoke = quiet.clone();
    spoke.push(ev_at(
        1,
        "chat.assistant.reported",
        json!({"text": "先报一句"}),
        60_000,
    ));
    assert_eq!(
        old(&LoopState::round_boundary(3, 0, 1, policy(1000, 2))),
        None
    );
    assert!(!owes("policy.report", &spoke, 60_000));

    // 配额用完。
    let mut exhausted = quiet.clone();
    exhausted.push(ev_at(
        1,
        "chat.assistant.reported",
        json!({"text": "一"}),
        100,
    ));
    exhausted.push(ev_at(
        1,
        "chat.assistant.reported",
        json!({"text": "二"}),
        200,
    ));
    assert_eq!(
        old(&LoopState::round_boundary(3, 60_000, 2, policy(1000, 2))),
        None
    );
    assert!(!owes("policy.report", &exhausted, 60_000));

    // 开关关掉。
    let off = vec![
        ev_at(
            1,
            "session.config",
            json!({
                "progress_enabled": false,
                "progress_interval_ms": 1000,
                "progress_max_per_turn": 2,
            }),
            10,
        ),
        ev_at(1, "user.message", json!({"text": "hi", "id": "u-1"}), 20),
    ];
    let disabled = ProgressPolicy {
        enabled: false,
        ..policy(1000, 2)
    };
    assert_eq!(
        old(&LoopState::round_boundary(3, 60_000, 0, disabled)),
        None
    );
    assert!(!owes("policy.report", &off, 60_000));

    // 续写幕：旧侧配额随请求复位；新侧按幕锚重新数。
    let mut resumed = exhausted.clone();
    resumed.push(ev_at(
        1,
        "turn.stage",
        json!({"stage": "resume", "round": 0, "result": "approve"}),
        3_000,
    ));
    resumed.push(ev_at(
        1,
        "session.config",
        json!({
            "progress_enabled": true,
            "progress_interval_ms": 1000,
            "progress_max_per_turn": 2,
        }),
        3_001,
    ));
    assert_eq!(
        old(&LoopState::round_boundary(1, 2_000, 0, policy(1000, 2))),
        Some("report")
    );
    assert!(owes("policy.report", &resumed, 5_000));
}

// ── 全局裁决序 / 临时执行行 ──────────────────────────────────────────────

#[test]
fn execute_row_closes_on_turn_closed() {
    let open = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(1, "session.config", plain()),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    let picked = drive(ROWS, &frozen(), &open, 0).expect("开着的轮次总是欠一次执行");
    assert_eq!(picked.decl.name, "execute-turn");
    assert_eq!(picked.decl.by, Some("execute"));

    let mut closed = open.clone();
    closed.push(ev(1, "chat.assistant.final", json!({"text": "答完了"})));
    assert!(
        drive(ROWS, &frozen(), &closed, 0).is_none(),
        "收束后无账可还"
    );
}

#[test]
fn prio_is_global_and_registration_order_does_not_matter() {
    // 判决 / 召回 / 执行同时可还：`prio` 最小者胜，而不是表里先出现的那行。
    let events = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(
            1,
            "session.config",
            json!({
                "classify_enabled": true,
                "is_persistent": true,
                "auto_compress": false,
            }),
        ),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    let f = frozen();
    assert_eq!(
        drive(ROWS, &f, &events, 0).map(|d| d.decl.name),
        Some("policy.classify")
    );

    // 注册顺序反过来，结论不变。
    let mut reversed: Vec<Decl> = ROWS.iter().map(copy).collect();
    reversed.reverse();
    assert_eq!(
        drive(&reversed, &f, &events, 0).map(|d| d.decl.name),
        Some("policy.classify")
    );

    // 判过之后轮到召回（40），再之后才是执行（60）。
    let mut decided = events.clone();
    decided.push(ev(1, "turn.stage", json!({"stage": "decide", "round": 0})));
    assert_eq!(
        drive(ROWS, &f, &decided, 0).map(|d| d.decl.name),
        Some("policy.recall")
    );
    decided.push(ev(
        1,
        "turn.stage",
        json!({"stage": "recall", "round": 0, "result": "miss"}),
    ));
    assert_eq!(
        drive(ROWS, &f, &decided, 0).map(|d| d.decl.name),
        Some("execute-turn")
    );
}

// ── 表本身 ───────────────────────────────────────────────────────────────

#[test]
fn rows_are_stable_and_globally_ordered() {
    let names: Vec<_> = ROWS.iter().map(|d| d.name).collect();
    assert_eq!(
        names,
        vec![
            "policy.classify",
            "policy.report",
            "policy.compress",
            "policy.recall",
            "policy.reflex",
            "execute-turn",
        ]
    );
    assert!(
        ROWS.windows(2).all(|w| w[0].prio < w[1].prio),
        "`prio` 是全局裁决序：表里的先后必须严格递增"
    );
    assert_eq!(
        ROWS.iter().map(|d| d.tier).collect::<Vec<_>>(),
        vec![
            crate::symbio_core::LatencyTier::Fast,
            crate::symbio_core::LatencyTier::Reflex,
            crate::symbio_core::LatencyTier::Deep,
            crate::symbio_core::LatencyTier::Reflex,
            crate::symbio_core::LatencyTier::Reflex,
            crate::symbio_core::LatencyTier::Deep,
        ]
    );
}

#[test]
fn every_row_is_closed_and_every_used_source_declared() {
    let gaps = check_rows(ROWS, &frozen());
    assert!(
        gaps.iter().all(|g| matches!(g, Gap::OrphanFact { .. })),
        "行表自身的缺口（无出口 / 键不一致 / 源未声明）：{gaps:?}"
    );
}

#[test]
fn facts_without_a_row_are_pinned() {
    // 「每个声明的事实都有归宿」的缺口不能静默消失：下列每一条都是**已知**的
    // 后续任务，而不是遗漏。
    //
    //   request / sent / response / failed —— T-223 协议行（in_flight 的读方）
    //   turn.dispatch                     —— T-225 档位事实，读方是 core 的派生函数
    //   authz.denied                      —— T-226 授权拒绝行
    let gaps = check_rows(ROWS, &frozen());
    let orphans: Vec<_> = gaps
        .iter()
        .filter_map(|g| match g {
            Gap::OrphanFact { fact } => Some(*fact),
            _ => None,
        })
        .collect();
    assert_eq!(
        orphans,
        vec![
            "request",
            "sent",
            "response",
            "failed",
            "turn.dispatch",
            "authz.denied",
        ]
    );
}
