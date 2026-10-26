//! 行表的判据（判据 7 / 8 / 10 / 12）。
//!
//! 每个用例都从同一份日志切片出发，只改**一个**条件：只验「全满足时命中」在
//! 「恒返回 true」的实现下也通过。接线之后旧 `LoopState` 已不存在，这里对的是
//! 声明本身——每一行的 `owes` / `until` 与它读的视图字段。
//!
//! 汇报的「静默 + 配额 + 工具轮数」三条全在 `report_due` v4 的 `due` 里
//! （D-163 把轮数前提从触发点折进了视图），本文件对的就是那三格。

use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use super::super::control_decl::{FACTS, VIEWS};
use super::{load, LOOP_OWNER, PORTS, ROWS};
use symbio_core::ring::{check_rows, drive, owed, Decl, Gap, Pattern};
use symbio_core::Frozen;
use symbio_core::{Entity, Event, Seq, Verb};

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
/// 不看 `blocked`：「在等」与「没事」必须可区分（DE-015 §2.2），而 `by = None`
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
            // 点名的接上执行体，**其余的一律断开**：不这样做，任何一行原本 `by`
            // 非空的（端口派发行、外环的宣布行）都会按自己的 `prio` 抢在前面，
            // 夹具就成了「全表的最小债」而不是「这几行之间的次序」。
            c.by = names.contains(&c.name).then_some(super::EXECUTE);
            c
        })
        .collect()
}

/// 去掉一个模式键里的 `attempt`：退回 D-138 之前的逻辑键 `(port, id)`。
fn without_attempt(key: &'static [&'static str]) -> &'static [&'static str] {
    Box::leak(
        key.iter()
            .copied()
            .filter(|k| *k != "attempt")
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    )
}

