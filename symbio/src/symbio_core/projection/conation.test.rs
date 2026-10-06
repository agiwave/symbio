//! `conation` 投影的单测。
//!
//! 与实现**同级**分文件（`projection/conation.rs` + 同级 `conation.test.rs`，
//! 见 `CONTRIBUTING.md`）。本文件钉四件事：欲的**升格靠溯源认**（不是靠 goal 字符串）、
//! **as-of** 口径、**无溯源的欲可读但永不升格**（I2 边界）、以及**只声明升格过的那条**
//! （不误伤同批的别的欲）。

use super::*;
use crate::symbio_core::authz::PRINCIPAL_AUTONOMOUS as INITIATOR;
use crate::symbio_core::event::{
    Entity, Event, Seq, Verb, EVENT_CONATION_EXPRESSED, EVENT_SYSTEM_TRIGGERED, EVENT_TASK_OPENED,
};
use crate::symbio_core::store::{EventStore, Store};

/// 溯源锚：真实链路里欲是从一次触发出发的（`system.triggered`）。
fn seed_trigger(store: &EventStore) -> u64 {
    store
        .append(
            Event::pending(
                "sys-0",
                EVENT_SYSTEM_TRIGGERED,
                Entity::System,
                Verb::Opened,
                0,
                INITIATOR,
            )
            .with_ts(10)
            .with_payload(serde_json::json!({ "kind": "scheduled" })),
        )
        .expect("触发入格")
        .value()
}

/// 表达一条欲（带溯源），返回它的 seq——升格要靠它认。
fn express(store: &EventStore, id: &str, goal: &str, source: u64, ts: i64) -> u64 {
    store
        .append(
            Event::pending(
                id.to_string(),
                EVENT_CONATION_EXPRESSED,
                Entity::Conation,
                Verb::Opened,
                0,
                INITIATOR,
            )
            .with_ts(ts)
            .with_produced_by(source)
            .with_payload(serde_json::json!({ "goal": goal })),
        )
        .expect("欲入格")
        .value()
}

/// 由某条欲长出一个任务（`produced_by` 指回欲 seq——`open_long_goal` 的形状）。
fn open_task(store: &EventStore, id: &str, task_id: &str, goal: &str, source: u64, ts: i64) {
    store
        .append(
            Event::pending(
                id.to_string(),
                EVENT_TASK_OPENED,
                Entity::Task,
                Verb::Opened,
                0,
                INITIATOR,
            )
            .with_ts(ts)
            .with_produced_by(source)
            .with_payload(serde_json::json!({
                "task_id": task_id,
                "goal": goal,
                "budget_ms": 86_400_000,
            })),
        )
        .expect("任务入格");
}

fn view(store: &EventStore, now: i64) -> ConationView {
    conation()
        .apply(&store.range(Seq::new(0)), now, Budget::generous())
        .value
}

/// 欲先入格、任务后长出来：升格前不算声明，升格后才算。
#[test]
fn an_expressed_intent_is_pending_until_a_task_derives_from_it() {
    let store = EventStore::new();
    let anchor = seed_trigger(&store);
    let want = express(&store, "want-0", "把周报写了", anchor, 100);

    let before = view(&store, i64::MAX);
    assert_eq!(before.intents.len(), 1);
    assert_eq!(before.intents[0].goal, "把周报写了");
    assert_eq!(before.intents[0].task_id, None, "未升格 ⇒ 没有派生任务");
    assert!(
        !before.is_declared("把周报写了"),
        "光有欲不算声明——心跳照旧该开这一格"
    );

    open_task(&store, "o-1", "hb-1", "把周报写了", want, 200);
    let after = view(&store, i64::MAX);
    assert_eq!(after.intents[0].task_id.as_deref(), Some("hb-1"));
    assert!(
        after.is_declared("把周报写了"),
        "升格之后才算声明过（心跳据此不再重开）"
    );
}

/// **升格靠溯源认，不靠 goal 字符串相等**——这正是本投影替代插件内全表扫时收窄的那一点：
/// 同目标但来源别处的任务不得算作这条欲的产物。
#[test]
fn promotion_is_recognised_by_provenance_not_by_goal_text() {
    let store = EventStore::new();
    let anchor = seed_trigger(&store);
    let want = express(&store, "want-0", "整理全年归档", anchor, 100);

    // 同目标、但溯源指向触发（不是这条欲）——比如别的机制开的任务。
    open_task(&store, "o-other", "t-other", "整理全年归档", anchor, 150);
    let by_text = view(&store, i64::MAX);
    assert_eq!(
        by_text.intents[0].task_id, None,
        "goal 字符串相等不等于这条欲升格了"
    );
    assert!(!by_text.is_declared("整理全年归档"));

    // 真正由这条欲长出来的任务：produced_by = 欲 seq。
    open_task(&store, "o-1", "hb-1", "整理全年归档", want, 200);
    let by_provenance = view(&store, i64::MAX);
    assert_eq!(by_provenance.intents[0].task_id.as_deref(), Some("hb-1"));
    assert!(by_provenance.is_declared("整理全年归档"));
}

/// as-of：`ts > now` 的欲与升格一概不见（与其余投影同口径）。
#[test]
fn as_of_hides_intents_and_promotions_from_the_future() {
    let store = EventStore::new();
    let anchor = seed_trigger(&store);
    let want = express(&store, "want-0", "跨天推进", anchor, 1_000);
    open_task(&store, "o-1", "hb-1", "跨天推进", want, 2_000);

    assert!(
        !view(&store, 500)
            .intents
            .iter()
            .any(|i| i.goal == "跨天推进"),
        "as-of 早于表达 ⇒ 这条欲根本不在视图里"
    );
    let mid = view(&store, 1_500);
    assert_eq!(mid.intents.len(), 1, "欲已可见");
    assert_eq!(mid.intents[0].task_id, None, "但升格还没发生");
    assert!(view(&store, 2_500).is_declared("跨天推进"));
}

/// I2 边界：无溯源的欲**进得了视图**（它是事实，藏起来才是撒谎），
/// 但永远升不了格（`ConationCandidate::from_event` 返回 `None`，闸门那道门先关）。
#[test]
fn a_sourceless_intent_is_readable_but_never_promotes() {
    let store = EventStore::new();
    store
        .append(
            Event::pending(
                "want-orphan",
                EVENT_CONATION_EXPRESSED,
                Entity::Conation,
                Verb::Opened,
                0,
                INITIATOR,
            )
            .with_ts(100)
            .with_payload(serde_json::json!({ "goal": "来路不明" })),
        )
        .expect("欲入格");

    let v = view(&store, i64::MAX);
    assert_eq!(v.intents.len(), 1, "事实不藏");
    assert_eq!(v.intents[0].task_id, None);
    assert!(!v.is_declared("来路不明"));
}

/// 声明是**按目标**的：升格了 A 不等于声明过 B（否则去重会误伤同批的别的欲）。
#[test]
fn declaring_one_goal_does_not_declare_another() {
    let store = EventStore::new();
    let anchor = seed_trigger(&store);
    let a = express(&store, "want-a", "目标 A", anchor, 100);
    express(&store, "want-b", "目标 B", anchor, 110);
    open_task(&store, "o-a", "hb-a", "目标 A", a, 200);

    let v = view(&store, i64::MAX);
    assert_eq!(v.intents.len(), 2, "两条欲都在视图里——欲是流，不合并");
    assert!(v.is_declared("目标 A"));
    assert!(!v.is_declared("目标 B"), "只有升格过的那条才算声明");
}
