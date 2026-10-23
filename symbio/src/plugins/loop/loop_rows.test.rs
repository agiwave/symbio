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

use super::{PORTS, ROWS};
use crate::symbio_core::ring::{check_rows, drive, owed, Decl, Gap, Pattern};
use crate::symbio_core::Frozen;
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

/// 生产装载路径：六个注册口 + `seal`（声明集、执行体、端口、行一起进表）。
/// 用例拿到的就是会话开幕时那一张表——不是「先冻结声明、行自己想起来再查」。
fn frozen() -> Arc<Frozen> {
    super::load(ROWS).expect("生产行表必须装载得起来，否则义务环拒绝启动")
}

/// 这一行此刻是否**欠着**这笔账（`open`：`owes` 命中且没有被 `until` 关掉）。
///
/// 不看 `blocked`：「在等」与「没事」必须可区分（PL-016 §2.2），而 `by = None`
/// 的未接线行是**在等**，不是没欠。要验可还性请用 `drive` / `next` 或直读 `blocked`。
fn owes(name: &str, events: &[Event], now: i64) -> bool {
    owed(ROWS, &frozen(), events, now)
        .iter()
        .any(|d| d.decl.name == name)
}

fn copy(decl: &Decl) -> Decl {
    Decl {
        name: decl.name,
        prio: decl.prio,
        by: decl.by,
        tier: decl.tier,
        expand: decl.expand,
        owes: decl.owes,
        until: decl.until,
        unless: decl.unless,
    }
}

