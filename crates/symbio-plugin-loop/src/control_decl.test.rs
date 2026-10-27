//! 控制面声明集的单元用例：装载 / 冻结，以及全部视图的口径。
//!
//! 反向用例是主菜——一个只会让正确输入通过的白名单等于没有检查。

use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use symbio_core::ring::{Decl, Pattern};
use symbio_core::{Builder, Entity, Frozen, LatencyTier, PluginDeclarations, Seq, Verb};

use super::*;

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

fn cover(fact: &'static str, by: Option<&'static str>) -> Decl {
    let exits: &'static [Pattern] = Box::leak(Box::new([Pattern {
        source: fact,
        key: &[],
        eq: &[],
    }]));
    Decl {
        name: fact,
        prio: 99,
        by,
        tier: LatencyTier::Reflex,
        expand: None,
        owes: exits[0],
        until: exits,
        unless: &[],
    }
}

/// 控制面声明集**装起来**的样子：检查的出口只有一个（§3.3），所以连「只想看视图算得
/// 对不对」的用例也得让这张表通过全部检查——给每个事实补一行归宿即可，控制面
/// （`plugins::loop::loop_rows`）的真行本来就在做同一件事。
fn core_table() -> Arc<Frozen> {
    let mut rows: Vec<Decl> = FACTS.iter().map(|f| cover(f.name, None)).collect();
    for name in [
        EVENT_REQUEST,
        EVENT_SENT,
        EVENT_RESPONSE,
        EVENT_FAILED,
        EVENT_RETRY,
    ] {
        rows.push(cover(name, None));
    }
    let rows: &'static [Decl] = Box::leak(rows.into_boxed_slice());
    let mut b = Builder::new();
    b.declare(
        super::super::loop_rows::LOOP_OWNER,
        PluginDeclarations {
            events: EVENTS,
            facts: FACTS,
            views: VIEWS,
            ports: &[],
            actors: Vec::new(),
            rows,
        },
    );
    b.seal().expect("控制面声明集必须能冻结")
}

/// 每条控制面事实都必须有事件行（`seal` 的 `FactWithoutEvent` 的直接断言）。
///
/// 迁移期这里还有一条「owner 声明与内核 `CELLS` 必须同格」的对表用例——它是
/// 两份表并存期的临时判据，随 D-203 归零批的内核名字表一起退役：现在**只有**一份
/// 表（owner 的 `EventDecl`），「两份表说不一致」这个失败模式已经不可能发生。
#[test]
fn every_control_fact_has_an_event_row() {
    for f in FACTS {
        assert!(
            EVENTS.iter().any(|e| e.name == f.name),
            "{} 有字段声明却没有事件行",
            f.name
        );
    }
}

#[test]
fn control_plane_loads_and_freezes() {
    let frozen = core_table();
    // 名单即**控制面声明集**：每条都被某一行或某条被行用到的视图消费（判据 7）。
    // 只在审计投影里读的断言格（`turn.dispatch` → `invariants::declared_budget_ms`）
    // 不在这里——它仍是事件名表里的业务事实，只是不进表。
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
            "authz.denied",
            "authz.granted",
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
            // 本幕的 `turn` 端口走到哪一步（v1 / D-168）：外环宣布行的唯一前件源，
            // 单开一张而不并进 `turn_head`——并进会把 `request` 拉进出口线，
            // 与 `open-tools` 连成假环。
            "turn_round",
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

    // `ver` 是口径的显式版本：读面或折叠方式一变就要升，否则「同一份日志、两个答案」
    // 在回放里无从解释。这里钉的是各视图**当前**的口径版本。
    // `turn_head` 到 v13：`closure`（D-168 裁决 4，本幕最近一条收束格的 `reason`）
    // 是最新的一格，幕间判据读它决定「还开不开下一幕」；v12 的 `prepared`
    // （D-168 裁决 3，本幕落过 `inputs` 阶段格没有）是 `prepare-turn` 的前件；
    // v11 的 `model_open`（D-166，本幕已宣布、没派出的模型往返）让执行行的让位
    // 从「`prio` 碰巧排对」变成事实前件；
    // 而 `reflex_answered`（D-163）、`answered`（D-162 裁决 1）与 `has_final` /
    // `closeable` / `tools_open` 的读面自 v8 起只看「这一轮自己的模型往返」
    // （内部往返带 `via`，D-122 决策 2）。
    assert_eq!(frozen.view("in_flight").unwrap().ver, 3);
    assert_eq!(frozen.view("stale_sent").unwrap().ver, 2);
    assert_eq!(frozen.view("turn_head").unwrap().ver, 13);
    assert_eq!(frozen.view("turn_stages").unwrap().ver, 1);
    assert_eq!(frozen.view("turn_closed").unwrap().ver, 1);
    assert_eq!(frozen.view("report_due").unwrap().ver, 4);
    assert_eq!(frozen.view("reported_anchors").unwrap().ver, 1);
}

