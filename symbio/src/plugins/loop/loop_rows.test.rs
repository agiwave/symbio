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
            // 收束的两行排最前（`prio` 0 / 5）：一轮走到收束点时 `execute-turn`
            // 仍然欠着，收束若排在它后面就永远轮不到（ADR-116）。
            "authorize-close",
            "close-turn",
            "policy.classify",
            "policy.report",
            "policy.compress",
            "policy.recall",
            "policy.reflex",
            "execute-turn",
            // 每个端口两行（`dispatch` / `await`）+ 一条管所有端口的租约行
            // （T-223 的 `turn` / T-227 步 2 的 `model` / 步 3 的 `tool`）。
            "dispatch-turn",
            "dispatch-model",
            "dispatch-tool",
            "await-turn",
            "await-model",
            "await-tool",
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
            // 授权是纯矩阵查表，收束是落格——两者都不调模型。
            crate::symbio_core::LatencyTier::Fast,
            crate::symbio_core::LatencyTier::Fast,
            crate::symbio_core::LatencyTier::Fast,
            crate::symbio_core::LatencyTier::Reflex,
            crate::symbio_core::LatencyTier::Deep,
            crate::symbio_core::LatencyTier::Reflex,
            crate::symbio_core::LatencyTier::Reflex,
            crate::symbio_core::LatencyTier::Deep,
            // 派发 / 看护是记账，不调模型；`await` 跟的是被等的那一端。
            crate::symbio_core::LatencyTier::Fast,
            crate::symbio_core::LatencyTier::Fast,
            crate::symbio_core::LatencyTier::Fast,
            crate::symbio_core::LatencyTier::Deep,
            crate::symbio_core::LatencyTier::Deep,
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
    //
    // 协议五名已由 T-223 的端口行全部接住（`request` / `sent` / `response` /
    // `failed` / `retry` → dispatch / await / lease）；授权族两名（`authz.denied` /
    // `authz.granted`）与 `turn.closing` 由 ADR-116 的收束两行接住。
    let gaps = check_rows(ROWS, &frozen());
    let orphans: Vec<_> = gaps
        .iter()
        .filter_map(|g| match g {
            Gap::OrphanFact { fact } => Some(*fact),
            _ => None,
        })
        .collect();
    assert_eq!(orphans, vec!["turn.dispatch"]);
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

// ── 端口 `model` 的两行（T-227 步 2）────────────────────────────────────
//
// 与 `turn` 的两行**同模板**（T-223 判据 1）：加一个端口 = 1 条 `PortDecl` +
// 2 行模板 + 0 条新行。下面的用例钉住「同模板」这件事本身——模板漂了
// （比如 `model` 的行少一个 `until` 分支），端口的账就永远关不掉。

fn model_request(id: &str) -> Event {
    ev(
        1,
        crate::symbio_core::EVENT_REQUEST,
        json!({ "port": "model", "id": id, "attempt": 1 }),
    )
}

fn model_sent(id: &str) -> Event {
    ev_at(
        1,
        crate::symbio_core::EVENT_SENT,
        json!({
            "port": "model",
            "id": id,
            "attempt": 1,
            "lease_ms": 600_000,
            "attempt_cap": 1,
        }),
        0,
    )
}

#[test]
fn a_model_request_opens_dispatch_and_sent_closes_it() {
    // 开账：`request` 落了、`sent` 还没落 ⇒ 欠一次模型派发。
    let asked = vec![model_request("m-1")];
    assert!(owes("dispatch-model", &asked, 0));
    // 关账：引擎落 `sent` 之后不再是派发的账，变成**等**的账（`by = None` ⇒ blocked）。
    let out = vec![model_request("m-1"), model_sent("m-1")];
    assert!(!owes("dispatch-model", &out, 0));
    let debts = owed(ROWS, &frozen(), &out, 0);
    let wait = debts
        .iter()
        .find(|d| d.decl.name == "await-model")
        .expect("等应答的行在");
    assert!(wait.blocked, "「已派出、等应答」就是 Wait 的折叠");
}

#[test]
fn a_model_failure_closes_both_the_dispatch_and_the_wait() {
    // `failed` 关的是逻辑 id（所有尝试）：派发行与等待行同时收口。
    let events = vec![
        model_request("m-1"),
        model_sent("m-1"),
        ev(
            1,
            crate::symbio_core::EVENT_FAILED,
            json!({ "port": "model", "id": "m-1", "why": "boom" }),
        ),
    ];
    assert!(!owes("dispatch-model", &events, 0));
    assert!(!owes("await-model", &events, 0));
}

