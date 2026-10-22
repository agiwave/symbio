//! 义务环求解的单元用例：模式匹配 / 开账与关账 / 挂起 / 全局裁决序 / 纯性 / 装载期检查。
//!
//! 反向用例是主菜：一笔永远不会关的账、一个拼错的源名，都必须在装载期或求解时
//! **说出来**，而不是静默地永不命中。

use serde_json::{json, Value};

use super::super::{Builder, FactDecl, ViewDecl};
use super::*;
use crate::symbio_core::{Entity, Event, LatencyTier, Seq, Verb};

fn ev(seq: u64, turn: u64, kind: &str, payload: Value) -> Event {
    let mut e = Event::pending(
        format!("{kind}-{seq}"),
        kind,
        Entity::Turn,
        Verb::Asserted,
        turn,
        "test",
    )
    .with_payload(payload);
    e.seq = Some(Seq::new(seq));
    e
}

/// 对象视图：`now` 折成布尔字段（时间不进欠账公式的形态）。
fn ready_view(events: &[Event], now: i64) -> Value {
    match events
        .iter()
        .rev()
        .find(|e| e.kind == "f")
        .and_then(|e| e.payload.get("id"))
        .cloned()
    {
        Some(id) => json!({ "turn": 1, "id": id, "ready": now >= 100 }),
        None => Value::Null,
    }
}

/// 数组视图：每个元素一条实例。
fn list_view(events: &[Event], _now: i64) -> Value {
    Value::Array(
        events
            .iter()
            .filter(|e| e.kind == "f")
            .filter_map(|e| e.payload.get("id").cloned())
            .map(|id| json!({ "id": id }))
            .collect(),
    )
}

fn world() -> std::sync::Arc<Frozen> {
    let mut b = Builder::new();
    for name in ["f", "g", "h"] {
        b.fact(FactDecl {
            name,
            port: None,
            key: &[],
            expand: &[],
            fields: &["id", "x", "turn"],
        });
    }
    b.view(ViewDecl {
        name: "v",
        ver: 1,
        reads: &[("f", "id")],
        cost: 64,
        eval: ready_view,
    });
    b.view(ViewDecl {
        name: "v_list",
        ver: 1,
        reads: &[("f", "id")],
        cost: 64,
        eval: list_view,
    });
    b.seal().unwrap()
}

fn pattern(
    source: &'static str,
    key: &'static [&'static str],
    eq: &'static [(&'static str, Lit)],
) -> Pattern {
    Pattern { source, key, eq }
}

fn row(
    name: &'static str,
    prio: u8,
    by: Option<&'static str>,
    owes: Pattern,
    until: &[Pattern],
    unless: &[Pattern],
) -> Decl {
    // 用例里的出口是现场拼的切片，`Decl` 要 `'static`——测试进程内泄漏可忽略。
    let leak = |p: &[Pattern]| Box::leak(p.to_vec().into_boxed_slice()) as &'static [Pattern];
    Decl {
        name,
        prio,
        by,
        tier: LatencyTier::Reflex,
        owes,
        until: leak(until),
        unless: leak(unless),
    }
}

// ── 开账 / 关账 / 挂起 ───────────────────────────────────────────────────

#[test]
fn open_debt_needs_owes_and_no_same_key_until() {
    let rows = [row(
        "r",
        10,
        Some("act"),
        pattern("f", &["id"], &[("x", Lit::Str("go"))]),
        &[pattern("g", &["id"], &[])],
        &[],
    )];
    let events = vec![
        ev(1, 1, "f", json!({"id": "a", "x": "go"})),
        ev(2, 1, "f", json!({"id": "b", "x": "go"})),
        // 不满足 eq 的实例不开账。
        ev(3, 1, "f", json!({"id": "c", "x": "stop"})),
        // 只关掉同键那笔。
        ev(4, 1, "g", json!({"id": "a"})),
    ];
    let debts = owed(&rows, &world(), &events, 0);
    assert_eq!(debts.len(), 1);
    assert_eq!(debts[0].key, Key(vec![("id".into(), "b".into())]));
    assert!(!debts[0].blocked);
}