#[test]
fn every_declared_reference_is_a_field() {
    // `port` / `key` / `expand` 都是字段引用；声明不能自相矛盾。
    let frozen = core_table();
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

// ── 声明侧的检查 ─────────────────────────────────────────────────────────
//
// 每个用例都补一行 [`cover`] 归宿：检查的出口只有一个（§3.3），第 7 条不满足就会
// 混进同一份 `Err` 里，而这里断言的是**声明自己**的那一格——补上夹具的归宿，
// 「装载说的就是这一格」才成立。

#[test]
fn eval_takes_only_events_and_now() {
    // 签名即隔离：视图拿不到 `&Store`（写口）也拿不到别的视图。
    let eval: fn(&[Event], i64) -> Value = in_flight;
    assert_eq!(eval(&[], 0), json!([]));
    assert_eq!(core_table().eval("in_flight", &[], 0), Some(json!([])));
    assert_eq!(core_table().eval("ghost", &[], 0), None);
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
            // `failed` 关的是这一次尝试（D-138）：载荷带 `attempt`。
            json!({"port": "turn", "id": "c", "attempt": 2, "why": "boom"}),
        ),
    ];
    assert_eq!(
        core_table().eval("in_flight", &events, 0),
        Some(json!([{"id": "b", "port": "turn", "attempt": 1}]))
    );
}

#[test]
fn in_flight_is_attempt_scoped() {
    // 旧尝试的 `response` 不能收清新尝试的 `sent`（判据 8 的声明侧）。
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
        core_table().eval("in_flight", &events, 0),
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
        core_table().eval("in_flight", &events, 0),
        Some(json!([{"id": "a", "port": "turn", "attempt": 2}]))
    );
}

/// 一条带租约的派发事实（ `sent` 形状）。
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
    let at = |now: i64| core_table().eval("stale_sent", &events, now).unwrap();

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
        core_table().eval("stale_sent", &events, 9_000),
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
            // `failed` 关的是这一次尝试（D-138）：载荷带 `attempt`。
            json!({"port": "turn", "id": "dead", "attempt": 1, "why": "x"}),
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
        core_table().eval("stale_sent", &events, 100_000),
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
        core_table().eval("stale_sent", &events, 1_000_000),
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
            "skill_hit": true,
            // 判决那格的 `result` 缺席（旧夹具只记了「判过」）⇒ 不是答话轮。
            "answered": false,
            // 本幕没走快路 ⇒ 收束行不因它开账。
            "reflex_answered": false,
            // D-168：本幕还没落 `inputs` 阶段格 ⇒ 准备行欠。
            "prepared": false,
            // D-168 裁决 4：本幕未收束 ⇒ 没有「为什么停下」这一格。
            "closure": serde_json::Value::Null,
            "has_final": false,
            "closeable": false,
            "tools_open": false,
            // D-166：本幕没有「已宣布、没派出」的模型往返。
            "model_open": false
        })
    );

    // 快路已作答（D-163）：出口格把 `reflex_answered` 折真 ⇒ 收束行在**零模型往返**
    // 的轮上也开得出账（`has_final` 折不出没有应答的轮）。
    let mut reflex = events.clone();
    reflex.push(ev(
        1,
        "turn.stage",
        json!({"stage": "reflex", "round": 0, "result": "sk-1"}),
    ));
    let head = turn_head(&reflex, 0);
    assert_eq!(head["reflex_answered"], json!(true));
    assert_eq!(head["closeable"], json!(true));

    // `result:"miss"` 只关快路自己那一笔账，不算「已答」：那一轮照常进模型往返。
    let mut missed = events.clone();
    missed.push(ev(
        1,
        "turn.stage",
        json!({"stage": "reflex", "round": 0, "result": "miss"}),
    ));
    let head = turn_head(&missed, 0);
    assert_eq!(head["reflex_answered"], json!(false));
    assert_eq!(head["closeable"], json!(false));

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