/// 把整张表的 `owes` / `until` / `unless` 键退回逻辑 `(port, id)`。
///
/// 只给**反向自检**用：同一组事实在旧键口径下必须不成立，否则「重试重新开账」
/// 这条断言可能只是恒真，而不是键粒度带来的差别。
fn logical_keys(rows: &[Decl]) -> Vec<Decl> {
    let pattern = |p: &Pattern| Pattern {
        source: p.source,
        key: without_attempt(p.key),
        eq: p.eq,
    };
    rows.iter()
        .map(|d| Decl {
            name: d.name,
            prio: d.prio,
            by: d.by,
            tier: d.tier,
            expand: d.expand,
            owes: pattern(&d.owes),
            until: Box::leak(
                d.until
                    .iter()
                    .map(pattern)
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            ),
            unless: Box::leak(
                d.unless
                    .iter()
                    .map(pattern)
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            ),
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

    // 出口是快路自己的阶段格（D-163）：正文归收束格，快路只关自己这一笔账。
    let mut answered = hit.clone();
    answered.push(ev(
        1,
        "turn.stage",
        json!({"stage": "reflex", "round": 0, "result": "sk-1"}),
    ));
    assert!(!owes("policy.reflex", &answered, 0));
    // 收束本身**不是**这一行的出口：只落 final、没落快路格 ⇒ 仍欠（那一轮照常进模型往返）。
    let mut closed = hit.clone();
    closed.push(ev(1, "chat.assistant.final", json!({"text": "答完了"})));
    assert!(owes("policy.reflex", &closed, 0));
}

#[test]
fn report_row_owes_on_a_quiet_interval_with_quota_left() {
    // 本幕跑完过一批工具（带 `calls` 的模型应答 = 一轮）：轮数前提（`D-163`：
    // 口径 v4 折进 `due`）靠它满足。
    let worked_round = || {
        ev_at(
            1,
            symbio_core::EVENT_RESPONSE,
            json!({
                "port": "model",
                "id": "m-1",
                "attempt": 1,
                "calls": [{ "port": "tool", "id": "tool-1-d0-c1", "name": "read_file", "arguments": {} }],
            }),
            30,
        )
    };
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
        worked_round(),
    ];
    assert!(owes("policy.report", &quiet, 60_000));

    // 静默不够久。
    assert!(!owes("policy.report", &quiet, 500));

    // 轮数不够：同一段静默、配额也在，但本幕一批工具都没跑完 ⇒ 不说。
    // 这条是 v4 折进来的那一格（旧实现由触发点自己数批次边界）。
    let lazy = vec![
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
    assert!(!owes("policy.report", &lazy, 60_000));
    // 反向：把门槛提到两轮，同样只跑了一轮 ⇒ 不够。
    let mut two_rounds = quiet.clone();
    two_rounds[0] = ev_at(
        1,
        "session.config",
        json!({
            "progress_enabled": true,
            "progress_interval_ms": 1000,
            "progress_min_rounds": 2,
            "progress_max_per_turn": 2,
        }),
        10,
    );
    assert!(!owes("policy.report", &two_rounds, 60_000));

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

    // 开关关掉（其余三条全成立）。
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
        worked_round(),
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
    // 轮数也随幕重新开账：首幕那一轮不算本幕的（锚之前）。
    assert!(
        !owes("policy.report", &resumed, 5_000),
        "续写幕还没跑完自己的第一批工具 ⇒ 不算进度"
    );
    resumed.push(ev_at(
        1,
        symbio_core::EVENT_RESPONSE,
        json!({
            "port": "model",
            "id": "m-2",
            "attempt": 1,
            "calls": [{ "port": "tool", "id": "tool-1-d1-c1", "name": "read_file", "arguments": {} }],
        }),
        3_100,
    ));
    assert!(owes("policy.report", &resumed, 5_000));
}

// ── 五条策略行的两侧证据（生产形状 / 生产符号）──────────────────────────

/// 五条策略行的「不是装饰」证据：**开账用生产的落格形状，关账用生产的符号**。
///
/// 为什么单独立一条，而不是靠上面那些逐行用例：那些钉的是判据语义（什么条件欠、
/// 什么条件不欠），事件种类与 `stage` 值全是手写字面量。把 `StageName::Decide` 的
/// 人读名改掉，它们照样绿——而行表里写死的 `"decide"` 会从「关得上」变成
/// **「永远关不上」**：`policy.classify` 每轮重跑一遍判决，日志上却一个红字都没有。
/// 生产符号与行表的字面量是两份数据，只有本用例把它们逐字对上。
///
/// 分工说清楚：本用例只验「行读的那张视图真能由生产形状开、由生产符号关」；
/// 派发那一侧的行为（选中哪一行、交出哪个执行体）归各 host 的派发用例，不在这里。
#[test]
fn policy_rows_open_on_production_shapes_and_close_on_production_symbols() {
    use super::super::stage_events::{StageFact, StageName};
    use symbio_core::{
        EVENT_ASSISTANT_REPORTED, EVENT_SESSION_CONFIG, EVENT_TURN_STAGE, EVENT_USER_MESSAGE,
    };

    // 生产侧的三条来源，逐条对齐：
    //  - 用户格与配置格的**种类**与 id 形状：`chat_loop/drive.rs` 的 `u-{turn}` /
    //    `cfg-{turn}-{drive_seq}`；配置载荷的键集就是 `plain()` 那一份（视图按键取值，
    //    键名漂了 ⇒ 视图取不到 ⇒ 行恒不欠，策略静默停摆）。
    //  - 阶段格的**载荷**由 `StageFact::payload()` 给出、`stage` 值由 `StageName::name()`
    //    给出：这里一律不写字面量，行表能不能关账就只取决于生产那两处是不是同一个名字。
    let user = || ev(1, EVENT_USER_MESSAGE, json!({ "text": "hi", "id": "u-1" }));
    // 生产那份配置载荷（`plain()` 给全键集 = 关掉的会话），只覆盖用例关心的那一个开关。
    let with_flags = |flags: Value| {
        let mut payload = plain();
        for (k, v) in flags.as_object().expect("配置格是对象") {
            payload[k.clone()] = v.clone();
        }
        payload
    };
    let config = |flags: Value| ev(1, EVENT_SESSION_CONFIG, with_flags(flags));
    // 阶段格的载荷由 `StageFact::payload()` 给出（`stage_events::land_with_seqs` 落的
    // 就是这一份），`stage` 值由 `StageName::name()` 给出——这里不写字面量。
    let stage = |fact: StageFact| ev(1, EVENT_TURN_STAGE, fact.payload());
    let round = || stage(StageFact::request_round(0, "1"));

    // `policy.classify`：轮首 + 开关开 + 有发言 ⇒ 欠；`StageName::Decide` 落格 ⇒ 关。
    let mut head = vec![user(), config(json!({"classify_enabled": true})), round()];
    assert!(owes("policy.classify", &head, 0));
    head.push(stage(StageFact::ran(StageName::Decide, 0)));
    assert!(!owes("policy.classify", &head, 0));

    // `policy.compress`：同一张视图（`turn_head`）的另一组键；出口是 `StageName::Compress`。
    let mut compress = vec![user(), config(json!({"auto_compress": true})), round()];
    assert!(owes("policy.compress", &compress, 0));
    compress.push(stage(StageFact::compress(0, 1, 1)));
    assert!(!owes("policy.compress", &compress, 0));

    // `policy.recall`：持久会话才欠；出口是 `StageName::Recall`。
    let mut recall = vec![user(), config(json!({"is_persistent": true})), round()];
    assert!(owes("policy.recall", &recall, 0));
    recall.push(stage(StageFact::ran(StageName::Recall, 0)));
    assert!(!owes("policy.recall", &recall, 0));

    // `policy.reflex`：开账读的是**召回那格的 `result`**（`with_result` 是生产写它的唯一入口），
    // 出口是快路自己的阶段格 `StageName::Reflex`（D-164：正文归收束格，不在这里）。
    let mut reflex = vec![
        user(),
        config(json!({"is_persistent": true})),
        round(),
        stage(StageFact::ran(StageName::Recall, 0).with_result("hit")),
    ];
    assert!(owes("policy.reflex", &reflex, 0));
    reflex.push(stage(StageFact::reflex(0, 1, "sk-1")));
    assert!(!owes("policy.reflex", &reflex, 0));
    // 没有答完的轮上，收束行也不该被快路「替」成已答：`result:"miss"` 只关账、不开收束。
    let mut missed = vec![
        user(),
        config(json!({"is_persistent": true})),
        round(),
        stage(StageFact::ran(StageName::Recall, 0).with_result("hit")),
    ];
    missed.push(stage(StageFact::reflex(0, 1, "miss")));
    assert!(!owes("policy.reflex", &missed, 0));

    // `policy.report`：另一张视图（`report_due`，按静默时长与配额折）。出口格
    // `chat.assistant.reported` 用常量种类 + 生产 id 形状 `rp-{turn}-{no}`。
    let report_cfg = {
        let mut payload = with_flags(json!({}));
        payload["progress_enabled"] = json!(true);
        payload["progress_interval_ms"] = json!(1000);
        payload["progress_max_per_turn"] = json!(2);
        payload
    };
    let due = vec![
        ev_at(1, EVENT_SESSION_CONFIG, report_cfg.clone(), 10),
        ev_at(
            1,
            EVENT_USER_MESSAGE,
            json!({ "text": "hi", "id": "u-1" }),
            20,
        ),
        // 本幕跑完过一批工具（`calls` 非空的模型应答）：轮数前提在 v4 的 `due` 里。
        ev_at(
            1,
            symbio_core::EVENT_RESPONSE,
            json!({
                "port": "model",
                "id": "m-1",
                "attempt": 1,
                "calls": [{ "port": "tool", "id": "tool-1-d0-c1", "name": "read_file", "arguments": {} }],
            }),
            30,
        ),
    ];
    assert!(owes("policy.report", &due, 60_000));
    let mut spoke = due.clone();
    spoke.push(ev_at(
        1,
        EVENT_ASSISTANT_REPORTED,
        json!({ "text": "先报一句" }),
        60_000,
    ));
    assert!(!owes("policy.report", &spoke, 60_000));

    // 反向自检：把 `stage` 的值换成**任何别的**字符串，行就再也关不上——
    // 上面那条断言不是恒真，它确实在读生产那一个名字。
    let mut drifted = vec![user(), config(json!({"classify_enabled": true})), round()];
    drifted.push(ev(
        1,
        EVENT_TURN_STAGE,
        json!({ "stage": "classify", "round": 0 }),
    ));
    assert!(owes("policy.classify", &drifted, 0));
}

// ── 全局裁决序 / 临时执行行 ──────────────────────────────────────────────

/// 承认在册的三处幂等点问（`obligation-precheck OB-004`），点名的两行**必须接了执行体**。
///
/// 这是 OB-004 在 Rust 侧的同一条事实，读的是 `static ROWS` 本体而不是文本：判据脚本
/// 按正则读 `by: Some(`，形状一改就可能读不到（而它读不到就等于放行）；这条不会。
/// 它钉的是承认的**前提**——「动手前复核」只在有人动手时才成立：`by = None` 的行的
/// 唯一派发点就是那处直问，把它写成护栏是给调度旁路洗白。
#[test]
fn the_rows_guarded_by_admitted_prechecks_have_executors() {
    for name in ["open-tools", "authorize-tool"] {
        let decl = ROWS
            .iter()
            .find(|d| d.name == name)
            .unwrap_or_else(|| panic!("承认在册点名的行 `{name}` 不在 `ROWS` 里"));
        assert!(
            decl.by.is_some(),
            "`{name}` 没有执行体 ⇒ 那三处直问不是护栏，是它唯一的派发点"
        );
    }
    // 剩下的策略行正相反：它们没有执行体，所以那几处直问**就是**派发点（调度旁路，
    // 归零对象）。轮首那四行（`policy.classify` / `policy.compress` / `policy.recall` /
    // `policy.reflex`）已由轮首派发点接管（D-159 / D-163），`policy.report` 由批次
    // 边界派发点接管（D-163），都不再在这笔账里。
    // 这一格是 `D-158` 记的那笔账的**归零点**：再接回一行就是倒退（旁路读数 0/1）。
    let unwired: Vec<&str> = ROWS
        .iter()
        .filter(|d| d.name.starts_with("policy.") && d.by.is_none())
        .map(|d| d.name)
        .collect();
    assert!(
        unwired.is_empty(),
        "未接线的策略行必须清空（`D-158` 的台账）：{unwired:?}"
    );
}

/// 幕间判据（D-168 裁决 4）：还开不开下一幕，由**收束格的 `reason`** 说。
///
/// 这一格此前是 `close_turn` 的返回值（`TurnFlow`），由 `chat_loop` 的 `match` 决定
/// ——那是主循环体里最后一个「按步骤号决定下一步」的判定点。改成读事实之后，判据只剩
/// 「`exhausted` ∧ 轮次没到上限」两条，两条都能在日志上验。
#[test]
fn the_next_stage_is_read_from_the_closure_reason() {
    // 幕锚 = 用户格，本幕轮次 = `round`。
    let base = |closure: Option<&str>, round: u64| {
        let mut events = vec![
            ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
            ev(1, "turn.stage", json!({"stage": "round", "round": round})),
        ];
        if let Some(reason) = closure {
            events.push(ev(
                1,
                "chat.assistant.final",
                json!({"text": "答完了", "reason": reason}),
            ));
        }
        events
    };

    // 被输出上限掐断、轮次还没到上限 ⇒ 再开一幕，轮次取 `round + 1`。
    assert_eq!(
        super::next_stage_round(&base(Some("exhausted"), 0), 1, 0),
        Some(1)
    );
    assert_eq!(
        super::next_stage_round(&base(Some("exhausted"), 2), 1, 0),
        Some(3)
    );
    // 到了上限 ⇒ 收工。方向是「宁可少发一次」：多续一次就是多一次模型往返。
    assert_eq!(
        super::next_stage_round(&base(Some("exhausted"), super::MAX_STAGE_ROUNDS), 1, 0),
        None
    );
    // 自然说完 ⇒ 收工。
    assert_eq!(
        super::next_stage_round(&base(Some("completed"), 0), 1, 0),
        None
    );
    // 还没收束 ⇒ 收工（这一幕照样往前走，但走的是环，不是「再开一幕」）。
    assert_eq!(super::next_stage_round(&base(None, 0), 1, 0), None);

    // `round` 读的是**本幕**那一格（续写幕复用轮号，首幕的轮次格不能算进来）。
    // 幕序：首幕（轮次 0，截断收束）→ 续写幕（锚是 `resume` 格）→ 它的轮次格 → 收束。
    let mut resumed = base(Some("exhausted"), 0);
    resumed.push(ev(
        1,
        "turn.stage",
        json!({"stage": "resume", "round": 1, "result": "retry"}),
    ));
    resumed.push(ev(1, "turn.stage", json!({"stage": "round", "round": 1})));
    resumed.push(ev(
        1,
        "chat.assistant.final",
        json!({"text": "又答完了", "reason": "exhausted"}),
    ));
    assert_eq!(super::next_stage_round(&resumed, 1, 0), Some(2));

    // 续写幕还没落自己的轮次格 ⇒ `round` 折成 0（锚之后的默认值），**不许**读成
    // 首幕那个 0 之后再被首幕的收束格带着走。
    let mut bare = base(Some("exhausted"), 0);
    bare.push(ev(
        1,
        "turn.stage",
        json!({"stage": "resume", "round": 1, "result": "retry"}),
    ));
    bare.push(ev(
        1,
        "chat.assistant.final",
        json!({"text": "又答完了", "reason": "exhausted"}),
    ));
    assert_eq!(super::next_stage_round(&bare, 1, 0), Some(1));
}

/// 「本幕派出过没有」读的是派发格自己落的终局（`turn_round.done`），不是返回值。
///
/// 外环停下来之后要靠它把「没派出去」与「派出了但没关」分开——两者都不许安静地收场，
/// 但要查的地方不同（前者查行表前件，后者查收束行为什么没开账）。
#[test]
fn the_stage_knows_whether_it_dispatched() {
    let base = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    assert!(!super::stage_dispatched(&base, 1, 0), "还没宣布，更没派出");

    let mut announced = base.clone();
    announced.push(ev(
        1,
        "request",
        json!({"port": "turn", "id": "turn-1-d0-r0", "attempt": 1}),
    ));
    assert!(
        !super::stage_dispatched(&announced, 1, 0),
        "宣布了不算派出：`request` 只是「欠一笔往返」"
    );

    let mut done = announced.clone();
    done.push(ev(
        1,
        "response",
        json!({"port": "turn", "id": "turn-1-d0-r0"}),
    ));
    assert!(super::stage_dispatched(&done, 1, 0));
}

#[test]
fn the_outer_ring_announces_before_it_dispatches() {
    // 幕首三格：用户格（幕锚）+ 配置 + 轮次格。
    let open = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(1, "session.config", plain()),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    let call = json!({"port": "turn", "id": "turn-1-d0-r0", "attempt": 1});

    // 第一笔债是**准备**（56）：一幕先备输入，才谈得上宣布与派出（D-168 裁决 3）。
    let picked = super::next_stage(&open, 1, 0, super::STAGE_ACTORS).expect("幕首欠一次准备");
    assert_eq!(picked.row, "prepare-turn");
    assert_eq!(picked.by, super::PREPARE);

    // 输入备好之后，第二笔债是**宣布**（57），不是派出：外环的「宣布 → 派出」是
    // 两个环步（D-168 与 D-166 同形）。这一格以前是 `execute-turn` 直接抢到，
    // 因为「一幕欠一次往返」那时还不落成事实。
    let mut prepared = open.clone();
    prepared.push(ev(
        1,
        "turn.stage",
        json!({"stage": "inputs", "round": 0, "result": "ready"}),
    ));
    let picked =
        super::next_stage(&prepared, 1, 0, super::STAGE_ACTORS).expect("备好输入之后欠一次宣布");
    assert_eq!(picked.row, "announce-turn");
    assert_eq!(picked.by, super::ANNOUNCE);

    // 宣布之后：那笔账在日志上开着，轮到派发行。
    let mut announced = prepared.clone();
    announced.push(ev(1, "request", call.clone()));
    let picked = super::next_stage(&announced, 1, 0, super::STAGE_ACTORS).expect("宣布之后欠派出");
    assert_eq!(picked.row, "dispatch-turn");
    assert_eq!(picked.by, super::TURN_RELAY);

    // 派出且答回来：外环这一幕不再欠。轮内那台 host 的账不在这里（它的子集不含
    // `execute-turn` / `close-turn`，那两行由 `next` 那张表回答）。
    let mut answered = announced.clone();
    answered.push(ev(1, "sent", call.clone()));
    answered.push(ev(1, "response", call.clone()));
    assert!(
        super::next_stage(&answered, 1, 0, super::STAGE_ACTORS).is_none(),
        "往返已有终局，外环这一幕收工"
    );

    // 派发被判死（中止 / 适配器失败）：宣布行**不许**重新开账。这一格是
    // `turn_round.done` 存在的全部理由——`failed` 之后本幕没有收束格，
    // 只看「该不该收束」会让宣布行开一笔又失败、再开一笔。
    let mut failed = announced.clone();
    failed.push(ev(
        1,
        "failed",
        json!({"port": "turn", "id": "turn-1-d0-r0", "attempt": 1, "why": "aborted"}),
    ));
    assert!(
        super::next_stage(&failed, 1, 0, super::STAGE_ACTORS).is_none(),
        "派发失败之后不许重新宣布一笔"
    );
}

#[test]
fn execute_row_closes_on_turn_closed() {
    let open = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(1, "session.config", plain()),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    // 问的是**轮内那台 host**（`ACTORS` 子集）：外环的宣布行不在这张表里，
    // 否则它按 57 < 60 抢在前面——那正是两台 host 各有行子集的理由。
    let picked = super::next(&open, 1, 0, super::ACTORS).expect("开着的轮次总是欠一次执行");
    assert_eq!(picked.row, "execute-turn");
    assert_eq!(picked.by, super::EXECUTE);

    let mut closed = open.clone();
    closed.push(ev(1, "chat.assistant.final", json!({"text": "答完了"})));
    assert!(
        super::next(&closed, 1, 0, super::ACTORS).is_none(),
        "收束后无账可还"
    );

    // 快路已作答（D-163）：`execute-turn` 的前件排除它 ⇒ 第一笔债就是收束。
    // 零模型往返因此是**表上的**判据，不靠 55 < 60 的优先级巧合。
    let mut reflex = open.clone();
    reflex.push(ev(
        1,
        "turn.stage",
        json!({"stage": "reflex", "round": 0, "result": "sk-1"}),
    ));
    let picked = super::next(&reflex, 1, 0, super::ACTORS).expect("快路空出的那笔账是收束");
    assert_eq!(picked.row, "close-turn");
    assert_eq!(picked.by, super::CLOSE);
}

/// 定稿答话轮（措辞产线出话）：**表认为该派发 `close-turn`，而产品路径压根不进轮内驱动**。
///
/// 这条不对称是 [D-125](../../../docs/003-design/015-义务环.md#d-125-定稿答话轮不进轮内驱动循环环覆盖的是欠着的往返收束格由轮首准备就地落 "DE-015-义务环") 点名要写下来的：
/// 措辞往返的 `response(port=model)` 带 `calls: []` ⇒ `turn.has_final` 成立 ⇒ 收束行的前件
/// **成立**。但那一轮的答话是 `compose` 在轮首之前备好的，轮首 host 跑完判决（`answered`
/// 那一格）就在准备段里收束、`TurnExit::Completed` 返回——轮内驱动循环压根没起来，
/// `close-turn` **没被问**。
///
/// 为什么要钉一条「表说该派发、实际上不问」的用例：只看表的人会得出「它应该被派发」，
/// 然后去追一个不存在的 bug（第一版判断就是这么来的）。把不对称钉在用例里，
/// 它就从「疑似漏派发」变成「已知且写明的边界」。
#[test]
fn a_drafted_turns_closing_row_is_owed_but_the_driver_never_asks() {
    let drafted = vec![
        ev(
            1,
            "user.message",
            json!({"text": "现在几点了", "id": "u-1"}),
        ),
        ev(
            1,
            "turn.stage",
            json!({"stage": "decide", "reason": "from_context"}),
        ),
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
    // 未接线的行 `by = None` ⇒ `blocked`，会被 `drive` 滤掉（D-118 订正 1）。
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
            // 收束：前件是 `turn.has_final`，执行体是 `close`。
            "close-turn",
            // 外环的准备行（D-168 裁决 3）：一幕先备输入（56），再谈宣布（57）。
            "prepare-turn",
            // 外环的宣布行（D-168）：同一点上比执行行更靠前（57 < 60），
            // 因为「一幕欠一次 `turn` 往返」的第一步是宣布它。
            "announce-turn",
            "execute-turn",
            // 每个端口两行（`dispatch` / `await`）+ 一条管所有端口的租约行
            // （ `turn` / 步 2 的 `model` / 步 3 的 `tool`）；
            // `open-tools` 是工具调用的创建行（D-117），不与派发合并。
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
            symbio_core::LatencyTier::Fast,
            symbio_core::LatencyTier::Fast,
            symbio_core::LatencyTier::Reflex,
            symbio_core::LatencyTier::Deep,
            symbio_core::LatencyTier::Reflex,
            symbio_core::LatencyTier::Reflex,
            // 收束入格不调模型：反射档。
            symbio_core::LatencyTier::Reflex,
            // 准备与宣布都只入格不调模型（模型在它们内部的 host 里）；派出才进深度档。
            symbio_core::LatencyTier::Fast,
            symbio_core::LatencyTier::Fast,
            symbio_core::LatencyTier::Deep,
            // 派发 / 看护是记账，不调模型；`await` 跟的是被等的那一端。
            symbio_core::LatencyTier::Fast,
            symbio_core::LatencyTier::Fast,
            symbio_core::LatencyTier::Fast,
            symbio_core::LatencyTier::Fast,
            // 拒绝收尾与派发同一执行体（不调工具）。
            symbio_core::LatencyTier::Fast,
            symbio_core::LatencyTier::Deep,
            symbio_core::LatencyTier::Deep,
            symbio_core::LatencyTier::Deep,
            symbio_core::LatencyTier::Fast,
        ]
    );
}

// ── 装载期检查的反面夹具（装载失败即拒绝启动）──────────────────────
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
        tier: symbio_core::LatencyTier::Fast,
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

// ── 端口 `turn` 的三行─────────────────────────────────────────

fn request(id: &str, attempt: u64) -> Event {
    ev(
        1,
        symbio_core::EVENT_REQUEST,
        json!({ "port": "turn", "id": id, "attempt": attempt }),
    )
}

fn sent(id: &str, attempt: u64, lease_ms: u64, cap: u64, ts: i64) -> Event {
    ev_at(
        1,
        symbio_core::EVENT_SENT,
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
            symbio_core::EVENT_FAILED,
            json!({ "port": "turn", "id": "t-1", "attempt": 1, "why": "boom" }),
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
        symbio_core::EVENT_RESPONSE,
        json!({ "port": "turn", "id": "t-1", "attempt": 1 }),
    ));
    assert!(!owes("lease", &answered, 100_000));
}

