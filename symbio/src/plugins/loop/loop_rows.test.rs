//! 行表的判据（T-222 判据 7 / 10）。
//!
//! 每个用例都从同一份日志切片出发，只改**一个**条件：只验「全满足时命中」在
//! 「恒返回 true」的实现下也通过。接线之后旧 `LoopState` 已不存在，这里对的是
//! 声明本身——每一行的 `owes` / `until` 与它读的视图字段。
//!
//! 汇报的 `round > 0 ∧ 工具轮数 ≥ min_rounds` 是触发点的前提（日志里没有当前
//! 工具轮数那一格），本文件对的是「静默 + 配额」这一段；执行行是 T-222 的临时形态。

use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

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

/// 全部策略关掉的配置。
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

// ── 五条策略行逐条判据 ───────────────────────────────────────────────────

#[test]
fn classify_row_owes_only_at_turn_head() {
    let open = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(
            1,
            "session.config",
            json!({"classify_enabled": true, "auto_compress": false}),
        ),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
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
    assert!(!owes("policy.classify", &later, 0));

    // 已经判过。
    let mut done = open.clone();
    done.push(ev(1, "turn.stage", json!({"stage": "decide", "round": 0})));
    assert!(!owes("policy.classify", &done, 0));
}

#[test]
fn compress_row_owes_while_switch_is_on_and_not_done() {
    let on = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(
            1,
            "session.config",
            json!({"classify_enabled": false, "auto_compress": true}),
        ),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
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
    assert!(!owes("policy.compress", &off, 0));

    let mut done = on.clone();
    done.push(ev(
        1,
        "turn.stage",
        json!({"stage": "compress", "round": 0}),
    ));
    assert!(!owes("policy.compress", &done, 0));
}

#[test]
fn recall_row_owes_only_at_turn_head_with_a_persistent_store() {
    let ready = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(
            1,
            "session.config",
            json!({"classify_enabled": false, "is_persistent": true}),
        ),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
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
    assert!(!owes("policy.recall", &transient, 0));

    let mut done = ready.clone();
    done.push(ev(
        1,
        "turn.stage",
        json!({"stage": "recall", "round": 0, "result": "miss"}),
    ));
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
    assert!(!owes("policy.recall", &later, 0));
}

#[test]
fn reflex_row_owes_on_a_recall_hit_outside_resume() {
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
    assert!(!owes("policy.reflex", &resumed, 0));

    // 收束后关账。
    let mut closed = hit.clone();
    closed.push(ev(1, "chat.assistant.final", json!({"text": "答完了"})));
    assert!(!owes("policy.reflex", &closed, 0));
}

#[test]
fn report_row_owes_on_a_quiet_interval_with_quota_left() {
    let quiet = vec![
        ev_at(
            1,
            "session.config",
            json!({
                "progress_enabled": true,
                "progress_interval_ms": 1000,
                "progress_min_rounds": 0,
                "progress_max_per_turn": 2,
            }),
            10,
        ),
        ev_at(1, "user.message", json!({"text": "hi", "id": "u-1"}), 20),
    ];
    assert!(owes("policy.report", &quiet, 60_000));

    // 静默不够久。
    assert!(!owes("policy.report", &quiet, 500));

    // 刚说过一句：静默锚前移到那句。
    let mut spoke = quiet.clone();
    spoke.push(ev_at(
        1,
        "chat.assistant.reported",
        json!({"text": "先报一句"}),
        60_000,
    ));
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
    assert!(!owes("policy.report", &exhausted, 60_000));

    // 开关关掉。
    let off = vec![
        ev_at(
            1,
            "session.config",
            json!({
                "progress_enabled": false,
                "progress_interval_ms": 1000,
                "progress_min_rounds": 0,
                "progress_max_per_turn": 2,
            }),
            10,
        ),
        ev_at(1, "user.message", json!({"text": "hi", "id": "u-1"}), 20),
    ];
    assert!(!owes("policy.report", &off, 60_000));

    // 续写幕：配额随幕重新开账，首幕报过不影响本幕。
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
            "progress_min_rounds": 0,
            "progress_max_per_turn": 2,
        }),
        3_001,
    ));
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
            // 端口 `turn` 的两行主模板 + 一条管所有端口的租约行（T-223）。
            "dispatch-turn",
            "await-turn",
            "lease",
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
            // 派发 / 看护是记账，不调模型；`await` 跟的是被等的那一端。
            crate::symbio_core::LatencyTier::Fast,
            crate::symbio_core::LatencyTier::Deep,
            crate::symbio_core::LatencyTier::Fast,
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
    //   turn.dispatch —— T-225 档位事实，读方是 core 的派生函数
    //   authz.denied  —— T-226 授权拒绝行
    //
    // 协议五名已由 T-223 的端口行全部接住（`request` / `sent` / `response` /
    // `failed` / `retry` → dispatch / await / lease）。
    let gaps = check_rows(ROWS, &frozen());
    let orphans: Vec<_> = gaps
        .iter()
        .filter_map(|g| match g {
            Gap::OrphanFact { fact } => Some(*fact),
            _ => None,
        })
        .collect();
    assert_eq!(orphans, vec!["turn.dispatch", "authz.denied"]);
}