/// 幕间判据的**唯一数据面**（D-168 裁决 4）：`closure` 折的是本幕最近一条收束格的
/// `reason` 一格，不是任何执行体的返回值。
#[test]
fn turn_head_folds_the_closure_reason_of_the_current_stage() {
    let base = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    // 未收束 ⇒ `null`。「还没停」与「停下了但没记原因」是两件事，这里只表达前者。
    assert_eq!(turn_head(&base, 0)["closure"], Value::Null);

    let closed = |reason: &str| {
        let mut events = base.clone();
        events.push(ev(
            1,
            "chat.assistant.final",
            json!({"text": "hi", "reason": reason}),
        ));
        events
    };
    // 被输出上限掐断 ⇒ 幕间判据（`next_stage_round`）据它再开一幕。
    assert_eq!(
        turn_head(&closed("exhausted"), 0)["closure"],
        json!("exhausted")
    );
    // 自然说完 ⇒ 收工。
    assert_eq!(
        turn_head(&closed("completed"), 0)["closure"],
        json!("completed")
    );

    // 收束格没有 `reason` 的日志（本轮之前写的）：折成 `null` ⇒ 判据读成「不再开
    // 下一幕」。宁可少续写一次，也不拿一个来路不明的形状去再发一次请求。
    let mut bare = base.clone();
    bare.push(ev(1, "chat.assistant.final", json!({"text": "hi"})));
    assert_eq!(turn_head(&bare, 0)["closure"], Value::Null);
}

#[test]
fn turn_head_answers_only_the_drive_the_verdict_landed_in() {
    // `answered` 折的是判决那格 `result == "answered"`（D-162 裁决 1）：压缩与召回的
    // 前件读它，所以它必须是**这一格**——判成「继续走」的 `proceed` 不许开成答话轮。
    let decide = |result: &str| {
        vec![
            ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
            ev(1, "session.config", flags(true, true, true)),
            ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
            ev(
                1,
                "turn.stage",
                json!({"stage": "decide", "round": 0, "result": result}),
            ),
        ]
    };
    assert_eq!(turn_head(&decide("proceed"), 0)["answered"], json!(false));
    assert_eq!(turn_head(&decide("answered"), 0)["answered"], json!(true));
    // 没有判决格 ⇒ 恒 `false`：卸载 `classify` 时压缩与召回照旧开账。
    let no_verdict = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(1, "session.config", flags(true, true, true)),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    assert_eq!(turn_head(&no_verdict, 0)["answered"], json!(false));

    // 续写幕重跑判决：首幕判成答话不算到本幕头上（折叠按幕锚，同 `classify_done`）。
    let mut resumed = decide("answered");
    resumed.push(ev(
        1,
        "turn.stage",
        json!({"stage": "resume", "round": 0, "result": "approve"}),
    ));
    resumed.push(ev(1, "turn.stage", json!({"stage": "round", "round": 0})));
    assert_eq!(turn_head(&resumed, 0)["answered"], json!(false));
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

/// 模型应答：`calls` 为给定数组（空数组 = 「答了话、没要工具」）。
fn model_response(turn: u64, id: &str, calls: Value) -> Event {
    ev(
        turn,
        EVENT_RESPONSE,
        json!({ "port": "model", "id": id, "attempt": 1, "calls": calls }),
    )
}

#[test]
fn turn_head_has_final_reads_the_latest_model_response_calls() {
    // D-121 决策 2：`has_final` 是**视图字段**，口径钉在日志上——本幕**最近一条**
    // `response(port=model)` 的 `calls` 为空。收束行（`close-turn`）的前件只有这一格，
    // 执行体手里那份内存草稿不参与判定。
    let events = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        model_response(1, "m-1", json!([{"port": "tool", "id": "c-1"}])),
        model_response(1, "m-2", json!([])),
    ];
    assert_eq!(
        turn_head(&events, 0)["has_final"],
        json!(true),
        "最新一条响应没有调用 ⇒ 本幕欠一次收束"
    );
}

