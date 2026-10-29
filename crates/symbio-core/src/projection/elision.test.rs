//! 区间省略判据：闭区间、seq 口径、坏载荷少藏、分支照旧合流。

use super::*;
use crate::event::{Entity, Event, Seq, Verb};
use crate::test_fixtures::{
    CHAT, EVENT_MESSAGE_ADDED, EVENT_TURN_BRANCH, EVENT_USER_MESSAGE, MESSAGE,
};

fn sequenced(kind: &str, seq: u64) -> Event {
    let mut e = Event::pending(
        format!("e-{seq}"),
        kind,
        Entity::Turn,
        Verb::Opened,
        0,
        "user",
    );
    e.seq = Some(Seq::new(seq));
    e
}

/// 一条「这段被摘要取代」的诞生格（载荷 `elision`）。
fn replacement(from: u64, to: u64) -> Event {
    let mut e = Event::pending(
        "repl",
        EVENT_MESSAGE_ADDED,
        Entity::Message,
        Verb::Opened,
        0,
        "notes",
    );
    e.seq = Some(Seq::new(100));
    e.payload = serde_json::json!({
        "messages": [],
        "elision": { "from": from, "to": to },
    });
    e
}

fn collect(events: &[Event]) -> Elisions {
    Elisions::collect(events, &CHAT, &MESSAGE, |_| false)
}

/// 区间是**闭**的：两端都算在内，区间外不受影响。
#[test]
fn the_range_is_closed_on_both_ends() {
    let events = vec![
        replacement(10, 12),
        sequenced(EVENT_USER_MESSAGE, 9),
        sequenced(EVENT_USER_MESSAGE, 10),
        sequenced(EVENT_USER_MESSAGE, 11),
        sequenced(EVENT_USER_MESSAGE, 12),
        sequenced(EVENT_USER_MESSAGE, 13),
    ];
    let elisions = collect(&events);
    let hit: Vec<u64> = events
        .iter()
        .filter(|e| elisions.elides(e))
        .filter_map(|e| e.seq.map(|s| s.value()))
        .collect();
    assert_eq!(hit, [10, 11, 12], "闭区间两端都在内，区间外一条不藏");
}

/// 单点区间（`from == to`）只藏那一条——摘要行取代一条工具结果时用的就是这个形状。
#[test]
fn a_single_point_range_elides_exactly_one_event() {
    let events = vec![
        replacement(7, 7),
        sequenced(EVENT_USER_MESSAGE, 6),
        sequenced(EVENT_USER_MESSAGE, 7),
        sequenced(EVENT_USER_MESSAGE, 8),
    ];
    let elisions = collect(&events);
    let hit: Vec<u64> = events
        .iter()
        .filter(|e| elisions.elides(e))
        .filter_map(|e| e.seq.map(|s| s.value()))
        .collect();
    assert_eq!(hit, [7]);
}

/// `seq: None` 的事件（只存在于夹具 / 未落盘的行）**不会**命中显式区间：
/// 区间指的是存储分配的位置，没有位置就等于没被指到。
#[test]
fn an_event_without_a_seq_is_never_inside_a_range() {
    let mut pending = Event::pending(
        "no-seq",
        EVENT_USER_MESSAGE,
        Entity::Turn,
        Verb::Opened,
        0,
        "user",
    );
    pending.seq = None;
    let events = vec![replacement(1, 100), pending];
    assert!(!collect(&events).elides(&events[1]));
}

/// 坏载荷（`from > to` / 缺字段 / 非整数）一律视为**没有省略**：容错方向是「少藏」。
#[test]
fn a_malformed_range_hides_nothing() {
    let mut backwards = replacement(12, 10);
    backwards.seq = Some(Seq::new(100));
    let mut missing = replacement(0, 0);
    missing.payload = serde_json::json!({ "messages": [], "elision": { "to": 5 } });
    let events = vec![backwards, missing, sequenced(EVENT_USER_MESSAGE, 5)];
    assert!(!collect(&events).elides(&events[2]));
}

/// 分支那一路照旧合流：未选中支的根（载荷 `turn_id`）仍被判成不读。
#[test]
fn an_unselected_branch_still_elides_through_the_same_judgement() {
    let branch = Event::pending(
        "b-1",
        EVENT_TURN_BRANCH,
        Entity::Turn,
        Verb::Asserted,
        0,
        "user",
    )
    .with_payload(serde_json::json!({ "of": "t-1", "into": "t-2" }));
    let old = Event::pending(
        "old",
        EVENT_USER_MESSAGE,
        Entity::Turn,
        Verb::Opened,
        0,
        "user",
    )
    .with_payload(serde_json::json!({ "turn_id": "t-1", "text": "旧支" }));
    let new = Event::pending(
        "new",
        EVENT_USER_MESSAGE,
        Entity::Turn,
        Verb::Opened,
        0,
        "user",
    )
    .with_payload(serde_json::json!({ "turn_id": "t-2", "text": "新支" }));
    let events = vec![branch, old, new];
    let elisions = collect(&events);
    assert!(elisions.elides(&events[1]), "未选中支该不读");
    assert!(!elisions.elides(&events[2]), "选中的那一支照读");
}

/// 判别位只有一个：载荷带不带 `elision`。
#[test]
fn only_the_payload_key_decides_a_replacement() {
    let with = replacement(1, 2);
    let without = Event::pending(
        "card",
        EVENT_MESSAGE_ADDED,
        Entity::Message,
        Verb::Opened,
        0,
        "user",
    )
    .with_payload(serde_json::json!({ "messages": [] }));
    assert!(is_replacement(&with));
    assert!(!is_replacement(&without));
}