#[test]
fn a_fact_without_until_keeps_the_debt_open_forever() {
    // 空 `until` 在装载期就报；即便硬塞进求解，它也仍然是一笔永远开着的账。
    let rows = [row(
        "r",
        10,
        Some("act"),
        pattern("f", &["id"], &[]),
        &[],
        &[],
    )];
    let events = vec![ev(1, 1, "f", json!({"id": "a"}))];
    assert_eq!(owed(&rows, &world(), &events, 0).len(), 1);
}

#[test]
fn unless_blocks_without_closing() {
    let rows = [row(
        "r",
        10,
        Some("act"),
        pattern("f", &["id"], &[]),
        &[pattern("g", &["id"], &[])],
        &[pattern("h", &["id"], &[])],
    )];
    let events = vec![
        ev(1, 1, "f", json!({"id": "a"})),
        ev(2, 1, "h", json!({"id": "a"})),
    ];
    let debts = owed(&rows, &world(), &events, 0);
    assert_eq!(debts.len(), 1);
    assert!(debts[0].blocked, "`unless` 命中 ⇒ 在等，而不是没事");
    assert!(drive(&rows, &world(), &events, 0).is_none());
}

#[test]
fn by_none_is_the_declared_waiting_state() {
    let rows = [row(
        "r",
        10,
        None,
        pattern("f", &["id"], &[]),
        &[pattern("g", &["id"], &[])],
        &[],
    )];
    let events = vec![ev(1, 1, "f", json!({"id": "a"}))];
    let debts = owed(&rows, &world(), &events, 0);
    assert_eq!(debts.len(), 1);
    assert!(debts[0].blocked);
    assert!(drive(&rows, &world(), &events, 0).is_none());
}

// ── 视图当源 ─────────────────────────────────────────────────────────────

#[test]
fn views_are_expanded_into_instances() {
    let obj_rows = [row(
        "obj",
        10,
        Some("act"),
        pattern("v", &["id"], &[("ready", Lit::Bool(true))]),
        &[pattern("g", &["id"], &[])],
        &[],
    )];
    let events = vec![ev(1, 1, "f", json!({"id": "a"}))];
    let frozen = world();
    assert!(
        owed(&obj_rows, &frozen, &events, 99).is_empty(),
        "`now` 未到"
    );
    let debts = owed(&obj_rows, &frozen, &events, 100);
    assert_eq!(debts.len(), 1, "`now` 由入参折成视图字段");
    assert_eq!(debts[0].key, Key(vec![("id".into(), "a".into())]));

    let list_rows = [row(
        "list",
        10,
        Some("act"),
        pattern("v_list", &["id"], &[]),
        &[pattern("g", &["id"], &[])],
        &[],
    )];
    let events = vec![
        ev(1, 1, "f", json!({"id": "a"})),
        ev(2, 1, "f", json!({"id": "b"})),
    ];
    assert_eq!(owed(&list_rows, &frozen, &events, 0).len(), 2);
}

#[test]
fn a_null_view_has_no_instances() {
    let rows = [row(
        "r",
        10,
        Some("act"),
        pattern("v", &["id"], &[]),
        &[pattern("g", &["id"], &[])],
        &[],
    )];
    // 没有 `f` ⇒ 对象视图返回 null ⇒ 没有实例（不是一笔空键的账）。
    assert!(owed(&rows, &world(), &[], 0).is_empty());
}

// ── 裁决序 ───────────────────────────────────────────────────────────────

#[test]
fn drive_takes_min_by_prio_then_name_then_key() {
    let low = row(
        "b",
        10,
        Some("b"),
        pattern("f", &["id"], &[]),
        &[pattern("g", &["id"], &[])],
        &[],
    );
    let earlier = row(
        "a",
        10,
        Some("a"),
        pattern("f", &["id"], &[]),
        &[pattern("g", &["id"], &[])],
        &[],
    );
    let later = row(
        "z",
        20,
        Some("z"),
        pattern("f", &["id"], &[]),
        &[pattern("g", &["id"], &[])],
        &[],
    );
    let events = vec![
        ev(1, 1, "f", json!({"id": "b"})),
        ev(2, 1, "f", json!({"id": "a"})),
    ];
    let frozen = world();
    // 同 `prio` 按行名；注册顺序相反也不影响。
    let rows = [low.clone(), earlier.clone(), later.clone()];
    let picked = drive(&rows, &frozen, &events, 0).unwrap();
    assert_eq!(picked.decl.name, "a");

    let rows = [later.clone(), earlier.clone(), low.clone()];
    assert_eq!(drive(&rows, &frozen, &events, 0).unwrap().decl.name, "a");
}