#[test]
fn turn_head_has_final_is_false_in_every_not_done_yet_shape() {
    let open = vec![ev(1, "user.message", json!({"text": "hi", "id": "u-1"}))];
    assert_eq!(
        turn_head(&open, 0)["has_final"],
        json!(false),
        "还没有任何模型应答（首步之前 / 中止幕）"
    );

    let still_calling = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        model_response(1, "m-1", json!([])),
        model_response(1, "m-2", json!([{"port": "tool", "id": "c-1"}])),
    ];
    assert_eq!(
        turn_head(&still_calling, 0)["has_final"],
        json!(false),
        "最新一条响应还带调用 ⇒ 只有一条空 `calls` 的响应不算收束"
    );

    let other_port = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(
            1,
            EVENT_RESPONSE,
            json!({ "port": "tool", "id": "c-1", "attempt": 1 }),
        ),
    ];
    assert_eq!(
        turn_head(&other_port, 0)["has_final"],
        json!(false),
        "`calls` 缺席的其它端口响应不是「模型答完了」"
    );

    let mut closed = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        model_response(1, "m-1", json!([])),
    ];
    assert_eq!(turn_head(&closed, 0)["has_final"], json!(true));
    closed.push(ev(
        1,
        "chat.assistant.final",
        json!({ "text": "done", "status": "ok" }),
    ));
    assert_eq!(
        turn_head(&closed, 0)["has_final"],
        json!(false),
        "本幕已收束 ⇒ 不再欠收束（否则 `close-turn` 永远开着）"
    );
}

#[test]
fn turn_head_has_final_ignores_responses_from_an_earlier_drive() {
    // 续写幕复用轮号：首幕那条空 `calls` 的响应不能替新幕开收束债（幕锚收窄）。
    let events = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        model_response(1, "m-1", json!([])),
        ev(
            1,
            "turn.stage",
            json!({ "stage": "resume", "round": 0, "result": "approve" }),
        ),
    ];
    assert_eq!(
        turn_head(&events, 0)["has_final"],
        json!(false),
        "锚点之前的响应属于首幕"
    );
}

#[test]
fn turn_head_closeable_is_true_when_the_model_round_never_answered() {
    // 步 5c：收束行的前件从「答完了」扩到「**模型没答成**」——
    // 中止与适配器失败这两条路没有「`calls` 为空的应答」（答都没答成，哪来应答），
    // `has_final` 因此不成立；但它们同样要把这一轮收掉，而收束格只由 `close`
    // 执行体写 ⇒ 行表得能从日志上判出「该收束了」。
    let events = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(
            1,
            EVENT_REQUEST,
            json!({ "port": "model", "id": "m-1", "attempt": 1 }),
        ),
        ev(
            1,
            EVENT_FAILED,
            json!({ "port": "model", "id": "m-1", "why": "adapter error" }),
        ),
    ];
    let head = turn_head(&events, 0);
    assert_eq!(head["has_final"], json!(false), "没有应答 ⇒ 没有「答完了」");
    assert_eq!(head["closeable"], json!(true), "模型没答成 ⇒ 该收束了");
}

