//! `projection` 单测 —— S0 出口判据 N1（投影确定性：双跑逐字节相同，[plan/04 §4 C6](../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）。
//!
//! 纯净性本身是**编译期**的（`new` 的泛型约束）；这里守的是二级：确定性可观测。

use super::*;
use crate::symbio_core::event::{Entity, Event, Verb};
use crate::symbio_core::store::Store;
use crate::symbio_core::view::Budget;

fn ev(id: &str, kind: &str, verb: Verb, turn: u64) -> Event {
    Event::pending(id, kind, Entity::Turn, verb, turn, "main").with_ts(1_000)
}

/// 骨架投影：统计**交给它的事件**里某 kind 的条数（S1 起的 `turnstate` 视图从
/// 这里长出来）。闭包只返回 `V`——降级与预算由 `apply` 统一执行（D-105）。
fn seen_count_projection() -> Projection<usize> {
    Projection::new(|events, _now| events.iter().filter(|e| e.kind == "user.message").count())
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
    assert_eq!(
        v1.used.ms, 6,
        "used.ms = 交给闭包的窗口长度（6 条事件全入窗）"
    );
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
    // 预算语义在框架层（D-105）：`budget.ms` 为扫描配额，超窗只交**切片头部**，
    // 截断 ⇒ 外壳 `degraded`。闭包对此一无所知——它只统计见到的事件。
    let p: Projection<String> = Projection::new(|events, _now| format!("{} 条", events.len()));
    let events: Vec<Event> = (0..10)
        .map(|i| ev(&format!("e{i}"), "user.message", Verb::Opened, i))
        .collect();
    let tight = p.apply(&events, 0, Budget::new(0, 3));
    assert!(tight.degraded, "超窗必须降级");
    assert_eq!(
        tight.value, "3 条",
        "窗口只交切片头部：闭包只见到配额内的事件（截尾不截头）"
    );
    assert_eq!(tight.used.ms, 3, "used.ms = 窗口实际长度");
    assert!(!tight.value.is_empty(), "降级也要给出可用的值，不能抛错");

    let ok = p.apply(&events, 0, Budget::new(0, 100));
    assert!(!ok.degraded);
    assert_eq!(ok.value, "10 条", "窗 >= 全量 ⇒ 行为与无预算逐字一致");
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

// ── `read_all`：全量读数的**具名口径**（ ⑧）──────────────────────

/// 一个把 `now` 透出来的投影：用它判「as-of 那一格到底传了什么」。
///
/// 口径写错（`now` 小一寸）在别的投影上是**静默少读事实**，这里让它可断言。
fn as_of_projection() -> Projection<i64> {
    Projection::new(|_events, now| now)
}

#[test]
fn read_all_is_the_named_form_of_the_full_reading_dialect() {
    let mut events = Vec::new();
    for i in 0..40 {
        events.push(ev(&format!("u{i}"), "user.message", Verb::Opened, i as u64));
    }
    let p = seen_count_projection();
    let named = p.read_all(&events);
    let spelled = p.apply(&events, i64::MAX, Budget::generous());
    assert_eq!(
        named, spelled,
        "`read_all` 必须是那两个参数的**唯一写法**，不是第三种语义"
    );
    assert_eq!(named.value, 40, "窗口 = 全量：一条不少");
    assert!(
        !named.degraded,
        "全量读数不得标降级（降级只属于有意的窄读数）"
    );

    // as-of 那一格：一切已发生 ⇒ `now` 必须是 `i64::MAX`。
    assert_eq!(
        as_of_projection().read_all(&events).value,
        i64::MAX,
        "WAL 里每一条都已发生，没有未来的事件可滤"
    );
}

#[test]
fn read_all_never_degrades_even_on_an_empty_slice() {
    // 空切片是「这里确实没有数」，不是「截断了」——两者读数不同，不得混。
    let v = seen_count_projection().read_all(&[]);
    assert!(!v.degraded);
    assert_eq!(v.value, 0);
    assert_eq!(v.used.ms, 0, "used = 窗口实际长度，全量读数下就是切片长度");
}
