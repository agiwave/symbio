//! `invariants` 单测 —— 每条检查都要证明「它红得起来」。
//!
//! 反向用例与本体的关系：正向用例证明检查**在违规时报警**，反向用例证明
//! 合规序列**不报警**（检查不是永远红的状态机）。两组都绿，检查才存在。

use super::*;
use crate::symbio_core::event::{Entity, Event, EventEnvelope, Verb};
use crate::symbio_core::store::{EventStore, Store};

/// 造一条 pending 事件（未入库，无 seq）。
fn pending(id: &str, kind: &str, entity: Entity, verb: Verb, turn: u64) -> Event {
    Event::pending(id, kind, entity, verb, turn, "main").with_produced_by(0)
}

/// 造一批**已入库**事件（seq 由 Store 分配 ⇒ 天然单调）。
fn stored(events: Vec<Event>) -> Vec<Event> {
    let store = EventStore::new();
    for e in events {
        store.append(e).expect("首插必成");
    }
    store.range(crate::symbio_core::event::Seq::new(0))
}

// ── C1：seq 单调 ───────────────────────────────────────────────────────

#[test]
fn seq_monotonic_passes_on_store_output() {
    let events = stored(vec![
        pending("e0", "user.message", Entity::Turn, Verb::Opened, 0),
        pending("e1", "chat.assistant.final", Entity::Turn, Verb::Closed, 0),
        pending("e2", "user.message", Entity::Turn, Verb::Opened, 1),
    ]);
    assert!(
        seq_monotonic(&events).is_empty(),
        "Store 产出的序列天然单调：{:?}",
        seq_monotonic(&events)
    );
}

#[test]
fn seq_gap_is_detected() {
    let mut events = stored(vec![
        pending("e0", "user.message", Entity::Turn, Verb::Opened, 0),
        pending("e1", "user.message", Entity::Turn, Verb::Opened, 1),
    ]);
    // 模拟「中间丢了一条」：删除 seq=1 后，后一条应接 seq=1 却是 seq=2。
    events.remove(1);
    // 重新造：seq 0 与 seq 2 相邻（跳号）。
    let mut e2 = pending("e2", "user.message", Entity::Turn, Verb::Opened, 2);
    e2.assign_seq(crate::symbio_core::event::Seq::new(2));
    events.push(e2);
    let bad = seq_monotonic(&events);
    assert_eq!(bad.len(), 1, "跳号必须被看见");
    assert_eq!(bad[0].event_id, "e2");
    assert!(bad[0].why.contains("严格单调"), "{}", bad[0].why);
}

#[test]
fn pending_events_are_skipped_by_seq_check() {
    // pending（未入库）事件没有 seq，不参与单调性检查——它们还不是事实。
    let events = vec![pending("p0", "user.message", Entity::Turn, Verb::Opened, 0)];
    assert!(seq_monotonic(&events).is_empty());
}

// ── C2 / N3：每 turn 至多 1 条 final ───────────────────────────────────

#[test]
fn one_final_per_turn_passes() {
    let events = stored(vec![
        pending("u0", "user.message", Entity::Turn, Verb::Opened, 0),
        pending("f0", "chat.assistant.final", Entity::Turn, Verb::Closed, 0),
        pending("u1", "user.message", Entity::Turn, Verb::Opened, 1),
        pending("f1", "chat.assistant.final", Entity::Turn, Verb::Closed, 1),
    ]);
    assert!(final_unique_per_turn(&events).is_empty());
}

#[test]
fn two_finals_in_one_turn_is_detected() {
    let events = stored(vec![
        pending("u0", "user.message", Entity::Turn, Verb::Opened, 0),
        pending("f0", "chat.assistant.final", Entity::Turn, Verb::Closed, 0),
        // 同一 turn 的第二条 final——转写收敛性被破坏。
        pending("f0b", "chat.assistant.final", Entity::Turn, Verb::Closed, 0),
    ]);
    let bad = final_unique_per_turn(&events);
    assert_eq!(bad.len(), 1, "一轮两条 final 必须被看见");
    assert_eq!(bad[0].event_id, "f0b");
    assert!(
        bad[0].why.contains("f0"),
        "违规要指出已存在的那条：{}",
        bad[0].why
    );
}

#[test]
fn closed_tasks_do_not_count_as_final() {
    // `task.completed` 也是 closed，但不是发言——N3 只管 turn 的发言收敛。
    let events = stored(vec![
        pending("t0", "task.completed", Entity::Task, Verb::Closed, 0),
        pending("t1", "task.completed", Entity::Task, Verb::Closed, 0),
    ]);
    assert!(final_unique_per_turn(&events).is_empty());
}

// ── C3 / N5：断言类事件必须带溯源 ──────────────────────────────────────

#[test]
fn asserted_with_provenance_passes() {
    let events = stored(vec![pending(
        "v0",
        "classify.verdict",
        Entity::Verdict,
        Verb::Asserted,
        0,
    )
    .with_produced_by(0)]);
    assert!(produced_by_coverage(&events).is_empty());
}

#[test]
fn asserted_without_provenance_is_detected() {
    let no_source = pending("v0", "classify.verdict", Entity::Verdict, Verb::Asserted, 0);
    // 显式去掉溯源（pending 默认带 0，这里造一条真正的裸断言）。
    let no_source = Event {
        produced_by: None,
        ..no_source
    };
    let events = stored(vec![no_source]);
    let bad = produced_by_coverage(&events);
    assert_eq!(bad.len(), 1, "裸断言必须被看见（I2 无溯源不声明）");
    assert_eq!(bad[0].event_id, "v0");
}

#[test]
fn non_asserted_events_need_no_provenance() {
    // 普通发言（opened / closed）不是断言，不要求 produced_by。
    let events = vec![pending("u0", "user.message", Entity::Turn, Verb::Opened, 0)];
    assert!(produced_by_coverage(&events).is_empty());
}

// ── check_all：三条合跑 ────────────────────────────────────────────────

#[test]
fn check_all_on_clean_sequence_is_empty() {
    let events = stored(vec![
        pending("u0", "user.message", Entity::Turn, Verb::Opened, 0),
        pending("f0", "chat.assistant.final", Entity::Turn, Verb::Closed, 0),
        pending("v0", "classify.verdict", Entity::Verdict, Verb::Asserted, 0).with_produced_by(0),
    ]);
    assert!(check_all(&events).is_empty(), "合规序列三查全绿");
}