#[test]
fn same_row_same_prio_breaks_ties_by_key() {
    let rows = [row(
        "r",
        10,
        Some("act"),
        pattern("f", &["id"], &[]),
        &[pattern("g", &["id"], &[])],
        &[],
    )];
    let events = vec![
        ev(1, 1, "f", json!({"id": "b"})),
        ev(2, 1, "f", json!({"id": "a"})),
    ];
    let picked = drive(&rows, &world(), &events, 0).unwrap();
    assert_eq!(picked.key.trace(), "id=a");
}

#[test]
fn empty_table_never_drives_but_is_complete() {
    let events = vec![ev(1, 1, "f", json!({"id": "a"}))];
    assert!(owed(&[], &world(), &events, 0).is_empty());
    assert!(drive(&[], &world(), &events, 0).is_none());
}

// ── 纯性 ─────────────────────────────────────────────────────────────────

#[test]
fn same_slice_same_now_same_debts() {
    let rows = [row(
        "r",
        10,
        Some("act"),
        pattern("v_list", &["id"], &[]),
        &[pattern("g", &["id"], &[])],
        &[],
    )];
    let events = vec![
        ev(1, 1, "f", json!({"id": "a"})),
        ev(2, 1, "f", json!({"id": "b"})),
    ];
    let frozen = world();
    let first: Vec<_> = owed(&rows, &frozen, &events, 500)
        .into_iter()
        .map(|d| (d.decl.name, d.key.trace(), d.blocked))
        .collect();
    let second: Vec<_> = owed(&rows, &frozen, &events, 500)
        .into_iter()
        .map(|d| (d.decl.name, d.key.trace(), d.blocked))
        .collect();
    assert_eq!(first, second);
    assert_eq!(first.len(), 2);
}

// ── 装载期检查 ───────────────────────────────────────────────────────────

#[test]
fn check_rows_finds_every_kind_of_gap() {
    let rows = [
        // 无出口
        row(
            "no-exit",
            10,
            Some("x"),
            pattern("f", &["id"], &[]),
            &[],
            &[],
        ),
        // 出口键与开账键不一致
        row(
            "key-mismatch",
            20,
            Some("x"),
            pattern("f", &["id"], &[]),
            &[pattern("g", &[], &[])],
            &[],
        ),
        // 未声明的源
        row(
            "unknown",
            30,
            Some("x"),
            pattern("ghost", &["id"], &[]),
            &[pattern("g", &["id"], &[])],
            &[],
        ),
        // 重名
        row(
            "unknown",
            40,
            Some("x"),
            pattern("f", &["id"], &[]),
            &[pattern("g", &["id"], &[])],
            &[],
        ),
    ];
    let gaps = check_rows(&rows, &world());
    assert!(gaps.contains(&Gap::NoExit { row: "no-exit" }));
    assert!(gaps.contains(&Gap::KeyMismatch {
        row: "key-mismatch"
    }));
    assert!(gaps.contains(&Gap::UnknownSource {
        row: "unknown",
        source: "ghost"
    }));
    assert!(gaps.contains(&Gap::DuplicateName { row: "unknown" }));
    // `h` 没有被任何行引用：拼错名字的静默形态在这里被点名。
    assert!(gaps.contains(&Gap::OrphanFact { fact: "h" }));
}

#[test]
fn a_complete_row_set_has_no_gaps() {
    let rows = [row(
        "r",
        10,
        Some("act"),
        pattern("f", &["id"], &[]),
        &[pattern("g", &["id"], &[]), pattern("h", &["id"], &[])],
        &[],
    )];
    assert!(check_rows(&rows, &world()).is_empty());
}
