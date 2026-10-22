//! 收束的授权级联（ADR-116）：三格一条线，四个判据。
//!
//! | # | 判据 | 用例 |
//! |---|---|---|
//! | 1 | 放行 ⇒ `turn.closing` + `authz.granted` 都入格，`close-turn` 可还 | [`a_granted_closure_lands_the_intent_and_the_grant`] |
//! | 2 | 拒绝 ⇒ `authz.denied` 入格、`authz.granted` 没有，`close-turn` **不开账** | [`a_denied_closure_never_opens_the_close_row`] |
//! | 3 | 幂等：同一轮同一格问两次只留一份决策 | [`asking_twice_does_not_duplicate_the_decision`] |
//! | 4 | 意图记的是**要落哪一格**（`key` 原样透传） | [`the_intent_names_the_cell_it_will_land`] |
//!
//! 判据 2 是 T-227 判据 8b 的可执行版：`denied` ⇒ 收束格不落，而且这个「不落」
//! 是**结构**（收束行的 `owes` 没有成立），不是某个 `if` 的结论。

use super::{allow, ClosureKind, Verdict};
use crate::symbio_core::{
    ActorSpec, Entity, Event, EventStore, Scope, Seq, Store, Verb, EVENT_ASSISTANT_FALLBACK,
    EVENT_ASSISTANT_FINAL, EVENT_AUTHZ_DENIED, EVENT_AUTHZ_GRANTED, EVENT_TURN_CLOSING,
    EVENT_USER_MESSAGE,
};

/// 种一条本轮用户格（决策 / 结论的溯源锚），返回它的 seq。
fn seed_user(store: &EventStore, turn: u64) -> u64 {
    store
        .append(
            Event::pending(
                format!("u-{turn}"),
                EVENT_USER_MESSAGE,
                Entity::Turn,
                Verb::Opened,
                turn,
                "user",
            )
            .with_payload(serde_json::json!({ "text": "你好" })),
        )
        .expect("用户格入格")
        .value()
}

/// 生产里唯一可达的拒绝路径：主体自己那一行放行、**父主体**不放行（T-105 上限）。
fn child_of_a_principal_without_reply() -> ActorSpec {
    let mut spec = ActorSpec::trivial("session/main");
    assert!(
        spec.can_land_speech(),
        "前提：主智能体自己那一行持 reply——拒绝必须来自上限那道闸"
    );
    spec.scope = Scope::Child("user".to_string());
    spec
}

fn kinds(store: &EventStore) -> Vec<String> {
    store
        .range(Seq::new(0))
        .iter()
        .map(|e| e.kind.clone())
        .collect()
}

/// 某一行此刻欠不欠（走生产同一个问表口）。
fn owed(row: &str, store: &EventStore, turn: u64) -> bool {
    super::super::loop_rows::owes(
        row,
        &store.range(Seq::new(0)),
        turn,
        crate::symbio_core::clock_now_ms(),
    )
}

/// 判据 1：放行 ⇒ 决策与许可都入格，收束行因此可还。
#[test]
fn a_granted_closure_lands_the_intent_and_the_grant() {
    let store = EventStore::new();
    let user_seq = seed_user(&store, 1);
    let spec = ActorSpec::trivial("session/main");

    let verdict = allow(
        &store,
        &spec,
        1,
        user_seq,
        ClosureKind::Speech,
        EVENT_ASSISTANT_FINAL,
    );
    assert_eq!(verdict, Verdict::Granted);
    assert_eq!(
        kinds(&store),
        vec![EVENT_USER_MESSAGE, EVENT_TURN_CLOSING, EVENT_AUTHZ_GRANTED],
        "顺序是决策 → 结论：许可必须晚于意图（否则授权行没有前件可判）"
    );

    // 两条断言类事实都带溯源（I2）——`produced_by` 指向本轮用户格。
    for e in store.range(Seq::new(0)).iter().skip(1) {
        assert_eq!(e.produced_by, Some(user_seq), "{} 缺溯源", e.kind);
    }

    // 收束行可还：这是「能落格」的**结构**形态。
    assert!(
        owed("close-turn", &store, 1),
        "`authz.granted` 落下之后收束行必须开账"
    );
}