#[test]
fn turn_head_closeable_ignores_a_failure_raised_for_an_earlier_drive() {
    // 崩溃恢复时租约看护会在**本幕锚之后**落一条 `failed(port=model)`——它收的是
    // 上一世那笔卡住的调用，而那笔的 `request` 在锚之前。少了「同一 `(port, id)`
    // 的请求也在本幕」这一条，恢复幕一进门就会判「该收束了」，把「重跑一次模型
    // 往返」读成「这一轮已经答不上来了」。
    let events = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(
            1,
            EVENT_REQUEST,
            json!({ "port": "model", "id": "m-1", "attempt": 1 }),
        ),
        ev(
            1,
            "turn.stage",
            json!({ "stage": "resume", "round": 0, "result": "approve" }),
        ),
        ev(
            1,
            EVENT_FAILED,
            json!({ "port": "model", "id": "m-1", "why": "lease_expired" }),
        ),
    ];
    assert_eq!(
        turn_head(&events, 0)["closeable"],
        json!(false),
        "请求在锚之前 ⇒ 这笔失败不是本幕的模型往返"
    );
}

#[test]
fn turn_head_closeable_is_false_once_the_turn_is_closed() {
    let mut events = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(
            1,
            EVENT_REQUEST,
            json!({ "port": "model", "id": "m-1", "attempt": 1 }),
        ),
        ev(
            1,
            EVENT_FAILED,
            json!({ "port": "model", "id": "m-1", "why": "adapter error" }),
        ),
    ];
    assert_eq!(turn_head(&events, 0)["closeable"], json!(true));
    events.push(ev(
        1,
        "chat.assistant.fallback",
        json!({ "text": "", "status": "failed" }),
    ));
    assert_eq!(
        turn_head(&events, 0)["closeable"],
        json!(false),
        "收束格落下 ⇒ 不再欠收束（否则 `close-turn` 永远开着）"
    );
}

#[test]
fn turn_head_tools_open_until_this_batchs_last_call_is_closed() {
    // `tools_open` 是执行行给工具批让位的前件（b）：本幕最近一条模型响应
    // 要过工具，而其中**还有一笔**没落 `response` / `failed` ⇒ 执行行不欠，创建 /
    // 授权 / 派发先跑完。
    let mut events = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        model_response(
            1,
            "m-1",
            json!([
                {"port": "tool", "id": "c-1"},
                {"port": "tool", "id": "c-2"},
            ]),
        ),
        ev(
            1,
            EVENT_RESPONSE,
            json!({ "port": "tool", "id": "c-1", "attempt": 1 }),
        ),
    ];
    assert_eq!(
        turn_head(&events, 0)["tools_open"],
        json!(true),
        "还剩一笔没关账 ⇒ 工具批仍开着"
    );

    // 最后一笔由 `failed` 关（拒绝 / 中止都是这条出口）。
    events.push(ev(
        1,
        EVENT_FAILED,
        json!({ "port": "tool", "id": "c-2", "why": "denied" }),
    ));
    assert_eq!(
        turn_head(&events, 0)["tools_open"],
        json!(false),
        "每一笔都有终局 ⇒ 换成执行行欠"
    );

    // 没有工具调用的应答：`tools_open` 恒假（收束路径）。
    let done = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        model_response(1, "m-1", json!([])),
    ];
    assert_eq!(turn_head(&done, 0)["tools_open"], json!(false));
}

#[test]
fn turn_head_tools_open_ignores_closures_from_an_earlier_drive() {
    // 续写幕复用轮号：首幕那条工具应答不能替新幕的同一 id 关账——否则新一批
    // 一开账就被判成「已经关过」，执行行抢先跑，工具批永无派发。
    let events = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        model_response(1, "m-1", json!([{"port": "tool", "id": "c-1"}])),
        ev(
            1,
            EVENT_RESPONSE,
            json!({ "port": "tool", "id": "c-1", "attempt": 1 }),
        ),
        ev(
            1,
            "turn.stage",
            json!({ "stage": "resume", "round": 0, "result": "approve" }),
        ),
        model_response(1, "m-2", json!([{"port": "tool", "id": "c-1"}])),
    ];
    assert_eq!(
        turn_head(&events, 0)["tools_open"],
        json!(true),
        "锚点之前的应答属于首幕"
    );
}

