//! `projection` 单测 —— S0 出口判据 N1（投影确定性：双跑逐字节相同，[plan/04 §4 C6](../../../../docs/plan/04-工程落地.md)）。
//!
//! 纯净性本身是**编译期**的（`new` 的泛型约束）；这里守的是二级：确定性可观测。

use super::*;
use crate::symbio_core::event::{Entity, Event, Verb};
use crate::symbio_core::store::Store;
use crate::symbio_core::view::Budget;

fn ev(id: &str, kind: &str, verb: Verb, turn: u64) -> Event {
    Event::pending(id, kind, Entity::Turn, verb, turn, "main").with_ts(1_000)
}

/// 骨架投影：统计某 turn 的已见事件数（S1 起的 `turnstate` 视图从这里长出来）。
/// 预算口径：数一遍事件，把拿到的 ms 如实上报（`used` 由投影自己记账）。
fn seen_count_projection() -> Projection<usize> {
    Projection::new(|events, _now, b| {
        let n = events.iter().filter(|e| e.kind == "user.message").count();
        View::ok(n, b)
    })
}

// ── N1 / C6：同一事件序列，双跑逐字节相同 ──────────────────────────────

#[test]
fn same_input_yields_byte_identical_view_on_repeat_runs() {
    let mut events = Vec::new();
    for i in 0..5 {
        events.push(ev(&format!("u{i}"), "user.message", Verb::Opened, i as u64));
    }
    events.push(ev("f0", "chat.assistant.final", Verb::Closed, 0));
    let p = seen_count_projection();
    let b = Budget::new(1_000, 10);

    let v1 = p.apply(&events, 1_000, b);
    let v2 = p.apply(&events, 1_000, b);
    assert_eq!(
        v1, v2,
        "同一事件序列 + 同一参数 ⇒ View 必须逐字节相同（N1）"
    );
    assert!(!v1.degraded);
    assert_eq!(v1.value, 5);
    assert_eq!(v1.used, b);
}

// ── 确定性对**输入**敏感（反向用例：改输入，结论必须变） ────────────────

#[test]
fn view_changes_when_input_changes() {
    let p = seen_count_projection();
    let b = Budget::new(1_000, 10);
    let few = vec![ev("u0", "user.message", Verb::Opened, 0)];
    let many = vec![
        ev("u0", "user.message", Verb::Opened, 0),
        ev("u1", "user.message", Verb::Opened, 1),
    ];
    assert_ne!(
        p.apply(&few, 1_000, b),
        p.apply(&many, 1_000, b),
        "输入不同 ⇒ 输出必须不同（否则投影没在读输入）"
    );
}

// ── 可降级：超预算返回 degraded 而不是报错（构造即锁定的义务） ──────────

#[test]
fn over_budget_projection_degrades_instead_of_erroring() {
    let p: Projection<String> = Projection::new(|events, _now, budget| {
        // 模拟「预算不够算完整视图」：预算 ms 少于事件数 ⇒ 降级但仍出值。
        if (budget.ms as usize) < events.len() {
            View::degraded(
                format!("{} 条（截断）", events.len()),
                Budget::new(0, budget.ms),
            )
        } else {
            View::ok(events.len().to_string(), Budget::new(0, budget.ms))
        }
    });
    let events: Vec<Event> = (0..10)
        .map(|i| ev(&format!("e{i}"), "user.message", Verb::Opened, i))
        .collect();
    let tight = p.apply(&events, 0, Budget::new(0, 3));
    assert!(tight.degraded, "超预算必须降级");
    assert!(!tight.value.is_empty(), "降级也要给出可用的值，不能抛错");

    let ok = p.apply(&events, 0, Budget::new(0, 100));
    assert!(!ok.degraded);
}

// ── 纯净性的编译期形状：投影能跑在**快照**上（拿不到 Store 也能工作） ────

#[test]
fn projection_runs_on_a_snapshot_without_any_store() {
    // 事件向量是普通数据；投影从不接触存储——这正是「签名无 &Store」的可运行证明。
    let store = crate::symbio_core::store::EventStore::new();
    store
        .append(ev("u0", "user.message", Verb::Opened, 0))
        .expect("首插必成");
    let snapshot = store.range(crate::symbio_core::event::Seq::new(0));

    let p = seen_count_projection();
    let v = p.apply(&snapshot, 0, Budget::generous());
    assert_eq!(v.value, 1);
}