/// 把指定几行接到真实动作上，其余照抄整张表。
///
/// 只在验**裁决序**时用：未接线的行 `by = None` ⇒ `blocked`，`drive` 不选它。
/// 造出一张「同时可还」的表才能看出 `prio` 的相对顺序。
fn wired(names: &[&str]) -> Vec<Decl> {
    ROWS.iter()
        .map(|d| {
            let mut c = copy(d);
            if names.contains(&c.name) {
                c.by = Some(super::EXECUTE);
            }
            c
        })
        .collect()
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

// ── 端口声明 ─────────────────────────────────────────────────────────────

/// 批次并发是端口声明的一部分：只有 `tool` 允许同批并发；`turn` / `model`
/// 都是一次调用，没有「同批子债」。
#[test]
fn only_the_tool_port_declares_batch_concurrency() {
    let of = |port: &str| PORTS.iter().find(|p| p.port == port).map(|p| p.concurrent);
    assert_eq!(of("tool"), Some(true));
    assert_eq!(of("turn"), Some(false));
    assert_eq!(of("model"), Some(false));
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

/// 定稿答话轮（措辞产线出话）：**表认为该派发 `close-turn`，而产品路径压根不进驱动**。
///
/// 这条不对称是 [ADR-125](../../../../docs/010-adr/125.md "ADR-125") 决策 4 点名要写下来的：
/// 措辞往返的 `response(port=model)` 带 `calls: []` ⇒ `turn.has_final` 成立 ⇒ 收束行的前件
/// **成立**。但那一轮的答话是 `compose` 在轮首之前备好的（草稿），运行器一看 `final_reply`
/// 非空就在轮首准备里收束、`PreparedFlow::Done` 返回——驱动循环压根没起来，**表没被问**。
///
/// 为什么要钉一条「表说该派发、实际上不问」的用例：只看表的人会得出「它应该被派发」，
/// 然后去追一个不存在的 bug（T-253 的第一版判断就是这么来的）。把不对称钉在用例里，
/// 它就从「疑似漏派发」变成「已知且写明的边界」。
#[test]
fn a_drafted_turns_closing_row_is_owed_but_the_driver_never_asks() {
    let drafted = vec![
        ev(1, "user.message", json!({"text": "现在几点了", "id": "u-1"})),
        ev(1, "turn.stage", json!({"stage": "decide", "reason": "from_context"})),
        ev(
            1,
            "response",
            json!({"port": "model", "id": "model-0-d0-w0", "calls": []}),
        ),
        ev(1, "turn.dispatch", json!({})),
    ];
    let picked = drive(ROWS, &frozen(), &drafted, 0).expect("表里有可还的债");
    assert_eq!(
        picked.decl.name, "close-turn",
        "措辞往返的 calls 为空 ⇒ has_final 成立 ⇒ 收束行前件成立（表没问，但问的话就是这个答案）"
    );
    assert_eq!(picked.decl.by, Some("close"));
}

#[test]
fn prio_is_global_and_registration_order_does_not_matter() {
    // 判决 / 召回 / 执行同时可还：`prio` 最小者胜，而不是表里先出现的那行。
    //
    // 本用例验的是**裁决序**，不是接线面，所以把这三行都接到同一个动作上：
    // 未接线的行 `by = None` ⇒ `blocked`，会被 `drive` 滤掉（ADR-118 订正 1）。
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
    let rows = wired(&["policy.classify", "policy.recall", "execute-turn"]);
    assert_eq!(
        drive(&rows, &f, &events, 0).map(|d| d.decl.name),
        Some("policy.classify")
    );

    // 注册顺序反过来，结论不变。
    let mut reversed = rows.clone();
    reversed.reverse();
    assert_eq!(
        drive(&reversed, &f, &events, 0).map(|d| d.decl.name),
        Some("policy.classify")
    );

    // 判过之后轮到召回（40），再之后才是执行（60）。
    let mut decided = events.clone();
    decided.push(ev(1, "turn.stage", json!({"stage": "decide", "round": 0})));
    assert_eq!(
        drive(&rows, &f, &decided, 0).map(|d| d.decl.name),
        Some("policy.recall")
    );
    decided.push(ev(
        1,
        "turn.stage",
        json!({"stage": "recall", "round": 0, "result": "miss"}),
    ));
    assert_eq!(
        drive(&rows, &f, &decided, 0).map(|d| d.decl.name),
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
            "authorize-tool",
            "policy.classify",
            "policy.report",
            "policy.compress",
            "policy.recall",
            "policy.reflex",
            // 收束（T-244 步 5）：前件是 `turn.has_final`，执行体是 `close`。
            "close-turn",
            "execute-turn",
            // 每个端口两行（`dispatch` / `await`）+ 一条管所有端口的租约行
            // （T-223 的 `turn` / T-227 步 2 的 `model` / 步 3 的 `tool`）；
            // `open-tools` 是工具调用的创建行（ADR-117），不与派发合并。
            "dispatch-turn",
            "dispatch-model",
            "open-tools",
            "dispatch-tools",
            // 拒绝之后还得有人收尾（否则全被拒的一批永远跑不到 `failed`）。
            "deny-tools",
            "await-turn",
            "await-model",
            "await-tools",
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
            crate::symbio_core::LatencyTier::Fast,
            crate::symbio_core::LatencyTier::Reflex,
            crate::symbio_core::LatencyTier::Deep,
            crate::symbio_core::LatencyTier::Reflex,
            crate::symbio_core::LatencyTier::Reflex,
            // 收束入格不调模型：反射档（T-244 步 5）。
            crate::symbio_core::LatencyTier::Reflex,
            crate::symbio_core::LatencyTier::Deep,
            // 派发 / 看护是记账，不调模型；`await` 跟的是被等的那一端。
            crate::symbio_core::LatencyTier::Fast,
            crate::symbio_core::LatencyTier::Fast,
            crate::symbio_core::LatencyTier::Fast,
            crate::symbio_core::LatencyTier::Fast,
            // 拒绝收尾与派发同一执行体（不调工具）。
            crate::symbio_core::LatencyTier::Fast,
            crate::symbio_core::LatencyTier::Deep,
            crate::symbio_core::LatencyTier::Deep,
            crate::symbio_core::LatencyTier::Deep,
            crate::symbio_core::LatencyTier::Fast,
        ]
    );
}

// ── 装载期检查的反面夹具（T-239：装载失败即拒绝启动）──────────────────────
//
// 每个夹具只破一道检查，好让「拒绝装载」逐条有名：行名就是它破的那一格。

const fn pat(source: &'static str, key: &'static [&'static str]) -> Pattern {
    Pattern {
        source,
        key,
        eq: &[],
    }
}

const fn row(
    name: &'static str,
    owes: Pattern,
    until: &'static [Pattern],
    expand: Option<(&'static str, &'static str)>,
) -> Decl {
    row_by(name, owes, until, expand, "execute")
}

/// [`row`] 的另一半：`by` 可以指到别的执行体。
///
/// 只给「两行共享一个出口事实集」那类夹具用——一条真环需要两个**执行体**的写入集
/// 有交集（自环 exempt，单执行体造不出环）。生产表里 `execute` 与 `authorizer`
/// 都写 `authz.denied`，于是拿它们能造出真环。
const fn row_by(
    name: &'static str,
    owes: Pattern,
    until: &'static [Pattern],
    expand: Option<(&'static str, &'static str)>,
    by: &'static str,
) -> Decl {
    Decl {
        name,
        prio: 1,
        by: Some(by),
        tier: crate::symbio_core::LatencyTier::Fast,
        owes,
        expand,
        until,
        unless: &[],
    }
}

static CLOSED: &[Pattern] = &[pat("turn_closed", &[])];
static CLOSED_TURN: &[Pattern] = &[pat("turn_closed", &["turn"])];
static SENT: &[Pattern] = &[pat("sent", &[])];
static SENT_ID: &[Pattern] = &[pat("sent", &["id"])];

static GAP_DUP: &[Decl] = &[
    row("dup", pat("user.message", &[]), CLOSED, None),
    row("dup", pat("user.message", &[]), CLOSED, None),
];
static GAP_NO_EXIT: &[Decl] = &[row("no-exit", pat("user.message", &[]), &[], None)];
static GAP_KEY_MISMATCH: &[Decl] = &[row(
    "key-mismatch",
    pat("user.message", &["port", "id"]),
    CLOSED_TURN,
    None,
)];
static GAP_UNKNOWN_SOURCE: &[Decl] = &[row("unknown-source", pat("nope.fact", &[]), CLOSED, None)];
static GAP_EXPAND_KEY: &[Decl] = &[row(
    "expand-key-missing",
    pat("response", &[]),
    SENT,
    Some(("calls", "id")),
)];
static GAP_UNDECLARED_EXPAND: &[Decl] = &[row(
    "undeclared-expand",
    pat("response", &["id"]),
    SENT_ID,
    Some(("nope", "id")),
)];
// 第 3 条：键字段不在 `user.message` 的载荷声明里。它与「这次没带这个字段」同形，
// 所以运行期一声不吭——只能装载期红。
static GAP_UNDECLARED_FIELD: &[Decl] = &[row(
    "undeclared-field",
    pat("user.message", &["nosuch"]),
    CLOSED_TURN,
    None,
)];
// 第 1 条：`cycle-a` 的出口格由 `cycle-b` 的执行体写，反向也一样——两行互相等对方
// 跑一次。一格执行体不够造环（自环 exempt），两个**写入集有交集**的执行体才成环：
// 生产表里 `execute`（收束闸拒那条路）与 `authorizer`（工具授权拒那条路）都写
// `authz.denied`，于是 a 绑 `execute` 出口写 `authz.denied`、b 绑 `authorizer`
// 出口也写 `authz.denied`，边 `a → b`（b 的执行体写得出 a 的出口）与 `b → a`
// 同时成立。
static GAP_CYCLE: &[Decl] = &[
    row_by(
        "cycle-a",
        pat("user.message", &["turn"]),
        &[pat("authz.denied", &["turn"])],
        None,
        "execute",
    ),
    row_by(
        "cycle-b",
        pat("user.message", &["turn"]),
        &[pat("authz.denied", &["turn"])],
        None,
        "authorizer",
    ),
];

#[test]
fn the_production_table_loads_with_no_gap() {
    // 判据 7 / 11 的正面：生产行表 + 生产声明表**零缺口**。装载失败即拒绝启动，
    // 所以「零缺口」不是一条可以带过的告警，而是能开幕的唯一条件。
    // `load` 的 `Ok` 就是「§3.4 七道全过」——检查的出口只有一个，不再另跑一遍。
    super::load(ROWS).expect("行表装载必须无缺口，否则义务环拒绝启动");
}

#[test]
fn every_kind_of_load_gap_refuses_to_load() {
    // 反向用例：每种缺口各造一行，装载逐条点名并拒绝（不降级成 info / error 日志）。
    let cases: &[(&str, &[Decl], &str)] = &[
        ("重复行名", GAP_DUP, "DuplicateName"),
        ("无出口的行", GAP_NO_EXIT, "NoExit"),
        ("键不一致的行", GAP_KEY_MISMATCH, "KeyMismatch"),
        ("源没声明的行", GAP_UNKNOWN_SOURCE, "UnknownSource"),
        (
            "展开元素键不在 owes.key",
            GAP_EXPAND_KEY,
            "ExpandKeyMissing",
        ),
        (
            "展开字段未在事实声明",
            GAP_UNDECLARED_EXPAND,
            "UndeclaredExpand",
        ),
        ("声明了却没人消费的事实名", &[], "OrphanFact"),
        (
            "模式引用未声明的字段",
            GAP_UNDECLARED_FIELD,
            "UndeclaredPatternField",
        ),
        ("谁还清谁成环", GAP_CYCLE, "DependencyCycle"),
        // 端口侧同一条 fail-fast：夹具里没有 `await-*`，三个端口的 `response` /
        // `failed` 都无人还（真表靠那三行覆盖，见 `every_declared_port_has_its_answer_exits`）。
        ("端口没有应答出口", GAP_NO_EXIT, "PortWithoutAnswerExit"),
    ];

    for (label, rows, needle) in cases {
        let gaps = super::load(rows)
            .err()
            .unwrap_or_else(|| panic!("{label}：有缺口的行表必须拒绝装载"));
        assert!(
            gaps.iter().any(|g| format!("{g:?}").contains(needle)),
            "{label} 应当点名 {needle}，实际缺口：{gaps:?}"
        );
    }
}

#[test]
fn declared_facts_all_have_a_row_home() {
    // 「每个控制面声明的事实都有归宿」（判据 7）钉在这里：孤儿事实名会让装载
    // 直接拒绝，而它防的是「拼错的名字静默永不被消费」。
    //
    // `turn.dispatch` 因此**不在声明集**：它的读方是审计投影
    // （`invariants::declared_budget_ms`），表里没有一行以它为前件或出口——
    // 按口径它是业务事实，归投影管，不该挂进表里当孤儿。
    let frozen = frozen();
    let orphans: Vec<_> = check_rows(ROWS, &frozen)
        .iter()
        .filter_map(|g| match g {
            Gap::OrphanFact { fact } => Some(*fact),
            _ => None,
        })
        .collect();
    assert!(
        orphans.is_empty(),
        "声明集里仍有没人消费的事实名：{orphans:?}"
    );
    assert!(
        frozen.fact("turn.dispatch").is_none(),
        "turn.dispatch 不该在控制面声明集里（它没有行消费者）"
    );
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

// ── 工具端口与授权级联（T-227 步 3）────────────────────────────────────

/// ADR-117：模型响应的 `calls` 集合逐元素开**创建债**，不是一笔整批债。
#[test]
fn a_model_response_expands_its_tool_calls_into_one_debt_each() {
    let calls = json!([
        { "port": "tool", "id": "tool-1-d0-call-a", "name": "read_file", "arguments": {} },
        { "port": "tool", "id": "tool-1-d0-call-b", "name": "read_file", "arguments": {} },
    ]);
    let answered = vec![ev(
        1,
        crate::symbio_core::EVENT_RESPONSE,
        json!({ "port": "model", "id": "m-1", "attempt": 1, "calls": calls }),
    )];

    let debts: Vec<_> = owed(ROWS, &frozen(), &answered, 0)
        .into_iter()
        .filter(|d| d.decl.name == "open-tools")
        .collect();
    assert_eq!(debts.len(), 2, "两个调用 = 两笔创建债");
    // 两笔各自成键：展开的意义就是元素之间分得开（`blocked` 不看——`open-tools`
    // 是未接线行，`by = None` 恒在等，但账确实欠着两笔）。
    let keys: Vec<_> = debts.iter().map(|d| d.key.trace()).collect();
    assert_eq!(
        keys,
        vec![
            "port=tool,id=tool-1-d0-call-a",
            "port=tool,id=tool-1-d0-call-b"
        ]
    );

    let requested = vec![
        answered[0].clone(),
        ev(
            1,
            crate::symbio_core::EVENT_REQUEST,
            json!({ "port": "tool", "id": "tool-1-d0-call-a", "attempt": 1 }),
        ),
        ev(
            1,
            crate::symbio_core::EVENT_REQUEST,
            json!({ "port": "tool", "id": "tool-1-d0-call-b", "attempt": 1 }),
        ),
    ];
    assert!(
        !owes("open-tools", &requested, 0),
        "两笔 request 落完即关账"
    );

    // 空集合 = 不开债：模型答了话、没要工具。
    let no_calls = vec![ev(
        1,
        crate::symbio_core::EVENT_RESPONSE,
        json!({ "port": "model", "id": "m-2", "attempt": 1, "calls": [] }),
    )];
    assert!(!owes("open-tools", &no_calls, 0));
}

/// 本地模型常在每轮重新用 `call_0`：协议 id 必须带上「本次模型往返」，
/// 否则下一轮的创建债会被上一轮的 `request` 提前关掉。
#[test]
fn the_same_model_call_id_in_a_later_round_gets_a_distinct_protocol_id() {
    let first =
        super::super::tool_dispatch::batch_call_id(1, 0, 0, Some("call_0"), "model-1-d0-s1");
    let second =
        super::super::tool_dispatch::batch_call_id(1, 0, 0, Some("call_0"), "model-1-d0-s2");
    assert_ne!(first, second);
    assert_ne!(
        super::super::tool_dispatch::batch_call_id(1, 0, 0, None, "model-1-d0-s1"),
        super::super::tool_dispatch::batch_call_id(1, 0, 0, None, "model-1-d0-s2"),
        "缺少模型 id 时也不能撞键"
    );
}

fn tool_request(id: &str) -> Event {
    ev(
        1,
        crate::symbio_core::EVENT_REQUEST,
        json!({
            "port": "tool",
            "id": id,
            "attempt": 1,
            "tool": "read_file",
            "risk": "low",
            "threshold": "medium",
            "approved": false,
            "deferred": false,
        }),
    )
}

fn tool_granted(id: &str) -> Event {
    ev(
        1,
        crate::symbio_core::EVENT_AUTHZ_GRANTED,
        json!({
            "key": id,
            "port": "tool",
            "id": id,
            "tool": "read_file",
            "risk": "low",
            "deferred": false,
        }),
    )
}

fn tool_denied(id: &str) -> Event {
    ev(
        1,
        crate::symbio_core::EVENT_AUTHZ_DENIED,
        json!({
            "key": id,
            "port": "tool",
            "id": id,
            "tool": "write_file",
            "kind": "tool",
            "reason": "denied",
            "detail": "needs_approval",
        }),
    )
}

fn tool_sent(id: &str) -> Event {
    ev_at(
        1,
        crate::symbio_core::EVENT_SENT,
        json!({
            "port": "tool",
            "id": id,
            "attempt": 1,
            "lease_ms": 600_000,
            "attempt_cap": 1,
        }),
        0,
    )
}

#[test]
fn a_tool_request_opens_authorization_and_both_answers_close_it() {
    let asked = vec![tool_request("c-1")];
    assert!(
        owes("authorize-tool", &asked, 0),
        "请求落了、还没判 ⇒ 欠一次授权"
    );
    assert!(
        !owes("dispatch-tools", &asked, 0),
        "没放行 ⇒ 派发行不成立（fail-closed）"
    );

    let allowed = vec![tool_request("c-1"), tool_granted("c-1")];
    assert!(!owes("authorize-tool", &allowed, 0));
    assert!(owes("dispatch-tools", &allowed, 0));

    let refused = vec![tool_request("c-1"), tool_denied("c-1")];
    assert!(!owes("authorize-tool", &refused, 0));
    assert!(
        !owes("dispatch-tools", &refused, 0),
        "拒绝 ⇒ 没有 granted ⇒ 派发行永不成立"
    );
}

#[test]
fn a_denied_tool_call_owes_a_close_only() {
    // T-244 步 5b：`dispatch-tools` 只欠 `granted`——一批全被拒时它不欠，若不另立
    // 一行，`failed{why}` 永不落，`request` 那笔账也永远不关。
    let refused = vec![tool_request("c-1"), tool_denied("c-1")];
    assert!(
        owes("deny-tools", &refused, 0),
        "被拒的一笔仍欠一次收尾（落 `failed`）"
    );
    assert!(!owes("dispatch-tools", &refused, 0));

    let mut closed = refused.clone();
    closed.push(ev(
        1,
        crate::symbio_core::EVENT_FAILED,
        json!({ "port": "tool", "id": "c-1", "why": "denied" }),
    ));
    assert!(!owes("deny-tools", &closed, 0), "`failed` 关掉这一笔");

    // 收束格的拒绝（不带 `port`）不是端口面的拒绝：按 eq 取不到实例，不误开工具债。
    let closure = vec![ev(
        1,
        crate::symbio_core::EVENT_AUTHZ_DENIED,
        json!({ "kind": "speech", "key": "chat.assistant.final", "detail": "拒" }),
    )];
    assert!(!owes("deny-tools", &closure, 0));
}

#[test]
fn a_granted_tool_call_gets_one_sent_then_waits_for_its_answer() {
    let dispatched = vec![tool_request("c-1"), tool_granted("c-1"), tool_sent("c-1")];
    assert!(!owes("dispatch-tools", &dispatched, 0));
    let debts = owed(ROWS, &frozen(), &dispatched, 0);
    let wait = debts
        .iter()
        .find(|d| d.decl.name == "await-tools")
        .expect("等工具应答的行在");
    assert!(wait.blocked, "「已派出、等应答」就是 Wait 的折叠");

    let answered = {
        let mut e = dispatched.clone();
        e.push(ev(
            1,
            crate::symbio_core::EVENT_RESPONSE,
            json!({ "port": "tool", "id": "c-1", "attempt": 1 }),
        ));
        e
    };
    assert!(!owes("await-tools", &answered, 0));

    let failed = {
        let mut e = dispatched.clone();
        e.push(ev(
            1,
            crate::symbio_core::EVENT_FAILED,
            json!({ "port": "tool", "id": "c-1", "why": "not_executed" }),
        ));
        e
    };
    assert!(!owes("dispatch-tools", &failed, 0));
    assert!(!owes("await-tools", &failed, 0));
}

/// T-252 判据 1：**先把现状钉成用例**（裁决之前也得有人钉住它）。
///
/// 租约到顶重试落的是**同一 `(port, id)`** 的 `retry{attempt:1}` + `request{attempt:2}`
/// （`port.rs` 的 `reap`），而三行工具债的键是 `(port, id)`、不含 `attempt`
/// （理由记在 `loop_rows.rs` 的键口径注释：`failed` 要能一次关掉所有尝试）
/// ⇒ 上一 attempt 的 `granted` 已经把授权行的门关上，第二次尝试的那笔 `request`
/// 在行表里**不欠任何东西**：没有任何执行体会判它、派它。
/// 「抬 `max_attempts` 的前置是键口径，不是把派发行接回执行体」——那条订正的全部根据
/// 就是这一段切片。
#[test]
fn a_retried_request_is_invisible_to_the_row_table() {
    let id = "c-1";
    let mut sliced = vec![
        tool_request(id),
        tool_granted(id),
        tool_sent(id),
        ev(
            1,
            crate::symbio_core::EVENT_RETRY,
            json!({ "port": "tool", "id": id, "attempt": 1 }),
        ),
        ev(
            1,
            crate::symbio_core::EVENT_REQUEST,
            json!({
                "port": "tool",
                "id": id,
                "attempt": 2,
                "tool": "read_file",
                "risk": "low",
                "threshold": "medium",
                "approved": false,
                "deferred": false,
            }),
        ),
    ];
    assert!(
        !owes("authorize-tool", &sliced, 0),
        "第二次尝试的 `request` 不开新账：键 `(port, id)` 已被上一 attempt 的 `granted` 关掉"
    );
    assert!(
        !owes("dispatch-tools", &sliced, 0),
        "也就没有任何执行体会派它（PL-016 §2.3 禁止的「永不派发的 request」）"
    );

    // 派发侧同一件事，而且要能红：未接线的行 `blocked`，`drive` 谁都不选，
    // 那样这条断言只是空转——所以把两行接到真实动作上再验「取不到这笔账」。
    let f = frozen();
    let rows = wired(&["authorize-tool", "dispatch-tools"]);
    assert_eq!(
        drive(&rows, &f, &sliced, 0).map(|d| d.decl.name),
        None,
        "接回执行体也派不动重试出来的那一笔 ⇒ 抬 max_attempts 的前置在键上（T-252）"
    );

    // 反向自检：红点必须落在「`granted` 提前关门」这一格，而不是键本身写错——
    // 同一组事实里去掉那一格，第二次尝试的 `request` 就重新开账。
    sliced.retain(|e| e.kind != crate::symbio_core::EVENT_AUTHZ_GRANTED);
    assert_eq!(
        drive(&rows, &f, &sliced, 0).map(|d| d.decl.name),
        Some("authorize-tool"),
        "没有上一 attempt 的出口格时派发器就选中它 ⇒ 隐形来自键的口径，不是模式拼错"
    );
}

#[test]
fn a_resumed_tool_call_is_authorized_and_awaited_like_a_batch_call() {
    // 批准恢复的那一笔是同一套账：id 带 `resume` 只是为了让重试幂等，
    // 行层不认这个前缀（`(port, id)` 才是键）。
    let id = "tool-resume-1-tc-1-1";
    let open = vec![tool_request(id), tool_granted(id), tool_sent(id)];
    assert!(!owes("authorize-tool", &open, 0));
    assert!(!owes("dispatch-tools", &open, 0));
    let wait = owed(ROWS, &frozen(), &open, 0)
        .into_iter()
        .find(|d| d.decl.name == "await-tools")
        .expect("恢复的那一笔派出后同样在等应答");
    assert!(wait.blocked);

    let mut done = open;
    done.push(ev(
        1,
        crate::symbio_core::EVENT_RESPONSE,
        json!({ "port": "tool", "id": id, "attempt": 1 }),
    ));
    assert!(!owes("await-tools", &done, 0));
}

/// ADR-117：恢复调用的债前件是审批动作写下的 `request`，不是模型响应。
///
/// 这一条钉住「`dispatch-tools` 不欠模型响应」这个决定：事实切片里**没有**
/// `response(port=model)`，恢复的那一笔仍照常走「授权 → 派发」两行。
#[test]
fn a_resumed_tool_call_does_not_need_a_model_response() {
    let id = "tool-resume-1-tc-1-1";
    let events = vec![tool_request(id), tool_granted(id)];

    assert!(
        events
            .iter()
            .all(|e| e.kind != crate::symbio_core::EVENT_RESPONSE
                || e.payload.get("port").and_then(Value::as_str) != Some("model")),
        "用例前提：日志里没有模型响应"
    );
    assert!(!owes("authorize-tool", &events, 0), "已放行 ⇒ 授权行关账");
    assert!(
        owes("dispatch-tools", &events, 0),
        "恢复的那一笔照样欠派发：债前件是它自己的 request，与模型响应无关"
    );
}

#[test]
fn an_aborted_tool_closes_the_await_with_its_own_reason() {
    let events = vec![
        tool_request("c-1"),
        tool_granted("c-1"),
        tool_sent("c-1"),
        ev(
            1,
            crate::symbio_core::EVENT_FAILED,
            json!({ "port": "tool", "id": "c-1", "why": "aborted" }),
        ),
    ];
    assert!(!owes("dispatch-tools", &events, 0));
    assert!(!owes("await-tools", &events, 0));
    assert_eq!(
        events.last().and_then(|e| e.payload.get("why")),
        Some(&json!("aborted")),
        "中止与批次尾的 not_executed 必须可区分"
    );
}

#[test]
fn a_supplement_does_not_answer_the_in_flight_tool() {
    // 补充（用户插话）不是工具应答：它既不关派发行、也不关等待行。
    let events = vec![
        tool_request("c-1"),
        tool_granted("c-1"),
        tool_sent("c-1"),
        ev(1, "user.message", json!({ "text": "等一下", "id": "u-2" })),
    ];
    assert!(!owes("dispatch-tools", &events, 0));
    let wait = owed(ROWS, &frozen(), &events, 0)
        .into_iter()
        .find(|d| d.decl.name == "await-tools")
        .expect("工具仍未被应答");
    assert!(wait.blocked, "补充只改变下一轮 prompt，不是这次调用的应答");
}

#[test]
fn a_closure_denial_does_not_close_a_tool_authorization() {
    // 收束格拒绝的载荷只有目标格名（`key = chat.assistant.final`），没有
    // `port` / `id`：按 `(port, id)` 绑定取不到实例，不能误关一笔工具授权。
    let closure_denial = ev(
        1,
        crate::symbio_core::EVENT_AUTHZ_DENIED,
        json!({
            "key": "chat.assistant.final",
            "kind": "speech",
            "reason": "denied",
            "detail": "缺 reply.first",
        }),
    );
    let events = vec![tool_request("c-1"), closure_denial];
    assert!(owes("authorize-tool", &events, 0));
    assert!(!owes("dispatch-tools", &events, 0));
}

#[test]
fn tool_rows_do_not_disturb_the_model_rows() {
    // 授权级联只认 `port = tool`：一条模型的 `request` 不开授权行。
    let events = vec![model_request("m-1")];
    assert!(!owes("authorize-tool", &events, 0));
    assert!(!owes("dispatch-tools", &events, 0));
}

// ── 派发器的选债口（T-233）────────────────────────────────────────────

#[test]
fn next_is_none_without_events() {
    // 空表不前进：装载失败 / 还没有本幕事实时，派发器不能猜一个动作。
    assert!(super::next(&[], 0, 0, super::ACTORS).is_none());
}

#[test]
fn next_picks_execute_turn_while_the_turn_is_open() {
    // 开着的幕欠一次执行；`execute-turn` 是当前最小可还债（其它策略都关着）。
    let events = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    let next = super::next(&events, 1, 0, super::ACTORS).expect("开着的幕应欠 execute-turn");
    assert_eq!(next.row, "execute-turn");
    assert_eq!(next.by, "execute");
}

/// 一条空 `calls` 的模型应答（「答完了」那一格）。
fn model_answer(turn: u64, id: &str) -> Event {
    ev(
        turn,
        crate::symbio_core::EVENT_RESPONSE,
        json!({ "port": "model", "id": id, "attempt": 1, "calls": [] }),
    )
}

/// 收束行（T-244 步 5）：答完了就欠一次入格，且它比执行行**先**被选中（55 < 60）。
///
/// 这条裁决序是这一格存在的全部理由：日志上已经有「没有工具调用的应答」时，
/// 下一个动作必须是入格，而不是再跑一次模型往返。
#[test]
fn close_row_owes_only_after_an_answer_without_calls() {
    let open = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    assert!(!owes("close-turn", &open, 0), "还没有模型应答 ⇒ 不欠收束");

    let with_calls = vec![
        open[0].clone(),
        open[1].clone(),
        ev(
            1,
            crate::symbio_core::EVENT_RESPONSE,
            json!({
                "port": "model",
                "id": "m-1",
                "attempt": 1,
                "calls": [{ "port": "tool", "id": "c-1" }],
            }),
        ),
    ];
    assert!(
        !owes("close-turn", &with_calls, 0),
        "最近一条应答还带工具调用 ⇒ 该还的是工具批次"
    );

    let mut answered = with_calls[..2].to_vec();
    answered.push(model_answer(1, "m-2"));
    assert!(owes("close-turn", &answered, 0), "应答落地 ⇒ 欠一次入格");

    let mut landed = answered.clone();
    landed.push(ev(1, "chat.assistant.final", json!({ "text": "答完了" })));
    assert!(!owes("close-turn", &landed, 0), "`final` 命中出口 ⇒ 关门");

    // 兜底格也在出口里（步 5c 复原 PL-017 §3.1 的 `final | fallback`：两个执行体写的
    // 格不相交之后，环没了，出口可以照方案写全）⇒ 命中即关门。
    let mut fell_back = answered;
    fell_back.push(ev(
        1,
        "chat.assistant.fallback",
        json!({ "text": "", "status": "error" }),
    ));
    assert!(
        !owes("close-turn", &fell_back, 0),
        "兜底格落下 ⇒ 出口命中，这笔债关门，不留一笔永不兑现的债"
    );
}

/// 收束行的前件是 `turn.closeable`（T-244 步 5c）：**答完了**或**模型没答成**。
///
/// 后半条是中止 / 适配器失败那两条路唯一的进账依据：它们没有「`calls` 为空的
/// 模型应答」（答都没答成，哪来应答），`has_final` 因此不成立；而收束格只由
/// `close` 执行体写，行表得能从日志上判出「该收束了」。
#[test]
fn close_row_owes_when_the_model_round_failed_in_this_drive() {
    let open = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    assert!(!owes("close-turn", &open, 0), "还没派发模型往返 ⇒ 不欠收束");

    let mut failed = open.clone();
    failed.push(ev(
        1,
        crate::symbio_core::EVENT_REQUEST,
        json!({ "port": "model", "id": "m-1", "attempt": 1 }),
    ));
    failed.push(ev(
        1,
        crate::symbio_core::EVENT_FAILED,
        json!({ "port": "model", "id": "m-1", "why": "adapter error" }),
    ));
    assert!(
        owes("close-turn", &failed, 0),
        "模型没答成 ⇒ 欠一次收束（中止格 / 兜底格由它落）"
    );

    // 请求不在本幕（上一世那笔卡住的调用被看护者判死）⇒ 不是本幕的模型往返。
    let mut stale = open.clone();
    stale.push(ev(
        1,
        crate::symbio_core::EVENT_REQUEST,
        json!({ "port": "model", "id": "m-1", "attempt": 1 }),
    ));
    stale.push(ev(
        1,
        "turn.stage",
        json!({ "stage": "resume", "round": 0, "result": "approve" }),
    ));
    stale.push(ev(
        1,
        crate::symbio_core::EVENT_FAILED,
        json!({ "port": "model", "id": "m-1", "why": "lease_expired" }),
    ));
    assert!(
        !owes("close-turn", &stale, 0),
        "失败收的是上一世那笔调用 ⇒ 本幕仍欠一次模型往返，不是一次收束"
    );
}

#[test]
fn next_picks_close_turn_before_execute_turn_once_answered() {
    let events = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
        model_answer(1, "m-1"),
    ];
    let next = super::next(&events, 1, 0, super::ACTORS).expect("答完了的幕欠一次收束");
    assert_eq!(next.row, "close-turn");
    assert_eq!(next.by, super::CLOSE);
    let run: super::super::turn_exec::Action = super::super::turn_exec::run_close;
    assert!(std::ptr::fn_addr_eq(next.action, run));
}

#[test]
fn next_is_none_once_the_turn_is_closed() {
    // 收束格出现后，`execute-turn` 的出口命中；表里没有别的可还债。
    let events = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
        ev(1, "chat.assistant.final", json!({"text": "ok"})),
    ];
    assert!(super::next(&events, 1, 0, super::ACTORS).is_none());
}

/// 「能选中」与「有人跑」是同一件事的两面：`next` 交出的不只是行名，还有动作本体。
/// 派发器因此没有任何按行名分支的余地（ADR-118 决策 1–2）。
#[test]
fn next_hands_back_the_action_itself() {
    let events = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    let next = super::next(&events, 1, 0, super::ACTORS).expect("开着的幕欠 execute-turn");
    assert_eq!(next.row, "execute-turn");
    assert_eq!(next.by, super::EXECUTE);
    let run: super::super::turn_exec::Action = super::super::turn_exec::run_execute;
    // 函数指针的 `==` 不作数（同一函数在不同 codegen unit 里地址可不同）；
    // `fn_addr_eq` 是这条比较的唯一合法写法。
    assert!(std::ptr::fn_addr_eq(next.action, run));
}

/// 行表与绑定表同批冻结：两个方向都不许多——每个 `by` 查得到动作，
/// 每个注册过的执行体也有行指向它。这正是装载期 `UnmannedRow` / `OrphanActor`
/// 在产线上兑现出来的形状，这里对着真表再验一次。
#[test]
fn the_row_table_and_the_binding_table_agree() {
    let f = frozen();
    let mut used: Vec<&str> = Vec::new();
    for d in ROWS {
        let Some(by) = d.by else { continue };
        assert!(
            super::entry_of(by, super::ACTORS).is_some(),
            "行 {} 的 `by` 没有动作",
            d.name
        );
        assert!(f.actor(by).is_some(), "行 {} 的 `by` 没有声明", d.name);
        used.push(by);
    }
    for a in f.actors() {
        assert!(used.contains(&a.name), "执行体 {} 没有任何行指向它", a.name);
    }
}

/// 过渡豁免是一条**有名有姓、会自己变红**的名单（ADR-123 决策 2）。
///
/// 它放宽的是运行期写入封闭，每加一条都要回答「为什么这条执行体声明不了这一格」——
/// `execute` + `chat.assistant.fallback` 那一条的理由与删除条件记在
/// `ActorEntry::transitional` 的头注（步 5c 之后：收束格全部归 `close` 执行体，
/// 只剩两条**控制面**失败——它们在日志上没有模型的终局事实，收束行开不出账）。
/// 名单变长必须先过这条用例。
#[test]
fn the_transitional_exemptions_are_pinned() {
    let actual: Vec<(&str, &[&str])> = super::ACTORS
        .iter()
        .map(|e| (e.decl.name, e.transitional))
        .filter(|(_, list)| !list.is_empty())
        .collect();
    let expected: Vec<(&str, &[&str])> = vec![(super::EXECUTE, &["chat.assistant.fallback"])];
    assert_eq!(actual, expected, "过渡豁免名单变了——先补裁决，再改这条用例");
}

/// 运行期写入封闭的口径（ADR-123 决策 2 的那一句「协议格豁免，领域格必须声明」）。
///
/// 派发器读账本用的就是这一个谓词：这里钉住三类各一条，越界那一类必须为假——
/// 名单外的领域格正是这条判据要拦的事。
#[test]
fn writes_closed_admits_declared_protocol_and_exempted_only() {
    let events = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    let next = super::next(&events, 1, 0, super::ACTORS).expect("开着的幕欠 execute-turn");
    // 声明过的领域格。
    assert!(next.writes_closed("turn.dispatch"));
    // 协议五格：引擎写，本来就不许出现在 `writes` 里（装载期 `ENGINE_ONLY_CELLS` 拦）。
    assert!(next.writes_closed("response"));
    // 过渡豁免：确实写得出、装载期声明不了的那一格。
    assert!(next.writes_closed("chat.assistant.fallback"));
    // 收束格归 `close` 执行体（步 5c）⇒ `execute` 写它就是越界。
    assert!(
        !next.writes_closed("chat.assistant.final"),
        "收束格不在 `execute` 的名单里——两个执行体写同一批格会让依赖图成环"
    );
    // 名单外：越界。
    assert!(
        !next.writes_closed("memory.encoded"),
        "名单外的领域格必须判越界，否则对账是空转"
    );
}

// ── 本次 drive 的 `now`：只来自唤醒事实（判据 12 · T-240）──────────────────

/// 一幕的日志：唤醒格（`ts = wake_ts`）+ 上一世派出、没回来的 `sent`（租约 1000ms）。
///
/// 返回切片与**幕锚**——唤醒格的 seq，生产代码里它就是 `drive_seq`。
fn drive_with_stale_sent(wake_ts: i64) -> (Vec<Event>, u64) {
    let wake = ev_at(
        1,
        "user.message",
        json!({ "text": "hi", "id": "u-1" }),
        wake_ts,
    );
    let anchor = wake.seq.expect("测试里的事件都带号").value();
    let mut events = vec![wake, sent("t-1", 1, 1_000, 1, 0)];
    events.insert(1, ev_at(1, "session.config", plain(), wake_ts - 1));
    (events, anchor)
}

#[test]
fn the_drive_now_is_the_wake_fact_timestamp() {
    // 唤醒 ts 落在租约期内 ⇒ 表说不欠收租。同一份日志换一个更大的 `now`（墙钟会给
    // 出的那种量级）就变成欠——这条差异正是「时间从日志来」与「时间现读」的分界。
    let (events, anchor) = drive_with_stale_sent(500);
    assert_eq!(super::wake_now_ms(&events, anchor), 500);
    assert!(!owes("lease", &events, super::wake_now_ms(&events, anchor)));
    assert!(owes("lease", &events, 60_000));

    // 停机更久的一次唤醒：日志没变，只是那条事实的 ts 更晚 ⇒ 租约到期。
    let (later, later_anchor) = drive_with_stale_sent(1_500);
    assert!(owes(
        "lease",
        &later,
        super::wake_now_ms(&later, later_anchor)
    ));

    // 幕锚不在这份切片里 ⇒ 0，不猜一个来路不明的时刻。
    assert_eq!(super::wake_now_ms(&events[1..], anchor), 0);
}

#[test]
fn the_same_log_owes_the_same_rows_whatever_the_clock_says() {
    // 幕内 `now` 不漂移：本幕之后落的新格（哪怕 ts 更晚）不改本次 drive 的时刻，
    // 也不改本次 drive 的欠账（回放可重建的前提）。
    let (events, anchor) = drive_with_stale_sent(500);
    let once = owed(
        ROWS,
        &frozen(),
        &events,
        super::wake_now_ms(&events, anchor),
    );
    let once_text = format!("{once:?}");
    assert!(
        !once.is_empty(),
        "空账上的「逐字相同」是恒真断言，这条必须先看住非空"
    );

    let mut grown = events.clone();
    grown.push(ev_at(
        1,
        "turn.stage",
        json!({ "stage": "round", "round": 0 }),
        9_000_000,
    ));
    assert_eq!(
        super::wake_now_ms(&grown, anchor),
        super::wake_now_ms(&events, anchor),
        "幕锚定了 `now`，本幕之后的事实再晚也不改它"
    );
    let twice = owed(ROWS, &frozen(), &grown, super::wake_now_ms(&grown, anchor));
    // 第二次拿的是**变长后的那份日志**：新格既能改 `now`（已钉住），也能改欠账
    // （多一条事实就多一次模式匹配）。两条都不变，才是「同一份日志重放逐字相同」。
    assert_eq!(once_text, format!("{twice:?}"));
}

/// 第 4 条在**真表**上的报告样例（T-242 验收口径 1）。
///
/// 钉住名单而不是钉住「非空」：新增一对相交必须在名单里显形，否则它今天就是一次
/// 静默遮蔽。名单本身不是错误——`lease` 与三行 `await-*` 同时开账是设计如此
/// （卡住的调用既在等应答，又该有人管），裁决由 `prio` 决出。
#[test]
fn the_overlap_report_on_the_production_table() {
    // 名单随表冻结一次（`seal` 通过才算得出），读侧只问表、不再自己算第二遍。
    let frozen = frozen();
    let report = frozen.overlaps();
    let rendered: Vec<(&str, &str, &[&str])> = report
        .iter()
        .map(|o| (o.rows[0], o.rows[1], o.facts.as_slice()))
        .collect();
    // `turn_head` 自 T-244 步 5 起多读两类格（`has_final` 的前件：模型响应的
    // `calls` + 两条收束格），步 5b 又加了 `failed`（`tools_open` 的另一种终局），
    // 步 5d 再给 `tools_open` 补了请求面（`request`），凡是共用这个幕头的相交对，
    // 名单就一起变宽——这正是第 4 条要「点名而不只报非空」的理由。
    let head: &[&str] = &[
        "chat.assistant.fallback",
        "chat.assistant.final",
        "failed",
        "request",
        "response",
        "session.config",
        "turn.stage",
        "user.message",
    ];
    assert_eq!(
        rendered,
        vec![
            // 三行策略都读同一个幕头（`turn_head` 折出的格），键同为 `(turn, round)`。
            ("policy.classify", "policy.compress", head),
            ("policy.classify", "policy.recall", head),
            ("policy.compress", "policy.recall", head),
            // 反射快路 / 收束 / 整轮执行同键 `(turn)`：`skill_hit` 与 `resumed` 互斥由
            // 视图折，装载期只报「可能」。
            ("policy.reflex", "close-turn", head),
            ("policy.reflex", "execute-turn", head),
            // 收束行与执行行也相交：两行在「本幕开着」这一点上同时成立，
            // 谁先还得由前件（`has_final`）与 `prio` 决——报告不知道这些。
            ("close-turn", "execute-turn", head),
            // 租约看护读 `stale_sent`，它与创建行 / 三行 await 的前件折出同一批协议格。
            ("open-tools", "lease", &["response"]),
            ("await-turn", "lease", &["sent"]),
            ("await-model", "lease", &["sent"]),
            ("await-tools", "lease", &["sent"]),
        ]
    );
}

/// 第 5 条的端口侧在**真表**上成立：三个端口的 `response` / `failed` 都有行在还，
/// `before` 都是声明过的协议格。
#[test]
fn every_declared_port_has_its_answer_exits() {
    // 真表上的第 5 条端口侧：`load` 的 `Ok` 已经包含这一格（检查的出口只有一个），
    // 这里再点名单问一次——它防的是「三个端口的应答出口没人还」这种静默。
    assert!(crate::symbio_core::ring::check_ports(ROWS, &frozen(), PORTS).is_empty());
}