// ── 端口 `turn` 的三行（T-223）─────────────────────────────────────────

fn request(id: &str, attempt: u64) -> Event {
    ev(
        1,
        crate::symbio_core::EVENT_REQUEST,
        json!({ "port": "turn", "id": id, "attempt": attempt }),
    )
}

fn sent(id: &str, attempt: u64, lease_ms: u64, cap: u64, ts: i64) -> Event {
    ev_at(
        1,
        crate::symbio_core::EVENT_SENT,
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
fn a_request_opens_dispatch_and_sent_closes_it() {
    // 开账：`request` 落了、`sent` 还没落 ⇒ 欠一次派发。
    let asked = vec![request("t-1", 1)];
    assert!(owes("dispatch-turn", &asked, 0));
    // 关账：引擎落 `sent` 之后不再是派发的账。
    let out = vec![request("t-1", 1), sent("t-1", 1, 1_000, 1, 0)];
    assert!(!owes("dispatch-turn", &out, 0));
    // 但它变成**等**的账：`await-turn` 在（且是 blocked：`by = None`）。
    let debts = owed(ROWS, &frozen(), &out, 0);
    let wait = debts.iter().find(|d| d.decl.name == "await-turn").unwrap();
    assert!(wait.blocked, "「已派出、等应答」就是 Wait 的折叠");
}

#[test]
fn a_failure_closes_both_the_dispatch_and_the_wait() {
    // `failed` 关的是逻辑 id（所有尝试）：派发行与等待行同时收口。
    let events = vec![
        request("t-1", 1),
        sent("t-1", 1, 1_000, 1, 0),
        ev(
            1,
            crate::symbio_core::EVENT_FAILED,
            json!({ "port": "turn", "id": "t-1", "why": "boom" }),
        ),
    ];
    assert!(!owes("dispatch-turn", &events, 0));
    assert!(!owes("await-turn", &events, 0));
}

#[test]
fn the_lease_row_opens_only_after_the_deadline() {
    // 时间不进欠账公式：租约行只对 `stale_sent.stale` 开账。
    let events = vec![sent("t-1", 1, 1_000, 1, 0)];
    assert!(!owes("lease", &events, 999));
    assert!(owes("lease", &events, 1_000));

    // 应答回来之后，哪怕时间继续走也不欠了。
    let mut answered = events.clone();
    answered.push(ev(
        1,
        crate::symbio_core::EVENT_RESPONSE,
        json!({ "port": "turn", "id": "t-1", "attempt": 1 }),
    ));
    assert!(!owes("lease", &answered, 100_000));
}

#[test]
fn the_lease_row_is_one_row_for_every_port() {
    // 第三行不按端口写：它读 `stale_sent` 的实例，端口在债键里。
    // 少了这一条性质，加端口就要加租约行，判据 1 的「0 个新行」就成了假话。
    let leases: Vec<_> = ROWS
        .iter()
        .filter(|d| d.name.starts_with("lease"))
        .collect();
    assert_eq!(leases.len(), 1);
    assert_eq!(leases[0].name, "lease");
    assert_eq!(leases[0].owes.source, "stale_sent");
}