// ── 「在等」与「没事」的区分（D-126）────────────────────────────

/// `waiting` 只认「开账条件 = `sent`」的账——即「已派出、应答还没回来」。
///
/// 反例同样是判据的一半：`lease` 的账**不算**在等（它开在 `stale_sent` 这个视图上，
/// 说的是「卡太久了」而不是「还在路上」），未接线的策略行也不算（那是「没人做」，
/// 不是「在等」）。三条一起才说明这个理由不是「有 blocked 债」的同义词。
#[test]
fn waiting_means_a_sent_request_without_a_terminal() {
    // 空日志：没事可做，不是在等。
    assert!(!super::waiting(&[], 1, 0));
    // 只落了 `request`：欠一次派发，不是「已派出」。
    assert!(!super::waiting(&[request("t-1", 1)], 1, 0));
    // `sent` 落了、终态没落 ⇒ 在等。
    let out = vec![request("t-1", 1), sent("t-1", 1, 1_000, 1, 0)];
    assert!(super::waiting(&out, 1, 0));
    // 租约到期也仍然「在等」——那笔往返还在路上，只是卡住了。
    assert!(super::waiting(&out, 1, 100_000));
    // 终态落了 ⇒ 不在等。
    for terminal in [symbio_core::EVENT_RESPONSE, symbio_core::EVENT_FAILED] {
        let mut done = out.clone();
        done.push(ev(
            1,
            terminal,
            json!({ "port": "turn", "id": "t-1", "attempt": 1 }),
        ));
        assert!(
            !super::waiting(&done, 1, 0),
            "{terminal} 落了之后不该还说「在等」"
        );
    }
}