/// 判据 2（判据 8b）：拒绝 ⇒ 否定事实入格，而**收束行根本不欠**。
#[test]
fn a_denied_closure_never_opens_the_close_row() {
    let store = EventStore::new();
    let user_seq = seed_user(&store, 1);
    let spec = child_of_a_principal_without_reply();

    let verdict = allow(
        &store,
        &spec,
        1,
        user_seq,
        ClosureKind::Speech,
        EVENT_ASSISTANT_FINAL,
    );
    let Verdict::Denied(why) = verdict else {
        panic!("父主体不持 reply ⇒ 必须拒");
    };
    assert!(why.contains("reply"), "拒因要说得清缺哪一项：{why}");
    assert_eq!(
        kinds(&store),
        vec![EVENT_USER_MESSAGE, EVENT_TURN_CLOSING, EVENT_AUTHZ_DENIED],
        "被拒时**没有** authz.granted——两者互斥"
    );

    // 否定事实的载荷与 T-219 同形（`reason` 是闭集值，`detail` 是人话）。
    let denial = store
        .range(Seq::new(0))
        .into_iter()
        .find(|e| e.kind == EVENT_AUTHZ_DENIED)
        .expect("否定事实在");
    assert_eq!(denial.payload["reason"], "denied");
    assert_eq!(denial.payload["kind"], "speech");
    assert_eq!(denial.payload["key"], EVENT_ASSISTANT_FINAL);
    assert_eq!(denial.payload["detail"], why.as_str());

    assert!(
        !owed("close-turn", &store, 1),
        "`denied` 不能让收束行开账——这就是「被拒的一轮没有收束格」的结构形态"
    );
    assert!(
        !owed("authorize-close", &store, 1),
        "结论已经落过，授权行不再欠（重跑闸只会再落一条同样的格）"
    );
}

/// 判据 3：同一轮同一格问两次，决策与结论各只留一条。
///
/// 续写幕会重跑同一轮：幂等键按 `(turn, kind)`，第二次是「已记录」而不是第二条。
#[test]
fn asking_twice_does_not_duplicate_the_decision() {
    let store = EventStore::new();
    let user_seq = seed_user(&store, 1);
    let spec = ActorSpec::trivial("session/main");
    let ask = || {
        allow(
            &store,
            &spec,
            1,
            user_seq,
            ClosureKind::Speech,
            EVENT_ASSISTANT_FINAL,
        )
    };

    assert_eq!(ask(), Verdict::Granted);
    // 第二次：收束格还没落，所以仍然 Granted——但**不再多落事实**。
    assert_eq!(ask(), Verdict::Granted);
    assert_eq!(
        kinds(&store),
        vec![EVENT_USER_MESSAGE, EVENT_TURN_CLOSING, EVENT_AUTHZ_GRANTED],
        "重问不得重复落格（幂等键 closing-/grant- 各管一条）"
    );
}

/// 判据 4：意图记的是**要落哪一格**——`key` 原样透传，读侧据此分 final / fallback。
#[test]
fn the_intent_names_the_cell_it_will_land() {
    let store = EventStore::new();
    let user_seq = seed_user(&store, 1);
    let spec = ActorSpec::trivial("session/main");

    assert_eq!(
        allow(
            &store,
            &spec,
            1,
            user_seq,
            ClosureKind::Speech,
            EVENT_ASSISTANT_FALLBACK,
        ),
        Verdict::Granted
    );
    let intent = store
        .range(Seq::new(0))
        .into_iter()
        .find(|e| e.kind == EVENT_TURN_CLOSING)
        .expect("意图在");
    assert_eq!(intent.payload["key"], EVENT_ASSISTANT_FALLBACK);
    assert_eq!(intent.payload["kind"], "speech");
    // 一轮至多一次收束：意图落过之后，授权行不再欠（结论也已落）。
    assert!(!owed("authorize-close", &store, 1));
}