#[test]
fn turn_head_follows_a_resume_anchor_back_to_an_archived_turn() {
    // 恢复一条被后续轮次归档的旧轮：切片从旧轮首条事件延伸到今天，最大轮号指向
    // 新轮。表头必须跟着**最新幕锚**（恢复幕的 `turn.stage`）回到旧轮——
    // 否则收束 / 执行两行看的是新轮，旧轮的恢复永远无债可还（d 退役
    // `resume_anchor` 旁路的前提）。
    let events = vec![
        ev_at(1, "user.message", json!({"text": "旧轮", "id": "u-1"}), 10),
        model_response(1, "m-1", json!([{"port": "tool", "id": "c-1"}])),
        ev_at(
            1,
            EVENT_RESPONSE,
            json!({ "port": "tool", "id": "c-1", "attempt": 1 }),
            30,
        ),
        ev_at(2, "user.message", json!({"text": "新轮", "id": "u-2"}), 100),
        ev_at(
            1,
            "turn.stage",
            json!({ "stage": "resume", "round": 0, "result": "approve" }),
            200,
        ),
    ];
    let head = turn_head(&events, 0);
    assert_eq!(head["turn"], json!(1), "表头回到被续写的旧轮");
    assert_eq!(head["resumed"], json!(true));
    assert_eq!(
        head["has_utterance"],
        json!(false),
        "续写幕没有新发言（与恢复前同一条判据）"
    );
}