/// `await-<port>` 是 Wait 折叠的**声明位**，不是装饰行——形状上必须与同端口的
/// `dispatch-*` 行互补（D-126）。四条一起才叫对偶：
///
/// 1. `by = None`：一旦接上执行体，它就成了同一笔账的**第二个派发器**；
/// 2. 开账条件 = 某条 `dispatch-*` 的**出口**（`sent`，同端口同键）——这是
///    「它记的正是派发行关掉的那一笔」的唯一写法；
/// 3. 出口 = 端口的终态（`response` / `failed`）加 `retry`，且落在同一端口上；
/// 4. 反向：每条 `dispatch-*` 都有对应的 `await-*`——新增端口不能「只派不等」。
///
/// 为什么是**用例**而不是装载期检查：夹具刻意只写一行（`ring.test.rs` 的
/// `await-x` 用例就是在验单行形状），装载期强制对偶会把「最小夹具」判成非法；
/// 而 `ROWS` 是 `&'static` 常量，对它跑用例与在装载期判**等价**，且不向夹具收税。
#[test]
fn every_await_row_is_the_dual_of_its_dispatch_row() {
    /// 同一笔账：同源、同键、同端口约束。
    fn same_binding(a: &Pattern, b: &Pattern) -> bool {
        a.source == b.source && a.key == b.key && a.eq == b.eq
    }
    /// 同一端口（键与等值约束相同，源可以不同）。
    fn same_port(a: &Pattern, b: &Pattern) -> bool {
        a.key == b.key && a.eq == b.eq
    }

    let mut waits = 0;
    for row in ROWS {
        let Some(stem) = row.name.strip_prefix("await-") else {
            continue;
        };
        waits += 1;
        assert!(
            row.by.is_none(),
            "`{}` 是 Wait 的折叠，接上执行体就成了第二个派发器",
            row.name
        );

        let dual = format!("dispatch-{stem}");
        let dispatch = ROWS
            .iter()
            .find(|d| d.name == dual.as_str())
            .unwrap_or_else(|| panic!("`{}` 没有对偶行 `{dual}`：等的东西没人派", row.name));
        assert!(
            dispatch.until.iter().any(|p| same_binding(p, &row.owes)),
            "`{}` 的开账条件必须等于 `{dual}` 的某个出口（`sent`）",
            row.name
        );

        for exit in row.until {
            assert!(
                matches!(
                    exit.source,
                    symbio_core::EVENT_RESPONSE
                        | symbio_core::EVENT_FAILED
                        | symbio_core::EVENT_RETRY
                ),
                "`{}` 的出口 `{}` 不是往返的终态（只认 response / failed / retry）",
                row.name,
                exit.source
            );
            assert!(
                same_port(exit, &row.owes),
                "`{}` 的出口 `{}` 落在别的端口上——收的不是同一笔账",
                row.name,
                exit.source
            );
        }
    }
    assert!(waits > 0, "一行 `await-*` 都没有 ⇒ 这条判据什么都没判");

    // 反向：派发过的端口都必须有等待位。
    for row in ROWS {
        let Some(stem) = row.name.strip_prefix("dispatch-") else {
            continue;
        };
        let wait = format!("await-{stem}");
        assert!(
            ROWS.iter().any(|d| d.name == wait.as_str()),
            "`{}` 没有 `{wait}`：派出去的这笔账没人记「在等」",
            row.name
        );
    }
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

// ── 端口 `model` 的两行────────────────────────────────────
//
// 与 `turn` 的两行**同模板**（判据 1）：加一个端口 = 1 条 `PortDecl` +
// 2 行模板 + 0 条新行。下面的用例钉住「同模板」这件事本身——模板漂了
// （比如 `model` 的行少一个 `until` 分支），端口的账就永远关不掉。

fn model_request(id: &str) -> Event {
    ev(
        1,
        symbio_core::EVENT_REQUEST,
        json!({ "port": "model", "id": id, "attempt": 1 }),
    )
}

fn model_sent(id: &str) -> Event {
    ev_at(
        1,
        symbio_core::EVENT_SENT,
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
            symbio_core::EVENT_FAILED,
            json!({ "port": "model", "id": "m-1", "attempt": 1, "why": "boom" }),
        ),
    ];
    assert!(!owes("dispatch-model", &events, 0));
    assert!(!owes("await-model", &events, 0));
}