#[test]
fn the_model_rows_do_not_disturb_the_turn_rows() {
    // 两个端口的行**互不串账**：一条 `turn` 的 `request` 不让 `model` 的派发行开账。
    let events = vec![request("t-1", 1)];
    assert!(owes("dispatch-turn", &events, 0));
    assert!(
        !owes("dispatch-model", &events, 0),
        "端口是债键的一部分：`turn` 的账不能开 `model` 的行"
    );
}

// ── 收束的授权级联（ADR-116）────────────────────────────────────────────
//
// 三格一条线：`turn.closing`（决策）→ `authz.granted` / `authz.denied`（结论）
// → 收束格。用例逐个钉住「谁开账、谁关账」，以及**否定不参与 `owes`**：
// `denied` 只关授权行，收束行根本不开账。

fn closing(turn: u64, key: &str) -> Event {
    ev(
        turn,
        crate::symbio_core::EVENT_TURN_CLOSING,
        json!({ "key": key, "kind": "speech" }),
    )
}

fn granted(turn: u64) -> Event {
    ev(
        turn,
        crate::symbio_core::EVENT_AUTHZ_GRANTED,
        json!({ "key": "chat.assistant.final", "kind": "speech" }),
    )
}

fn denied(turn: u64) -> Event {
    ev(
        turn,
        crate::symbio_core::EVENT_AUTHZ_DENIED,
        json!({ "key": "chat.assistant.final", "kind": "speech", "reason": "denied" }),
    )
}

#[test]
fn a_closure_intent_opens_the_authorization_row() {
    // 没有意图 ⇒ 不欠授权（六条收束路径之外，谁也不该被授权一次）。
    let quiet = vec![ev(1, "user.message", json!({"text": "hi", "id": "u-1"}))];
    assert!(!owes("authorize-close", &quiet, 0));

    // 意图落下 ⇒ 欠一次授权判定。
    let asked = vec![closing(1, "chat.assistant.final")];
    assert!(owes("authorize-close", &asked, 0));

    // 结论落下（两个方向都算关账）⇒ 不再欠。
    let mut yes = asked.clone();
    yes.push(granted(1));
    assert!(!owes("authorize-close", &yes, 0));
    let mut no = asked.clone();
    no.push(denied(1));
    assert!(!owes("authorize-close", &no, 0));
}

#[test]
fn only_a_grant_opens_the_close_row() {
    // 许可 ⇒ 欠一次收束。
    let yes = vec![closing(1, "chat.assistant.final"), granted(1)];
    assert!(owes("close-turn", &yes, 0));

    // 拒绝 ⇒ **收束行根本不欠**：这就是「被拒的一轮没有收束格」的结构形态，
    // 不是某个 `if` 的结论（判据 8b）。
    let no = vec![closing(1, "chat.assistant.final"), denied(1)];
    assert!(
        !owes("close-turn", &no, 0),
        "`denied` 不能让收束行开账——`owes` 里没有否定原子"
    );

    // 只有意图、还没有结论 ⇒ 也不欠（授权必须先于收束）。
    let pending = vec![closing(1, "chat.assistant.final")];
    assert!(!owes("close-turn", &pending, 0));
}

#[test]
fn a_closure_cell_pays_the_close_row() {
    // 两格收束都算还清：`final` 与 `fallback` 是同一件事的两个结果面。
    for kind in ["chat.assistant.final", "chat.assistant.fallback"] {
        let mut events = vec![closing(1, "chat.assistant.final"), granted(1)];
        assert!(owes("close-turn", &events, 0));
        events.push(ev(1, kind, json!({"text": "答完了"})));
        assert!(!owes("close-turn", &events, 0), "{kind} 应当还清收束行");
    }
}

#[test]
fn the_authorization_rows_do_not_disturb_the_policy_rows() {
    // 没有收束意图时，两条收束行对策略面完全无感：轮首该判的还是判。
    let events = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(
            1,
            "session.config",
            json!({"classify_enabled": true, "auto_compress": false}),
        ),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    assert_eq!(
        drive(ROWS, &frozen(), &events, 0).map(|d| d.decl.name),
        Some("policy.classify")
    );

    // 反过来：收束意图落下时，收束的两行**先于**执行行（`prio` 0 / 5 < 60）——
    // 否则单环驱动会先再执行一次而永远收不了尾。
    let mut closing_down = events.clone();
    closing_down.push(ev(1, "turn.stage", json!({"stage": "decide", "round": 0})));
    closing_down.push(closing(1, "chat.assistant.final"));
    assert_eq!(
        drive(ROWS, &frozen(), &closing_down, 0).map(|d| d.decl.name),
        Some("authorize-close")
    );
    closing_down.push(granted(1));
    assert_eq!(
        drive(ROWS, &frozen(), &closing_down, 0).map(|d| d.decl.name),
        Some("close-turn")
    );
}