#[test]
fn turn_head_tools_open_on_an_unanswered_wake_request() {
    // 恢复幕交回的那一笔：模型响应是首幕的（本幕没有新的模型应答），旧批次也
    // 早已关账——唯一「开着」的是唤醒请求本身。请求落在幕锚之前，按幕锚判会漏，
    // 故判据按请求的 seq 取「之后有没有终局」。
    let mut events = vec![
        ev_at(1, "user.message", json!({"text": "hi", "id": "u-1"}), 10),
        model_response(1, "m-1", json!([{"port": "tool", "id": "c-1"}])),
        ev_at(
            1,
            EVENT_RESPONSE,
            json!({ "port": "tool", "id": "c-1", "attempt": 1, "needs_user_action": true }),
            30,
        ),
        ev_at(
            1,
            EVENT_REQUEST,
            json!({ "port": "tool", "id": "w-1", "attempt": 1, "intent": "execute" }),
            40,
        ),
        ev_at(
            1,
            "turn.stage",
            json!({ "stage": "resume", "round": 0, "result": "approve" }),
            50,
        ),
    ];
    assert_eq!(
        turn_head(&events, 0)["tools_open"],
        json!(true),
        "唤醒请求还没有终局 ⇒ 执行行让位给派发行"
    );
    events.push(ev_at(
        1,
        EVENT_RESPONSE,
        json!({ "port": "tool", "id": "w-1", "attempt": 1 }),
        60,
    ));
    assert_eq!(
        turn_head(&events, 0)["tools_open"],
        json!(false),
        "唤醒请求关账后执行行才开"
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

/// 本幕跑完过一批工具：一条带 `calls` 的模型应答就是一轮（v4 的轮数前提读它）。
fn worked(at: i64) -> Event {
    ev_at(
        1,
        EVENT_RESPONSE,
        json!({
            "port": "model",
            "id": "m-1",
            "attempt": 1,
            "calls": [{ "port": "tool", "id": "tool-1-d0-c1", "name": "read_file", "arguments": {} }],
        }),
        at,
    )
}

#[test]
fn report_due_needs_log_and_now_only() {
    let events = vec![
        ev_at(1, "session.config", cfg(1000, 2), 10),
        ev_at(1, "user.message", json!({"text": "hi"}), 20),
        worked(30),
    ];
    // 静默 1500ms > 1000ms，配额未用完，已跑完一批工具 ⇒ 到期。
    assert_eq!(
        report_due(&events, 1520),
        json!({
            "turn": 1,
            "anchor": seq(&events[1]),
            "due": true,
            "quiet_ms": 1500,
            "tool_rounds": 1,
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
            "tool_rounds": 1,
            "min_rounds": 0,
            "reports": 0,
            "quota": 2
        })
    );
    // 一轮工具都没跑完（同一段静默、同一配额）⇒ 不说：那是「还没开始」，不是进度。
    let lazy = &events[..2];
    assert_eq!(report_due(lazy, 1520)["due"], json!(false));
    assert_eq!(report_due(lazy, 1520)["tool_rounds"], json!(0));
}

#[test]
fn report_due_resets_quiet_on_assistant_speech() {
    let events = vec![
        ev_at(1, "session.config", cfg(1000, 2), 10),
        ev_at(1, "user.message", json!({"text": "hi"}), 20),
        worked(40),
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
            "anchor": seq(&events[3]),
            "due": false,
            "quiet_ms": 200,
            "tool_rounds": 1,
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
        // 第二段之后又跑完一批工具：第三段到期时轮数前提也成立。
        worked(2_500),
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
    assert_eq!(due["tool_rounds"], json!(1));
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
        // 续写幕自己也跑完一批工具：轮数同样随幕重新开账。
        worked(9_500),
    ];
    let due = report_due(&events, 10_500);
    assert_eq!(due["reports"], json!(0));
    assert_eq!(due["tool_rounds"], json!(1));
    assert_eq!(due["due"], json!(true));
}

#[test]
fn report_due_is_empty_without_facts() {
    assert_eq!(report_due(&[], 1_000), Value::Null);
}

#[test]
fn views_are_pure_same_slice_same_now_same_value() {
    // D-115 决策 1：`now` 是入参。同一份事实切片求值两次逐字相同；
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
        worked(40),
    ];
    assert_eq!(report_due(&events, 5_000), report_due(&events, 5_000));
    assert_eq!(turn_head(&events, 5_000), turn_head(&events, 5_000));
    assert_eq!(report_due(&events, 5_000)["due"], json!(true));
    assert_eq!(report_due(&events, 500)["due"], json!(false));
}

#[test]
fn turn_head_model_open_until_the_round_is_dispatched() {
    let base = vec![
        ev(1, "user.message", json!({"text": "hi", "id": "u-1"})),
        ev(1, "turn.stage", json!({"stage": "round", "round": 0})),
    ];
    assert_eq!(turn_head(&base, 0)["model_open"], json!(false));

    let mut announced = base.clone();
    announced.push(ev(
        1,
        EVENT_REQUEST,
        json!({"port": "model", "id": "model-1-d0-s0", "attempt": 1}),
    ));
    assert_eq!(
        turn_head(&announced, 0)["model_open"],
        json!(true),
        "宣布之后那一笔还开着 ⇒ 派发行该被选中"
    );

    let mut dispatched = announced.clone();
    dispatched.push(ev(
        1,
        EVENT_SENT,
        json!({"port": "model", "id": "model-1-d0-s0", "attempt": 1}),
    ));
    assert_eq!(
        turn_head(&dispatched, 0)["model_open"],
        json!(false),
        "派出之后账变成 `await-model`，本行的让位理由消失"
    );

    // 另一个 attempt 的 `sent` 关不掉这一笔（键是 `(port, id, attempt)`）。
    let mut other_attempt = announced.clone();
    other_attempt.push(ev(
        1,
        EVENT_SENT,
        json!({"port": "model", "id": "model-1-d0-s0", "attempt": 2}),
    ));
    assert_eq!(
        turn_head(&other_attempt, 0)["model_open"],
        json!(true),
        "按 attempt 对键：重试的 `sent` 不能把上一笔请求提前关掉"
    );

    let mut internal = base.clone();
    internal.push(ev(
        1,
        EVENT_REQUEST,
        json!({"port": "model", "id": "model-z0", "attempt": 1, "via": "compress/summary"}),
    ));
    assert_eq!(
        turn_head(&internal, 0)["model_open"],
        json!(false),
        "内部往返带 `via`，不属于「本幕要派发的那一轮」"
    );
}