/// D-166：宣布之后、派出之前，执行行让位给派发行。
///
/// 两个数字（60 / 66）本身**不是**判据——`prio` 一改次序就变，而「这一步有没有活
/// 可干」是事实问题。让位由 `turn_head.model_open` 说；派出之后（`sent` 落了）
/// 执行行重新开账，模型才能接着说下一句。
#[test]
fn the_execute_row_steps_aside_while_a_round_is_undispatched() {
    let mut head = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    // 幕开着、没有工具批、没走快路 ⇒ 执行行欠；派发行不欠（没有请求）。
    assert!(owes("execute-turn", &head, 0));
    assert!(!owes("dispatch-model", &head, 0));

    // 宣布之后：派发行欠，执行行让位（否则它会再宣布一笔，同一幕两条请求并存）。
    head.push(model_request("m-1"));
    assert!(owes("dispatch-model", &head, 0));
    assert!(
        !owes("execute-turn", &head, 0),
        "请求已宣布、还没派出 ⇒ 执行行不许再宣布下一笔"
    );

    // 派出之后：派发行的账变成「等应答」，执行行的让位理由消失。
    head.push(model_sent("m-1"));
    assert!(!owes("dispatch-model", &head, 0));
    assert!(owes("execute-turn", &head, 0), "派出之后执行行重新开账");
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

// ── 工具端口与授权级联────────────────────────────────────

/// D-117：模型响应的 `calls` 集合逐元素开**创建债**，不是一笔整批债。
#[test]
fn a_model_response_expands_its_tool_calls_into_one_debt_each() {
    let calls = json!([
        { "port": "tool", "id": "tool-1-d0-call-a", "name": "read_file", "arguments": {} },
        { "port": "tool", "id": "tool-1-d0-call-b", "name": "read_file", "arguments": {} },
    ]);
    let answered = vec![ev(
        1,
        symbio_core::EVENT_RESPONSE,
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
            symbio_core::EVENT_REQUEST,
            json!({ "port": "tool", "id": "tool-1-d0-call-a", "attempt": 1 }),
        ),
        ev(
            1,
            symbio_core::EVENT_REQUEST,
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
        symbio_core::EVENT_RESPONSE,
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
        symbio_core::EVENT_REQUEST,
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
        symbio_core::EVENT_AUTHZ_GRANTED,
        json!({
            "key": id,
            "port": "tool",
            "id": id,
            "attempt": 1,
            "tool": "read_file",
            "risk": "low",
            "deferred": false,
        }),
    )
}

fn tool_denied(id: &str) -> Event {
    ev(
        1,
        symbio_core::EVENT_AUTHZ_DENIED,
        json!({
            "key": id,
            "port": "tool",
            "id": id,
            "attempt": 1,
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
        symbio_core::EVENT_SENT,
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
    // 步 5b：`dispatch-tools` 只欠 `granted`——一批全被拒时它不欠，若不另立
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
        symbio_core::EVENT_FAILED,
        json!({ "port": "tool", "id": "c-1", "attempt": 1, "why": "denied" }),
    ));
    assert!(!owes("deny-tools", &closed, 0), "`failed` 关掉这一笔");

    // 收束格的拒绝（不带 `port`）不是端口面的拒绝：按 eq 取不到实例，不误开工具债。
    let closure = vec![ev(
        1,
        symbio_core::EVENT_AUTHZ_DENIED,
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
            symbio_core::EVENT_RESPONSE,
            json!({ "port": "tool", "id": "c-1", "attempt": 1 }),
        ));
        e
    };
    assert!(!owes("await-tools", &answered, 0));

    let failed = {
        let mut e = dispatched.clone();
        e.push(ev(
            1,
            symbio_core::EVENT_FAILED,
            json!({ "port": "tool", "id": "c-1", "attempt": 1, "why": "not_executed" }),
        ));
        e
    };
    assert!(!owes("dispatch-tools", &failed, 0));
    assert!(!owes("await-tools", &failed, 0));
}

/// 判据 1：重试写出的 `request{attempt+1}` **重新打开**授权与派发两行（D-138）。
///
/// 三行工具债的键取 `(port, id, attempt)`：上一 attempt 的 `granted` / `sent`
/// 只关它自己那一次。若键退回逻辑 `(port, id)`，第二次尝试的 `request`
/// 会被上一 attempt 的出口提前关掉——没有任何执行体会判它、派它，
/// 「永不派发的 request」正是 PL-016 §2.3 禁止的形态。
#[test]
fn a_retried_request_reopens_authorization_and_dispatch() {
    let id = "c-1";
    let mut sliced = vec![
        tool_request(id),
        tool_granted(id),
        tool_sent(id),
        ev(
            1,
            symbio_core::EVENT_RETRY,
            json!({ "port": "tool", "id": id, "attempt": 1 }),
        ),
        ev(
            1,
            symbio_core::EVENT_REQUEST,
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
        owes("authorize-tool", &sliced, 0),
        "第二次尝试还没判 ⇒ 授权行重新开账"
    );
    assert!(
        !owes("dispatch-tools", &sliced, 0),
        "第二次尝试还没放行 ⇒ 派发行不成立（fail-closed）"
    );

    let f = frozen();
    let rows = wired(&["authorize-tool", "dispatch-tools"]);
    assert_eq!(
        drive(&rows, &f, &sliced, 0).map(|d| d.decl.name),
        Some("authorize-tool"),
        "派发器选中重试出来的那一笔：它先欠一次授权"
    );

    // 第二次尝试放行之后，派发行接着欠；上一 attempt 的 `sent` 不能替它关门。
    sliced.push(ev(
        1,
        symbio_core::EVENT_AUTHZ_GRANTED,
        json!({
            "key": id,
            "port": "tool",
            "id": id,
            "attempt": 2,
            "tool": "read_file",
            "risk": "low",
            "deferred": false,
        }),
    ));
    assert!(!owes("authorize-tool", &sliced, 0));
    assert!(
        owes("dispatch-tools", &sliced, 0),
        "第二次尝试已放行 ⇒ 欠一次派发"
    );
    assert_eq!(
        drive(&rows, &f, &sliced, 0).map(|d| d.decl.name),
        Some("dispatch-tools"),
        "派发器选中第二次尝试的派发"
    );

    // 反向自检：把整张表的键退回逻辑 `(port, id)`，同一组事实里第二次尝试的
    // `request` 就**不再开账**——红点落在键的口径上，而不是模式拼错。
    //
    // 反向表不走 `load`：全表退回逻辑键会撞上装载期检查（键形状变宽的行对
    // 重新相交 ⇒ 依赖成环），那是旧口径的病理，不是本用例的对象。`owed` 对
    // `frozen` 只取事实声明，行来自参数——所以直接用生产冻结表配反向行表。
    let logical = logical_keys(&rows);
    let reopened = owed(&logical, &frozen(), &sliced, 0)
        .iter()
        .any(|d| d.decl.name == "authorize-tool");
    assert!(
        !reopened,
        "键退回 (port, id) 时上一 attempt 的 granted 会提前关门 ⇒ 重试出来的 request 永不派发"
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
        symbio_core::EVENT_RESPONSE,
        json!({ "port": "tool", "id": id, "attempt": 1 }),
    ));
    assert!(!owes("await-tools", &done, 0));
}

/// D-117：恢复调用的债前件是审批动作写下的 `request`，不是模型响应。
///
/// 这一条钉住「`dispatch-tools` 不欠模型响应」这个决定：事实切片里**没有**
/// `response(port=model)`，恢复的那一笔仍照常走「授权 → 派发」两行。
#[test]
fn a_resumed_tool_call_does_not_need_a_model_response() {
    let id = "tool-resume-1-tc-1-1";
    let events = vec![tool_request(id), tool_granted(id)];

    assert!(
        events.iter().all(|e| e.kind != symbio_core::EVENT_RESPONSE
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
            symbio_core::EVENT_FAILED,
            json!({ "port": "tool", "id": "c-1", "attempt": 1, "why": "aborted" }),
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
        symbio_core::EVENT_AUTHZ_DENIED,
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

// ── 派发器的选债口────────────────────────────────────────────

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
        symbio_core::EVENT_RESPONSE,
        json!({ "port": "model", "id": id, "attempt": 1, "calls": [] }),
    )
}

/// 收束行：答完了就欠一次入格，且它比执行行**先**被选中（55 < 60）。
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
            symbio_core::EVENT_RESPONSE,
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

    // 兜底格也在出口里（步 5c 复原 DE-015 §3.1 的 `final | fallback`：两个执行体写的
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

/// 收束行的前件是 `turn.closeable`（c）：**答完了**或**模型没答成**。
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
        symbio_core::EVENT_REQUEST,
        json!({ "port": "model", "id": "m-1", "attempt": 1 }),
    ));
    failed.push(ev(
        1,
        symbio_core::EVENT_FAILED,
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
        symbio_core::EVENT_REQUEST,
        json!({ "port": "model", "id": "m-1", "attempt": 1 }),
    ));
    stale.push(ev(
        1,
        "turn.stage",
        json!({ "stage": "resume", "round": 0, "result": "approve" }),
    ));
    stale.push(ev(
        1,
        symbio_core::EVENT_FAILED,
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
/// 派发器因此没有任何按行名分支的余地（D-118 决策 1–2）。
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

/// 轮首 host 的欠账由**轮首**派发点选中，交出的动作是轮首绑定表里那一条（D-159）。
#[test]
fn next_head_selects_the_recall_row_at_turn_head() {
    let ready = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(
            1,
            "session.config",
            json!({"classify_enabled": false, "is_persistent": true}),
        ),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    let next = super::next_head(&ready, 1, 0, super::HEAD_ACTORS).expect("轮首欠一次召回准备");
    assert_eq!(next.row, "policy.recall");
    assert_eq!(next.by, super::RECALL);
    let run: super::super::head_exec::HeadAction = super::super::head_exec::run_recall;
    assert!(std::ptr::fn_addr_eq(next.action, run));
    // 出口那一格在它自己的名单里：执行体不落它 ⇒ 派发点读不到「做过」（下条钉这个）。
    assert!(next.writes_closed("turn.stage"));
}

/// 轮首两台债的**序**与**档位**：压缩（prio 30）先于召回（40），而两者的出口同族不同档。
///
/// 这一条同时钉住 D-160 的运行期那一半：`turn.stage{stage=compress}` 关不了召回行，
/// `…{stage=recall}` 也关不了压缩行——若出口按事实名配图，两行会互相当对方的关门方
/// （装载期那条 `cycle` 就是它的影子），轮首这两笔债谁也还不掉。
/// 批次边界那一台的派发点：到期时选中 `policy.report` 并交出轮中执行体，
/// 说过之后同一段静默就不再是它（`reported_anchors` 把账关掉）。
#[test]
fn next_report_selects_the_report_row_only_while_the_interval_is_due() {
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
        ev_at(
            1,
            symbio_core::EVENT_RESPONSE,
            json!({
                "port": "model",
                "id": "m-1",
                "attempt": 1,
                "calls": [{ "port": "tool", "id": "tool-1-d0-c1", "name": "read_file", "arguments": {} }],
            }),
            30,
        ),
    ];
    let next = super::next_report(&quiet, 1, 60_000, super::REPORT_ACTORS)
        .expect("静默到期且跑过一批工具 ⇒ 欠一次汇报");
    assert_eq!(next.row, "policy.report");
    assert_eq!(next.by, super::REPORT);
    let run: super::super::report_exec::ReportAction = super::super::report_exec::run_report;
    assert!(std::ptr::fn_addr_eq(next.action, run));
    // 出口那一格在它自己的名单里：执行体不落它 ⇒ 派发点读不到「做过」。
    assert!(next.writes_closed("chat.assistant.reported"));

    // 说过了：静默锚前移到那句，同一段静默只关一次账。
    let mut spoke = quiet.clone();
    spoke.push(ev_at(
        1,
        "chat.assistant.reported",
        json!({"text": "先报一句"}),
        60_000,
    ));
    assert!(
        super::next_report(&spoke, 1, 60_000, super::REPORT_ACTORS).is_none(),
        "刚说过一句 ⇒ 这一台此刻不欠"
    );
}

#[test]
fn head_sweep_runs_compress_before_recall() {
    let ready = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(
            1,
            "session.config",
            json!({"classify_enabled": false, "auto_compress": true, "is_persistent": true}),
        ),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    let next = super::next_head(&ready, 1, 0, super::HEAD_ACTORS).expect("水位过了就欠一次压缩");
    assert_eq!(
        next.row, "policy.compress",
        "同幕两笔债按 `prio` 排，不由派发点行序排"
    );
    assert_eq!(next.by, super::COMPRESS);
    let run: super::super::head_exec::HeadAction = super::super::head_exec::run_compress;
    assert!(std::ptr::fn_addr_eq(next.action, run));

    // 压缩格落地 ⇒ 压缩行关掉，召回行照旧欠：同族不同档互不关账。
    let mut after_compress = ready.clone();
    after_compress.push(ev(
        1,
        "turn.stage",
        json!({"stage": "compress", "round": 0}),
    ));
    let next = super::next_head(&after_compress, 1, 0, super::HEAD_ACTORS)
        .expect("压缩还完，本轮仍欠一次召回");
    assert_eq!(next.row, "policy.recall");

    // 反向：只有召回格时压缩行仍开着。
    let mut after_recall = ready.clone();
    after_recall.push(ev(
        1,
        "turn.stage",
        json!({"stage": "recall", "round": 0, "result": "miss"}),
    ));
    let next = super::next_head(&after_recall, 1, 0, super::HEAD_ACTORS)
        .expect("召回不落压缩那一档，压缩仍欠");
    assert_eq!(next.row, "policy.compress");
}

/// 判决判成「就此答话」的一轮：压缩与召回**都不开账**（[D-162] 裁决 1）。
///
/// 定稿答话轮压根不进驱动循环（DE-015 §5.1 / D-125），这两行的债在这一轮没有还的人。
/// 前件钉在 `turn_head` 的同一格上，不是跨键的 `unless`——`owed` 把 `unless` 的键与债
/// 键**逐字**比对，跨键时它是静默不命中（§3.4 第 2 条），表就照绿而产品照跑。
#[test]
fn an_answered_verdict_opens_neither_compress_nor_recall() {
    // 判决格由闭包**当场造**：`ev` 的 seq 取自全局计数，先造出来的格子会落在用户格
    // 之前，而 `turn_head` 的阶段折叠只认幕锚之后的那一格。
    let head = |result: &str| {
        vec![
            ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
            ev(
                1,
                "session.config",
                json!({"classify_enabled": true, "auto_compress": true, "is_persistent": true}),
            ),
            ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
            ev(
                1,
                "turn.stage",
                json!({"stage": "decide", "round": 0, "result": result}),
            ),
        ]
    };
    for row in ["policy.compress", "policy.recall"] {
        assert!(
            !owes(row, &head("answered"), 0),
            "`{row}` 在判成答话的一轮开了账"
        );
    }

    // 判成「继续走」⇒ 两行照旧开账（前件只挡答话轮，不挡常态轮）。
    for row in ["policy.compress", "policy.recall"] {
        assert!(owes(row, &head("proceed"), 0));
    }

    // 没有判决格（卸载 `classify`）⇒ 同样开账：`answered` 折不出来的默认值是 `false`。
    let undecided = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(
            1,
            "session.config",
            json!({"classify_enabled": false, "auto_compress": true, "is_persistent": true}),
        ),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    for row in ["policy.compress", "policy.recall"] {
        assert!(owes(row, &undecided, 0), "`{row}` 在没判过的一轮不开账");
    }
}

/// 召回格落地 ⇒ 轮首派发点收工：环**只靠执行体自己落格**推进。
///
/// 这条是 D-159 把「草稿交运行器落」那处寄存去掉的全部理由。出口格若寄在 `TurnState`
/// 里等步骤 4 才进日志，本幕的第二次选债照样读到 `recall_done = false` ⇒ 同一笔债
/// 再还一遍（派发点在轮首一步都走不动）。
#[test]
fn next_head_is_none_once_the_recall_cell_lands() {
    let mut done = vec![
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
    assert!(super::next_head(&done, 1, 0, super::HEAD_ACTORS).is_none());
    // 反向自检：出口格那一格是关门的原因——去掉它就重新欠账。
    done.remove(3);
    assert!(super::next_head(&done, 1, 0, super::HEAD_ACTORS).is_some());
}

/// **行子集由绑定表派生**（D-159 裁决 2）：轮内派发点选不到轮首的行，反之亦然。
///
/// 为什么不能给派发点整张 `ROWS`：`prio` 是**全局**序，`policy.recall`（40）比
/// `execute-turn`（60）小 ⇒ 整张表会让轮内派发点选中一行动作住在另一张表上的债，
/// 它只能报「表脱节」停住——两台 host 排在后面的债一起饿死。下面第 1 段钉的就是这个
/// 反例：整张表的求解结果确实是 `policy.recall`，所以裁决 2 不是偏好，是必需的。
#[test]
fn each_shard_only_sees_the_rows_its_binding_table_names() {
    let f = frozen();
    let ready = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(
            1,
            "session.config",
            json!({"classify_enabled": false, "is_persistent": true}),
        ),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    let whole = symbio_core::ring::drive(ROWS, &f, &ready, 0).expect("整张表有可还债");
    assert_eq!(
        whole.decl.name, "policy.recall",
        "整张表的最小债是轮首那一行 ⇒ 派发点不收整张表"
    );

    let turn_rows = super::host_rows(&f, &super::actor_names(super::ACTORS));
    assert!(
        turn_rows.iter().any(|d| d.name == "execute-turn")
            && turn_rows.iter().all(|d| d.name != "policy.recall"),
        "轮内子集里出现了轮首的行：{:?}",
        turn_rows.iter().map(|d| d.name).collect::<Vec<_>>()
    );
    let head_rows = super::host_rows(&f, &super::head_actor_names(super::HEAD_ACTORS));
    assert_eq!(
        head_rows.iter().map(|d| d.name).collect::<Vec<_>>(),
        vec![
            "policy.classify",
            "policy.compress",
            "policy.recall",
            "policy.reflex"
        ],
        "轮首子集就是它的绑定表点名的行——多一条就是跨 host 抢债"
    );
    let report_rows = super::host_rows(&f, &super::report_actor_names(super::REPORT_ACTORS));
    assert_eq!(
        report_rows.iter().map(|d| d.name).collect::<Vec<_>>(),
        vec!["policy.report"],
        "批次边界子集只有汇报那一行"
    );

    // 生产派发点各自的答案：同一个求解器，两份行子集。
    let next = super::next(&ready, 1, 0, super::ACTORS).expect("开着的幕欠 execute-turn");
    assert_eq!(next.row, "execute-turn");
    assert_eq!(next.by, super::EXECUTE);
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
            super::entry_of(by, super::ACTORS).is_some()
                || super::head_entry_of(by, super::HEAD_ACTORS).is_some()
                || super::report_entry_of(by, super::REPORT_ACTORS).is_some()
                || super::stage_entry_of(by, super::STAGE_ACTORS).is_some(),
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

/// 过渡豁免是一条**有名有姓、会自己变红**的名单（D-123 决策 2）。
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
    // 名单**空**（D-166）：唯一一条豁免是 `execute` 写的兜底格，而那一笔随「模型往返
    // 的派发」搬到了 `relay`——它的出口是引擎独占的 `sent` / `failed`，与 `close` 的
    // 出口不同族，于是那两格**声明得了**（`ACTOR_RELAY.writes`），不再需要豁免。
    // 从「一条豁免」到「零条」也是判据的一半：豁免只许随写方搬走而消失，不许为了
    // 让读数好看而新增。
    let expected: Vec<(&str, &[&str])> = Vec::new();
    assert_eq!(actual, expected, "过渡豁免名单变了——先补裁决，再改这条用例");
}

/// 运行期写入封闭的口径（D-123 决策 2 的那一句「协议格豁免，领域格必须声明」）。
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
    // 收束格里的兜底那一格**不**在 `execute` 的名单上（D-166）：它随模型往返的派发
    // 搬到了 `relay`——`execute` 只宣布请求，写兜底的时机不在它的动作窗口里。
    assert!(
        !next.writes_closed("chat.assistant.fallback"),
        "兜底格由 relay 写（D-166），execute 的窗口里没有这一笔"
    );
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

// ── 本次 drive 的 `now`：只来自唤醒事实（判据 12 · ）──────────────────

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

/// 第 4 条在**真表**上的报告样例（验收口径 1）。
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
    // `turn_head` 自 步 5 起多读两类格（`has_final` 的前件：模型响应的
    // `calls` + 两条收束格），步 5b 又加了 `failed`（`tools_open` 的另一种终局），
    // 步 5d 再给 `tools_open` 补了请求面（`request`），v11 又给 `model_open` 补了
    // 派发前格（`sent`，判「这一笔派出去了没有」）——凡是共用这个幕头的相交对，
    // 名单就一起变宽，这正是第 4 条要「点名而不只报非空」的理由。
    // 外环宣布行那一对：`turn_round` 的读面（幕锚两格 + 协议四族的键）。
    let turn_round: &[&str] = &[
        "failed",
        "request",
        "response",
        "sent",
        "turn.stage",
        "user.message",
    ];
    let head: &[&str] = &[
        "chat.assistant.fallback",
        "chat.assistant.final",
        "failed",
        "request",
        "response",
        "sent",
        "session.config",
        "turn.stage",
        "user.message",
    ];
    assert_eq!(
        rendered,
        vec![
            // 四行策略都读同一个幕头（`turn_head` 折出的格），键同为 `(turn, round)`。
            //
            // 准备行（D-168 裁决 3）也读幕头（`prepared` 那一格折在 `turn_head` 上），
            // 因此它与这四行各成一对：读面相同、前件不同（`prepared` vs 各行的开关与
            // 结论格）。它们今天由 `prio`（10/30/40/50 < 56）决出，报告不知道这些。
            ("policy.classify", "policy.compress", head),
            ("policy.classify", "policy.recall", head),
            ("policy.classify", "policy.reflex", head),
            ("policy.classify", "prepare-turn", head),
            ("policy.compress", "policy.recall", head),
            ("policy.compress", "policy.reflex", head),
            ("policy.compress", "prepare-turn", head),
            ("policy.recall", "policy.reflex", head),
            ("policy.recall", "prepare-turn", head),
            ("policy.reflex", "prepare-turn", head),
            // 收束行与执行行也相交：两行在「本幕开着」这一点上同时成立，
            // 谁先还得由前件（`has_final` / `reflex_answered`）与 `prio` 决——报告不知道这些。
            // 快路行不再与它们相交（D-163）：它的出口读 `(turn, round)`，键形状与
            // 收束 / 执行两行的 `(turn)` 不同，装载期不再报这两对。
            //
            // 外环宣布行（D-168）读的是 `turn_round`：那张视图的读面只含幕锚两格与
            // 协议四族的键，因此与收束 / 执行两行在「本幕的协议格」这一点上相交。
            // 名单显形而不是静默——这三对今天由前件与 `prio` 决出（55 < 57 < 60），
            // 报告只知道它们相交、不知道这些。三对按表里的下标排序，故宣布行那两对
            // 落在收束 / 执行之间与之后，与名单顺序一致。
            ("close-turn", "announce-turn", turn_round),
            ("close-turn", "execute-turn", head),
            ("announce-turn", "execute-turn", turn_round),
            // 租约看护读 `stale_sent`，它与三行 await 的前件折出同一批协议格。
            // `open-tools` 不再与 `lease` 相交（D-138）：创建行的 `response` 是
            // 逻辑键（它开的是 N 笔调用，不是某一次尝试），租约行的出口格收窄到
            // `(port, id, attempt)`——键形状不同，装载期不再报这一对。
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
    assert!(symbio_core::ring::check_ports(ROWS, &frozen(), PORTS).is_empty());
}

// ── 三条机制判据的可执行证据（判据 12 / 7 / 8）────────────────────
//
// 三条都是「机制声称已落地、此前只有 grep 与口径描述」的判据。它们各自治一种
// 静默失效，合起来才是「回放可重建」的完整形状：
//
//   12 输入闭包——结论只随四个显式参数动，不随真实时间动；
//    7 空表试金石——一页行表都不剩时求解器交空账，不 panic 也不造账；
//    8 任意前缀——日志停在任一格都还是一页说得通的账（拆批 / 崩溃恢复的前提）。

/// 判据 12：`owed` 的输入闭包 = `(rows, frozen, events, now)`，**没有第五样**。
///
/// 设计原文（§2.5 / §6 判据 12）：「只读事实、视图、事件时间戳；不读配置、墙钟、
/// 内存缓存」，验收口径「同一事实切片两次求值逐字相同」。本条把这条口径落成
/// **能红**的形状：
///
/// 1. 取一份「租约**没**到期」的切片（`now = 999`，租约线在 1000）；
/// 2. 让真实时间真的过去（睡 20ms，跨过一个时钟 tick）；
/// 3. 同一份切片、同一个 `now` 再求值一次，逐字相同。
///
/// 为什么这个选材能红：若实现把 `now` 参数当摆设、偷偷读墙钟（`SystemTime::now`，
/// 量级是纪元毫秒），第二次求值读到的就是天文数字，`lease` 行当场从「不欠」翻成
/// 「欠」——两份渲染就不同。选在租约线**之下**是关键：线之上时墙钟与参数给出
/// 同一答案（都到期），用例恒绿，那等于没判。`ring.rs` 全文 grep `SystemTime` /
/// `Instant::now` 零出现，与本条用例各据一半。
///
/// 反面对照（后半）：把显式参数从 999 挪到 1000，结论必须翻。两条合起来才说明
/// 时间**只**从参数进函数——否则一个压根不读时间的恒真实现也能过前半。
#[test]
fn owed_has_no_input_beyond_its_four_arguments() {
    let events = vec![sent("t-1", 1, 1_000, 1, 0)];
    let f = frozen();
    let before = owed(ROWS, &f, &events, 999);
    assert!(
        !before.is_empty(),
        "空账上的「逐字相同」是恒真断言，这条必须先看住非空"
    );
    assert!(
        before.iter().all(|d| d.decl.name != "lease"),
        "用例前提：now = 999 在租约线（ts 0 + 1000）之下"
    );

    // 让真实时间过去：读一次墙钟就会是不同的值。
    std::thread::sleep(std::time::Duration::from_millis(20));

    let after = owed(ROWS, &f, &events, 999);
    assert_eq!(
        format!("{before:?}"),
        format!("{after:?}"),
        "同一事实切片、同一个 now，隔了真实时间再求值，必须逐字相同"
    );

    // 反面对照：时间确实从第四个参数进来。
    let at_deadline = owed(ROWS, &f, &events, 1_000);
    assert!(
        at_deadline.iter().any(|d| d.decl.name == "lease"),
        "now = 1000 时租约行开账 ⇒ 判据捧的不是一个不读时间的实现"
    );
}

/// 判据 7：清空整张表 ⇒ 系统仍完整（`owed = ∅` ⇒ 永不前进）。
///
/// 这是「零新增机制键」的底线试金石：一页行表都不剩时，求解器必须交出空账，
/// 而不是 panic、也不是凭声明表里的名字造一笔债出来。跑的是 `owed(&[], …)`——
/// 清空的是**行表**（`ROWS`），声明表照旧（`Frozen` 是事实 / 视图 / 端口的冻结，
/// 不属于行表；空行表时它根本不会被解引用）。
///
/// 两层一起才叫完整：装载期 `load(&[])` **拒绝启动**（端口没有应答出口、声明的
/// 事实全是孤儿），运行期 `owed(&[])` 退化成 ∅。前者在开幕前拦，后者是万一绕过去
/// 之后的兜底——「永不前进」不是「起不来」，是起了也不动。
#[test]
fn an_empty_row_table_owes_nothing_and_never_panics() {
    let events = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(1, "session.config", plain()),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    let f = frozen();
    assert!(
        !owed(ROWS, &f, &events, 0).is_empty(),
        "用例前提：真表在这份日志上有账（否则「空表 ⇒ ∅」是恒真断言）"
    );

    // 一页行表都不剩。
    assert!(owed(&[], &f, &events, 0).is_empty());
    assert!(
        drive(&[], &f, &events, 0).is_none(),
        "没有可还的债 ⇒ 派发器谁都不选"
    );

    // 装载期那一层：空行表开不了幕。
    assert!(
        super::load(&[]).is_err(),
        "空行表必须拒绝装载，而不是开一个不动的幕"
    );
}

/// 声明 owner 清单（DE-023 D-178）：真表装载后，每条控制面事实与视图都能答出
/// 「谁交进来的」——控制面视图**全部**由 `plugins/loop` 声明，没有一条是无主的。
#[test]
fn every_control_plane_declaration_records_its_owner() {
    let f = load(ROWS).expect("真表必须能冻结");
    for fact in FACTS {
        assert_eq!(
            f.fact_owner(fact.name),
            Some(LOOP_OWNER),
            "事实 {} 的 owner 必须是交表方",
            fact.name
        );
    }
    for view in VIEWS {
        assert_eq!(
            f.view_owner(view.name),
            Some(LOOP_OWNER),
            "视图 {} 的 owner 必须是交表方",
            view.name
        );
    }
    // 生产装载里不允许出现「影子声明」：冻结表上的每一条都得有 owner。
    for fact in f.facts() {
        assert!(
            f.fact_owner(fact.name).is_some(),
            "{} 没有 owner",
            fact.name
        );
    }
    for view in f.views() {
        assert!(
            f.view_owner(view.name).is_some(),
            "{} 没有 owner",
            view.name
        );
    }
}

/// 一份 27 格的会话日志，形状照一次真实 CLI 实测（过 `--strict` 验收的长会话）：
/// 第一轮开幕即派发、随后崩溃只留下两笔 `sent`；第二轮重启，先判死上一世的两笔，
/// 再重跑，走完一次模型往返与一次工具往返。
///
/// 第一轮那两笔 `sent` 的 ts 回拨到租约线之前很久（实测里崩溃重启留下的就是这种
/// 形状）——所以重启后它们**必判死**，这正是截断测试要的「终局格落在后面」。
fn twenty_seven_cells() -> Vec<Event> {
    let stale = 1_000; // 第一轮两笔 sent 的 ts：远在租约线之前
    vec![
        // 第一轮：开幕、派发，然后没了（10 格）。
        ev(0, "user.message", json!({"text": "第一轮", "id": "u-0"})),
        ev(0, "session.config", plain()),
        ev(0, "turn.stage", json!({"stage": "round", "round": 0})),
        request("turn-0-d0-r0", 1),
        sent("turn-0-d0-r0", 1, 600_000, 1, stale),
        ev(
            0,
            "turn.stage",
            json!({"stage": "compress", "round": 0, "model_calls": 0}),
        ),
        ev(
            0,
            "turn.stage",
            json!({"stage": "recall", "round": 0, "result": "miss"}),
        ),
        ev(
            0,
            "turn.dispatch",
            json!({"budget_ms": 60000, "principal": "session/main", "tier": "deep"}),
        ),
        model_request("model-0-d0-s0"),
        ev_at(
            0,
            symbio_core::EVENT_SENT,
            json!({
                "port": "model", "id": "model-0-d0-s0", "attempt": 1,
                "lease_ms": 600_000, "attempt_cap": 1,
            }),
            stale,
        ),
        // 第二轮：重启，先判死，再重跑（17 格）。
        ev(
            1,
            "user.message",
            json!({"text": "第二轮（重启后）", "id": "u-1"}),
        ),
        ev(
            1,
            symbio_core::EVENT_FAILED,
            json!({
                "port": "turn", "id": "turn-0-d0-r0", "attempt": 1, "why": "lease_expired",
            }),
        ),
        ev(
            1,
            symbio_core::EVENT_FAILED,
            json!({
                "port": "model", "id": "model-0-d0-s0", "attempt": 1, "why": "lease_expired",
            }),
        ),
        ev(1, "session.config", plain()),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
        request("turn-1-d10-r0", 1),
        sent("turn-1-d10-r0", 1, 600_000, 1, 1_500_000),
        ev(
            1,
            "turn.stage",
            json!({"stage": "compress", "round": 0, "model_calls": 0}),
        ),
        ev(
            1,
            "turn.stage",
            json!({"stage": "recall", "round": 0, "result": "miss"}),
        ),
        ev(
            1,
            "turn.dispatch",
            json!({"budget_ms": 60000, "principal": "session/main", "tier": "deep"}),
        ),
        model_request("model-1-d10-s0"),
        ev_at(
            1,
            symbio_core::EVENT_SENT,
            json!({
                "port": "model", "id": "model-1-d10-s0", "attempt": 1,
                "lease_ms": 600_000, "attempt_cap": 1,
            }),
            1_500_100,
        ),
        ev(
            1,
            symbio_core::EVENT_RESPONSE,
            json!({
                "port": "model", "id": "model-1-d10-s0", "attempt": 1,
                "calls": [{ "port": "tool", "id": "tool-1-d10-call-a", "name": "read_file", "arguments": {} }],
            }),
        ),
        tool_request("tool-1-d10-call-a"),
        tool_granted("tool-1-d10-call-a"),
        ev_at(
            1,
            symbio_core::EVENT_SENT,
            json!({
                "port": "tool", "id": "tool-1-d10-call-a", "attempt": 1,
                "lease_ms": 600_000, "attempt_cap": 1,
            }),
            1_500_200,
        ),
        ev(
            1,
            symbio_core::EVENT_RESPONSE,
            json!({ "port": "tool", "id": "tool-1-d10-call-a", "attempt": 1 }),
        ),
    ]
}

/// 判据 8：事实的任意前缀都合法（拆批 / 崩溃恢复的前提）。
///
/// 做法：对 27 格会话逐前缀截断重放，每一段都要求满足三样——
///
/// 1. **求值有定义**：不 panic（崩溃恢复时日志停在哪一格是不确定的）；
/// 2. **每一段都确定**：同一段再求值一次逐字相同（回放可重建落到每个截断点上）；
/// 3. **追加一格无害事实，账一页都不许动**：`turn.dispatch` 不在控制面声明集里
///    （它的读方是审计投影，见 `declared_facts_all_have_a_row_home`），所以它是
///    可证明无害的那一格。视图若偷看全 log / 数格数 / 拿「最后一条」当锚，就会红。
///
/// 再加一条**截断敏感性**——这条判据真正的牙：前缀停在判死之前，第一轮两笔
/// 「派出未答」的账必须在（恢复才有东西可做）；前缀走到判死之后，它们必须没
/// （判死真的落了账）。`open_calls` 按切片顺序扫，所以偷看后面格的实现会在
/// 前半被提前关掉——那崩溃恢复就成了一句假话。
#[test]
fn every_prefix_of_a_session_log_is_a_legal_ledger() {
    let events = twenty_seven_cells();
    assert_eq!(events.len(), 27, "这份夹具就是那份 27 格会话");
    let f = frozen();
    let now = 2_000_000; // 远在第一轮那两笔的租约线之外
    assert!(
        f.fact("turn.dispatch").is_none(),
        "用例前提：dispatch 不在控制面声明集里（它才是可证明无害的那一格）"
    );

    for k in 0..=events.len() {
        let slice = &events[..k];
        let once = owed(ROWS, &f, slice, now);
        assert_eq!(
            format!("{once:?}"),
            format!("{:?}", owed(ROWS, &f, slice, now)),
            "第 {k} 格：同一段重放两次不一致"
        );
        let mut grown = slice.to_vec();
        grown.push(ev(
            k as u64,
            "turn.dispatch",
            json!({"budget_ms": 60000, "principal": "session/main", "tier": "deep"}),
        ));
        assert_eq!(
            format!("{once:?}"),
            format!("{:?}", owed(ROWS, &f, &grown, now)),
            "第 {k} 格：追加一格无害事实改了账 ⇒ 视图在偷看截断点之外的东西"
        );
    }

    // 空切片 ⇒ 空账：没有事实就没有实例（判据 7 的事实侧）。
    assert!(owed(ROWS, &f, &[], now).is_empty());

    // 截断敏感性：崩溃那一刻与判死之后。
    let crashed = &events[..10];
    for row in ["await-turn", "await-model", "lease"] {
        assert!(owes(row, crashed, now), "崩溃前缀上 `{row}` 必须欠着");
    }
    let reaped = &events[..13];
    for row in ["await-turn", "await-model", "lease"] {
        assert!(!owes(row, reaped, now), "判死之后 `{row}` 必须收掉");
    }
}