/// **判据 3（T-227）**：只给日志，`in_flight` 能说出**哪一次调用**还在等——
/// 粒度是调用，不是一轮。
///
/// ## 它防的是什么
///
/// 「一轮在途」是个太粗的说法：同一轮里可以有两次模型往返与一批工具，其中
/// 只有一次没回来。若视图按轮折叠（「这一轮有 `sent` 就算在途」），恢复时会
/// 把**已经回来的**那些也算成没回来——重跑它们就是重复执行副作用。
///
/// ## 为什么这条用例只喂日志
///
/// 恢复的唯一输入是日志（N2：恢复 = 重放）。用例里不构造任何执行体状态，
/// 直接拿一串 `request` / `sent` / `response` 问视图——视图说得出，恢复就
/// 说得出；视图说不出，恢复就只能猜。
#[test]
fn in_flight_rebuilds_per_call_not_per_turn() {
    let sent = |port: &str, id: &str| {
        ev(
            1,
            "sent",
            json!({
                "port": port,
                "id": id,
                "attempt": 1,
                "lease_ms": 600_000,
                "attempt_cap": 1,
            }),
        )
    };
    let answered =
        |port: &str, id: &str| ev(1, "response", json!({"port": port, "id": id, "attempt": 1}));

    let events = vec![
        // 第一次模型往返：回来了。
        sent("model", "model-1-d1-s0"),
        answered("model", "model-1-d1-s0"),
        // 第二次模型往返：派出去了，没回来。
        sent("model", "model-1-d1-s1"),
        // 一批工具：派出去了，没回来。
        sent("tool", "tool-1-d1-b0"),
    ];

    let view = super::view("in_flight", &events, 1, 0).expect("视图已声明");
    let waiting = view.as_array().expect("在途是一张表");
    let mut got: Vec<(String, String)> = waiting
        .iter()
        .map(|v| {
            (
                v["port"].as_str().unwrap_or_default().to_string(),
                v["id"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    got.sort();

    assert_eq!(
        got,
        vec![
            ("model".to_string(), "model-1-d1-s1".to_string()),
            ("tool".to_string(), "tool-1-d1-b0".to_string()),
        ],
        "在途必须逐次调用点得出：回来过的 s0 不许留在表里"
    );

    // 反向自检：把 `response` 那一格删掉，三条就都在途——那时本用例的第一条
    // 断言会红，而不是仍然通过。这条自检说明「逐调用」不是靠数数凑出来的。
    let all_open = vec![
        sent("model", "model-1-d1-s0"),
        sent("model", "model-1-d1-s1"),
        sent("tool", "tool-1-d1-b0"),
    ];
    assert_eq!(
        super::view("in_flight", &all_open, 1, 0)
            .and_then(|v| v.as_array().map(|a| a.len()))
            .unwrap_or_default(),
        3
    );
}

// ── 本幕欠不欠一次对外调用（T-227 步 5a）────────────────────────────────
//
// 两个视图（`turn_model_due` / `turn_tools_due`）把「下一步调模型还是调工具」从
// 内存里的响应搬到事实面。下面两组用例钉的是**同一条弧**：一次往返 → 一批工具 →
// 再一次往返 → 定稿。每一步都只问视图，不问任何内存状态。

/// 视图口径的「欠」（与 `owes` 同形，但问的是视图而不是行）。
fn due(name: &str, events: &[Event]) -> bool {
    matches!(
        super::view(name, events, 1, 0),
        Some(Value::Array(ref a)) if !a.is_empty()
    )
}

fn user_said_hi() -> Vec<Event> {
    vec![
        ev(
            1,
            "user.message",
            json!({"text": "帮我查一下", "id": "u-1"}),
        ),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ]
}

fn model_answered(wants_tools: bool) -> Event {
    ev(
        1,
        "response",
        json!({
            "port": "model",
            "id": "model-1-d1-s0",
            "attempt": 1,
            "has_tool_calls": wants_tools,
        }),
    )
}

fn model_went_out(step: u64) -> Event {
    ev(
        1,
        "sent",
        json!({
            "port": "model",
            "id": format!("model-1-d1-s{step}"),
            "attempt": 1,
            "lease_ms": 600_000,
            "attempt_cap": 1,
        }),
    )
}

fn tool_went_out(batch: u64) -> Event {
    ev(
        1,
        "sent",
        json!({
            "port": "tool",
            "id": format!("tool-1-d1-b{batch}"),
            "attempt": 1,
            "lease_ms": 600_000,
            "attempt_cap": 1,
        }),
    )
}

fn tool_answered(batch: u64) -> Event {
    ev(
        1,
        "response",
        json!({
            "port": "tool",
            "id": format!("tool-1-d1-b{batch}"),
            "attempt": 1,
        }),
    )
}

/// 模型往返的那条弧：没有应答 ⇒ 欠；要工具 ⇒ 先欠批、不欠往返；批回来 ⇒ 又欠
/// 往返；定稿 ⇒ 不欠；收束 ⇒ 不欠。
#[test]
fn a_turn_owes_a_model_round_trip_until_the_model_gives_a_final_answer() {
    let mut events = user_said_hi();
    assert!(due("turn_model_due", &events), "还没有应答 ⇒ 欠一次往返");
    assert!(
        !due("turn_tools_due", &events),
        "没有应答就没有批可跑——不许凭空派发"
    );

    // 派出去了、还没回来 ⇒ 不欠（一次往返还没回来时不许发第二次）。
    events.push(model_went_out(0));
    assert!(!due("turn_model_due", &events), "在途时不许再发一次");
    events.pop();

    // 应答要工具 ⇒ 往返不欠（欠的是批）。
    events.push(model_answered(true));
    assert!(
        !due("turn_model_due", &events),
        "要工具的应答不是定稿：此刻欠的是批，不是又一次往返"
    );
    assert!(due("turn_tools_due", &events), "要工具 ⇒ 欠一批");

    // 批回来了 ⇒ 又欠一次往返。
    events.push(tool_went_out(0));
    events.push(tool_answered(0));
    assert!(due("turn_model_due", &events), "批回来了 ⇒ 该再问模型一次");
    assert!(!due("turn_tools_due", &events), "批跑完了 ⇒ 不欠批");

    // 第二次应答不要工具 ⇒ 定稿。
    events.push(model_went_out(1));
    events.push(ev(
        1,
        "response",
        json!({
            "port": "model",
            "id": "model-1-d1-s1",
            "attempt": 1,
            "has_tool_calls": false,
        }),
    ));
    assert!(
        !due("turn_model_due", &events),
        "不带工具的应答 = 定稿，交给收束行"
    );

    // 收束之后两条都不欠（`until` 之外的第二个出口：视图自己看得见收束格）。
    events.push(ev(1, "chat.assistant.final", json!({"text": "查到了"})));
    assert!(!due("turn_model_due", &events));
    assert!(!due("turn_tools_due", &events));
}

/// 工具批的那条弧：只有「末次应答要工具 ∧ 还没跑 ∧ 没有在途批」三条同时成立才欠。
#[test]
fn a_turn_owes_a_tool_batch_only_while_the_answer_wants_one() {
    // 应答不要工具 ⇒ 不欠。
    let mut events = user_said_hi();
    events.push(model_answered(false));
    assert!(!due("turn_tools_due", &events), "定稿没有批");

    // 要工具但批已经在途 ⇒ 不欠（否则会把同一批跑两遍，副作用重复）。
    let mut events = user_said_hi();
    events.push(model_answered(true));
    assert!(due("turn_tools_due", &events));
    events.push(tool_went_out(0));
    assert!(
        !due("turn_tools_due", &events),
        "批在途时不许再派一批——那不是重试，是第二次执行"
    );

    // 批回来了 ⇒ 不欠（`wants` 与 `done` 对上了）。
    events.push(tool_answered(0));
    assert!(!due("turn_tools_due", &events));

    // 反向自检：把 `has_tool_calls` 那一格删掉，视图读成「不要工具」⇒ 不欠。
    // 这条自检钉的是「缺失读成 false」的口径——猜 `true` 会在恢复时凭空跑一批工具。
    let mut no_field = user_said_hi();
    no_field.push(ev(
        1,
        "response",
        json!({"port": "model", "id": "model-1-d1-s0", "attempt": 1}),
    ));
    assert!(
        !due("turn_tools_due", &no_field),
        "没有 `has_tool_calls` 的旧日志读成「没有工具调用」，不是「有」"
    );
}
