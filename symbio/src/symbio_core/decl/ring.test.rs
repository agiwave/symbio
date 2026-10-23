//! 义务环求解的单元用例：模式匹配 / 开账与关账 / 挂起 / 全局裁决序 / 纯性 / 装载期检查。
//!
//! 反向用例是主菜：一笔永远不会关的账、一个拼错的源名，都必须在装载期或求解时
//! **说出来**，而不是静默地永不命中。

use serde_json::{json, Value};

use super::super::{ActorDecl, Builder, FactDecl, ViewDecl};
use super::*;
use crate::symbio_core::{Entity, Event, LatencyTier, Seq, Verb};
use std::sync::Arc;

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

/// 用例世界里的唯一执行体：`act`。行的 `by` 只有指向它才算「有人跑」。
/// 写的两个事实正好覆盖各用例的出口（`g` / `h`），`done` 给批量世界用。
const ACT: ActorDecl = ActorDecl {
    name: "act",
    mode: LatencyTier::Deep,
    reads: &[],
    writes: &["g", "h", "done"],
};

/// 用例世界的**声明集**：`f` / `g` / `h` 三个事实与两个视图，加上这一用例的执行体。
///
/// 这里**不带**归宿行：第 7 条「声明事实有归宿」正是下面一批反向用例的被测面，
/// 夹具先替它们把归宿补好，那些用例就永远测不到东西。需要一份「只用来求解」的
/// 表时用 [`decl_world`]，它按事实各补一行。
fn declare(actors: &[ActorDecl]) -> Builder {
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
    for d in actors {
        b.actor(*d);
    }
    b
}

/// 只为了给一个声明事实**归宿**、不参与求解的一行（`by = None`）。
fn home(fact: &'static str) -> Decl {
    let exit: &'static [Pattern] = Box::leak(Box::new([pattern(fact, &[], &[])]));
    Decl {
        name: fact,
        prio: 99,
        by: None,
        tier: LatencyTier::Reflex,
        expand: None,
        owes: exit[0],
        until: exit,
        unless: &[],
    }
}

/// **装载只有一个出口**（DE-015 §3.3）：五张表灌进六个注册口，`seal()` 跑完全部
/// 检查。所以反向用例断言的是这个 `Err`，正例断言 `Ok`——不再有「先冻结、再自己想
/// 起来跑一遍行检查」这种两段式。
fn freeze(rows: &[Decl], actors: &[ActorDecl]) -> Result<Arc<Frozen>, Vec<Gap>> {
    let mut b = declare(actors);
    for r in rows {
        b.row(r.clone());
    }
    b.seal()
}

/// 反向用例的断言面：`seal()` 报出的缺口（装载成功 ⇒ 空表）。
fn seal_gaps(rows: &[Decl], actors: &[ActorDecl]) -> Vec<Gap> {
    freeze(rows, actors).err().unwrap_or_default()
}

/// 求解用例的**声明面**：事实与视图。每个事实补一行归宿才装得起来——`owed` /
/// `drive` 拿的是各用例自己那份 `rows`，这张表只负责「名字与视图存在」，所以
/// 归宿行写成 `by = None`，一个执行体也不注册。
fn decl_world() -> Arc<Frozen> {
    let mut b = declare(&[]);
    for name in ["f", "g", "h"] {
        b.row(home(name));
    }
    b.seal().expect("用例的声明集必须能冻结")
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
        expand: None,
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
    let debts = owed(&rows, &decl_world(), &events, 0);
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
    assert_eq!(owed(&rows, &decl_world(), &events, 0).len(), 1);
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
    let debts = owed(&rows, &decl_world(), &events, 0);
    assert_eq!(debts.len(), 1);
    assert!(debts[0].blocked, "`unless` 命中 ⇒ 在等，而不是没事");
    assert!(drive(&rows, &decl_world(), &events, 0).is_none());
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
    let debts = owed(&rows, &decl_world(), &events, 0);
    assert_eq!(debts.len(), 1);
    assert!(debts[0].blocked);
    assert!(drive(&rows, &decl_world(), &events, 0).is_none());
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
    let frozen = decl_world();
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
    assert!(owed(&rows, &decl_world(), &[], 0).is_empty());
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
    let frozen = decl_world();
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
    let picked = drive(&rows, &decl_world(), &events, 0).unwrap();
    assert_eq!(picked.key.trace(), "id=a");
}

#[test]
fn empty_table_never_drives_but_is_complete() {
    let events = vec![ev(1, 1, "f", json!({"id": "a"}))];
    assert!(owed(&[], &decl_world(), &events, 0).is_empty());
    assert!(drive(&[], &decl_world(), &events, 0).is_none());
}

/// 守卫用例的替身（D-118 订正 1）：「`by` 查不到动作」不再是求解时静默跳过，
/// 而是装载期就红。`accept` 谓词存在时，这一行会永远不被选中却全绿；现在它拒绝启动。
#[test]
fn a_by_without_an_actor_is_a_load_time_gap() {
    let rows = [row(
        "unwired",
        0,
        Some("nobody"),
        pattern("f", &["id"], &[]),
        &[pattern("g", &["id"], &[])],
        &[],
    )];
    let gaps = seal_gaps(&rows, &[ACT]);
    assert!(gaps.contains(&Gap::UnmannedRow {
        row: "unwired",
        by: "nobody"
    }));
    // 反方向同样要红：声明了执行体却没有行指向它 = 裸函数换了个写法。
    assert!(gaps.contains(&Gap::OrphanActor { actor: "act" }));
}

/// `by = None` 是「这笔账不由引擎执行」的唯一声明位：它不是缺口，
/// 但它也不该带来一个执行体。
#[test]
fn a_none_by_needs_no_actor() {
    let rows = [row(
        "waiting",
        0,
        None,
        pattern("f", &["id"], &[]),
        &[pattern("g", &["id"], &[]), pattern("h", &["id"], &[])],
        &[],
    )];
    assert!(seal_gaps(&rows, &[]).is_empty());
}

/// 执行体写的名字必须够关上自己那一行的出口（第 6 条收口）。
/// 否则「有人跑」也只是跑得离那笔债更近不了——装载期就该指出。
#[test]
fn an_actor_that_writes_nothing_on_the_exit_is_a_gap() {
    let rows = [row(
        "r",
        10,
        Some("act"),
        pattern("f", &["id"], &[]),
        &[pattern("g", &["id"], &[])],
        &[],
    )];
    let gaps = seal_gaps(
        &rows,
        &[ActorDecl {
            name: "act",
            mode: LatencyTier::Deep,
            reads: &[],
            writes: &["z"],
        }],
    );
    assert!(gaps.contains(&Gap::ActorCannotClose {
        row: "r",
        actor: "act"
    }));
}

/// 第 6 条的第二档（D-124）：出口**全是**引擎独占格的行——派发行的形状。
///
/// 交集判不了它：那等于要求执行体声明一条第 5 条禁止它声明的格，两条检查互斥，
/// 这一行永远只能 `by: None`（`dispatch-turn` / `dispatch-model` / `dispatch-tools`
/// 正是这个形状，DE-015 §3.1 要给它们接 `courier`）。改判成「名单非空」之后，
/// **空名单仍然要红**——这一档不是「不判」。
#[test]
fn an_actor_serving_an_engine_exit_row_must_still_declare_writes() {
    let gaps_for = |writes: &'static [&'static str]| -> Vec<Gap> {
        let mut b = declare(&[ActorDecl {
            name: "act",
            mode: LatencyTier::Deep,
            reads: &[],
            writes,
        }]);
        // 派发行按**端口**开账，夹具世界照生产那一格补上 `sent` 与它的字段。
        b.fact(FactDecl {
            name: "sent",
            port: Some("port"),
            key: &["id", "attempt"],
            expand: &[],
            fields: &["port", "id", "attempt", "lease_ms", "attempt_cap"],
        });
        b.row(home("f"));
        b.row(home("sent"));
        b.row(row(
            "r",
            10,
            Some("act"),
            pattern("f", &["id"], &[]),
            &[pattern("sent", &["id"], &[])],
            &[],
        ));
        b.seal().err().unwrap_or_default()
    };
    let gap = Gap::ActorCannotClose {
        row: "r",
        actor: "act",
    };
    assert!(
        !gaps_for(&["g"]).contains(&gap),
        "出口全为引擎格、名单登记了领域格 ⇒ 派发行可接线（第二档判得住）"
    );
    assert!(
        gaps_for(&[]).contains(&gap),
        "同样这一行，名单为空 ⇒ 仍然红：第二档不是豁免，是换了判的那一半"
    );
}

/// 时延闸门：行的 `tier` 比执行体的 `mode` 更慢 ⇒ 这一行不可能按时跑完。
/// （反射档拿不到模型句柄——DE-015 §3.4 第 6 条的后半。）
#[test]
fn a_row_slower_than_its_actor_mode_is_a_gap() {
    let rows = {
        let mut d = row(
            "r",
            10,
            Some("act"),
            pattern("f", &["id"], &[]),
            &[pattern("g", &["id"], &[])],
            &[],
        );
        d.tier = LatencyTier::Deep;
        [d]
    };
    assert!(seal_gaps(
        &rows,
        &[ActorDecl {
            name: "act",
            mode: LatencyTier::Reflex,
            reads: &[],
            writes: &["g"],
        }],
    )
    .contains(&Gap::ActorModeTooNarrow {
        row: "r",
        actor: "act"
    }));
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
    let frozen = decl_world();
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
fn seal_finds_every_kind_of_gap() {
    let rows = [
        // 无出口
        row(
            "no-exit",
            10,
            Some("act"),
            pattern("f", &["id"], &[]),
            &[],
            &[],
        ),
        // 出口键与开账键不一致
        row(
            "key-mismatch",
            20,
            Some("act"),
            pattern("f", &["id"], &[]),
            &[pattern("g", &[], &[])],
            &[],
        ),
        // 未声明的源
        row(
            "unknown",
            30,
            Some("act"),
            pattern("ghost", &["id"], &[]),
            &[pattern("g", &["id"], &[])],
            &[],
        ),
        // 重名
        row(
            "unknown",
            40,
            Some("act"),
            pattern("f", &["id"], &[]),
            &[pattern("g", &["id"], &[])],
            &[],
        ),
    ];
    let gaps = seal_gaps(&rows, &[ACT]);
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
    assert!(seal_gaps(&rows, &[ACT]).is_empty());
}

// ── 端口声明─────────────────────────────────────────────────────

/// 端口的声明是**按端口一次**，不是按行三次：`before` / `lease_ms` /
/// `max_attempts` 对同一个端口的每一行都一样，抄进行里就是三份会漂移的副本。
#[test]
fn a_port_is_declared_once_and_found_by_name() {
    const PORTS: &[PortDecl] = &[
        PortDecl {
            port: "turn",
            before: "sent",
            lease_ms: 600_000,
            max_attempts: 3,
            concurrent: false,
        },
        PortDecl {
            port: "tool",
            before: "sent",
            lease_ms: 30_000,
            max_attempts: 2,
            concurrent: true,
        },
    ];
    assert_eq!(port_of(PORTS, "turn").map(|p| p.lease_ms), Some(600_000));
    assert_eq!(port_of(PORTS, "tool").map(|p| p.max_attempts), Some(2));
    // 并发许可是端口级声明的一部分，与租约 / 尝试上限同处一次声明。
    assert_eq!(port_of(PORTS, "tool").map(|p| p.concurrent), Some(true));
    // 没有这个端口 ⇒ `None`，不是默认值：漏声明要显形，不能静默拿一个默认租约。
    assert!(port_of(PORTS, "model").is_none());
}

// ── 集合展开（DE-015 §3.3）──────────────────────────────────────────────

/// 一篮子的**声明集**：`batch` 的 `calls` 声明为可展开字段。
fn declare_batch() -> Builder {
    let mut b = Builder::new();
    b.fact(FactDecl {
        name: "batch",
        port: None,
        key: &[],
        expand: &["calls"],
        fields: &["id", "calls", "turn"],
    });
    b.fact(FactDecl {
        name: "done",
        port: None,
        key: &[],
        expand: &[],
        fields: &["id"],
    });
    b.actor(ACT);
    b
}

/// 求解用例用的篮子表：一行「`batch` 欠、`done` 还」给两个事实归宿，同时让 `act`
/// 不是孤儿执行体——第 7 条与第 6 条都不许靠夹具说谎。
fn world_batch() -> Arc<Frozen> {
    let mut b = declare_batch();
    b.row(row(
        "home",
        99,
        Some("act"),
        pattern("batch", &[], &[]),
        &[pattern("done", &[], &[])],
        &[],
    ));
    b.seal().expect("篮子世界的声明集必须能冻结")
}

/// 篮子表上的装载缺口（`rows` 与声明集一起 `seal`）。
fn seal_batch(rows: &[Decl]) -> Vec<Gap> {
    let mut b = declare_batch();
    for r in rows {
        b.row(r.clone());
    }
    b.seal().err().unwrap_or_default()
}

fn row_exp(
    name: &'static str,
    expand: Option<(&'static str, &'static str)>,
    owes: Pattern,
    until: &[Pattern],
) -> Decl {
    let leak = |p: &[Pattern]| Box::leak(p.to_vec().into_boxed_slice()) as &'static [Pattern];
    Decl {
        name,
        prio: 10,
        by: Some("act"),
        tier: LatencyTier::Reflex,
        expand,
        owes,
        until: leak(until),
        unless: &[],
    }
}

/// 一个 `owes` 实例的列表逐元素开债；键取自元素，元素之间互不相同。
#[test]
fn expand_opens_one_debt_per_element() {
    let rows = [row_exp(
        "r",
        Some(("calls", "id")),
        pattern("batch", &["id"], &[]),
        &[pattern("done", &["id"], &[])],
    )];
    let events = vec![ev(
        1,
        1,
        "batch",
        json!({"id": "b1", "calls": [{"id": "c1"}, {"id": "c2"}, {"id": "c3"}]}),
    )];
    let debts = owed(&rows, &world_batch(), &events, 0);
    let keys: Vec<String> = debts.iter().map(|d| d.key.trace()).collect();
    assert_eq!(keys, vec!["id=c1", "id=c2", "id=c3"]);
}

/// 出口按键逐元素还：还掉两笔只留一笔在途。
#[test]
fn expand_until_closes_elements_one_by_one() {
    let rows = [row_exp(
        "r",
        Some(("calls", "id")),
        pattern("batch", &["id"], &[]),
        &[pattern("done", &["id"], &[])],
    )];
    let events = vec![
        ev(
            1,
            1,
            "batch",
            json!({"id": "b1", "calls": [{"id": "c1"}, {"id": "c2"}, {"id": "c3"}]}),
        ),
        ev(2, 1, "done", json!({"id": "c1"})),
        ev(3, 1, "done", json!({"id": "c3"})),
    ];
    let debts = owed(&rows, &world_batch(), &events, 0);
    let keys: Vec<String> = debts.iter().map(|d| d.key.trace()).collect();
    assert_eq!(keys, vec!["id=c2"]);
}

/// 列表字段缺失 / 不是数组 ⇒ 一笔都不开，不是「开一笔空键的债」。
#[test]
fn expand_without_the_list_opens_no_debt() {
    let rows = [row_exp(
        "r",
        Some(("calls", "id")),
        pattern("batch", &["id"], &[]),
        &[pattern("done", &["id"], &[])],
    )];
    for payload in [json!({"id": "b1"}), json!({"id": "b1", "calls": []})] {
        let events = vec![ev(1, 1, "batch", payload)];
        assert!(owed(&rows, &world_batch(), &events, 0).is_empty());
    }
}

/// 元素键字段必须在 `owes.key` 里，事实字段必须由 `FactDecl.expand` 承认。
#[test]
fn expand_gaps_are_load_time_errors() {
    let rows = [
        // `call_id` 不在 `owes.key = ["id"]` 里：元素之间分不出谁是谁。
        row_exp(
            "key-missing",
            Some(("calls", "call_id")),
            pattern("batch", &["id"], &[]),
            &[pattern("done", &["id"], &[])],
        ),
        // `nosuch` 没写进 `batch` 的 `expand` 声明。
        row_exp(
            "undeclared",
            Some(("nosuch", "id")),
            pattern("batch", &["id"], &[]),
            &[pattern("done", &["id"], &[])],
        ),
    ];
    let gaps = seal_batch(&rows);
    assert!(gaps.contains(&Gap::ExpandKeyMissing {
        row: "key-missing",
        field: "call_id"
    }));
    assert!(gaps.contains(&Gap::UndeclaredExpand {
        row: "undeclared",
        source: "batch",
        field: "nosuch"
    }));
}

/// 展开行本身是自洽的：没有 gap（正例，防上一条用例把「有 expand 就红」写死）。
#[test]
fn a_complete_expand_row_has_no_gaps() {
    let rows = [row_exp(
        "r",
        Some(("calls", "id")),
        pattern("batch", &["id"], &[]),
        &[pattern("done", &["id"], &[])],
    )];
    assert!(seal_batch(&rows).is_empty());
}

// ── 装载期七道检查的反向用例（§3.4，指认表见 [`check_rows`] 的文档）─────────

/// 第 3 条：模式引用了一个不在载荷声明里的字段。
///
/// 它与「这次没带这个字段」在求值时同形（取不到 ⇒ 不命中），所以运行期永远不会
/// 说错——只能装载期说。键与等值约束各查一遍，`role` 区分是哪一侧拼错。
#[test]
fn a_pattern_field_must_be_declared_by_the_fact() {
    let rows = [row(
        "typo",
        10,
        None,
        pattern("f", &["idea"], &[("x", Lit::Str("go"))]),
        &[pattern("g", &["id"], &[])],
        &[],
    )];
    assert!(
        seal_gaps(&rows, &[ACT]).contains(&Gap::UndeclaredPatternField {
            row: "typo",
            source: "f",
            field: "idea",
            role: "key",
        })
    );

    let rows = [row(
        "typo",
        10,
        None,
        pattern("f", &["id"], &[("em", Lit::Str("go"))]),
        &[pattern("g", &["id"], &[])],
        &[],
    )];
    assert!(
        seal_gaps(&rows, &[ACT]).contains(&Gap::UndeclaredPatternField {
            row: "typo",
            source: "f",
            field: "em",
            role: "eq",
        })
    );
}

/// 第 3 条的例外：信封三格每条事实都有，不该逐条声明（判它红就是把机制当错误）。
///
/// 夹具世界 `world()` 只声明 `f` / `g` / `h` 与一个执行体，一张两行的表本来就会报
/// 孤儿事实之类的缺口；这里只要求**第 3 条不响**——别把别的缺口算进这条用例。
#[test]
fn the_envelope_fields_need_no_declaration() {
    let rows = [row(
        "by-turn",
        10,
        None,
        pattern("f", &["turn"], &[]),
        &[pattern("g", &["turn", "seq", "ts"], &[])],
        &[],
    )];
    let gaps = seal_gaps(&rows, &[ACT]);
    assert!(
        !gaps
            .iter()
            .any(|g| matches!(g, Gap::UndeclaredPatternField { .. })),
        "信封三格不该要求逐条声明：{gaps:?}"
    );
}

/// 第 1 条：两行互相等对方先跑——保守依赖图上有环。
///
/// `one` 写 `g`（`r2` 的出口），`two` 写 `h`（`r1` 的出口）：每一行都「开着等另一行」，
/// 而另一行也在等它，于是没有任何一行能被派发到底。运行期的形状是每一步无进展。
#[test]
fn a_cycle_in_the_who_settles_whom_graph_is_a_gap() {
    const ONE: ActorDecl = ActorDecl {
        name: "one",
        mode: LatencyTier::Deep,
        reads: &[],
        writes: &["g", "h"],
    };
    const TWO: ActorDecl = ActorDecl {
        name: "two",
        mode: LatencyTier::Deep,
        reads: &[],
        writes: &["g", "h"],
    };
    let rows = [
        row(
            "r1",
            10,
            Some("one"),
            pattern("f", &["id"], &[]),
            &[pattern("g", &["id"], &[])],
            &[],
        ),
        row(
            "r2",
            20,
            Some("two"),
            pattern("f", &["id"], &[]),
            &[pattern("h", &["id"], &[])],
            &[],
        ),
    ];
    assert_eq!(
        seal_gaps(&rows, &[ONE, TWO]),
        vec![Gap::DependencyCycle {
            cycle: vec!["r1", "r2", "r1"]
        }]
    );
}

/// 自环不算环：一行自己写自己的出口正是正常形状（执行体干活把这一行关掉）。
/// 把自环算成环会让每一张真表都红，所以这一条是**正例**——只要求第 1 条不响。
#[test]
fn a_row_that_closes_itself_is_not_a_cycle() {
    let rows = [row(
        "r",
        10,
        Some("act"),
        pattern("f", &["id"], &[]),
        &[pattern("g", &["id"], &[])],
        &[],
    )];
    let gaps = seal_gaps(&rows, &[ACT]);
    assert!(
        !gaps
            .iter()
            .any(|g| matches!(g, Gap::DependencyCycle { .. })),
        "自环不是互相等待：{gaps:?}"
    );
}

/// 第 4 条：可能同时开账的行对**只报告**。
///
/// 静态判不出「会不会真的同时命中」（那要读日志），所以这里不红；但遮蔽必须在装载
/// 期可见——两行认同一笔债而 `prio` 相同，日志里看不出谁被选中，直到它静默不发生。
#[test]
fn overlapping_rows_are_reported_not_rejected() {
    let rows = [
        row(
            "r1",
            10,
            None,
            pattern("f", &["id"], &[]),
            &[pattern("g", &["id"], &[])],
            &[],
        ),
        row(
            "r2",
            20,
            None,
            pattern("f", &["id"], &[]),
            &[pattern("h", &["id"], &[])],
            &[],
        ),
    ];
    // 第 4 条是**报告**，只在装载成功的表上算得出：这里两张行都自洽 ⇒ `seal` 通过，
    // 名单就挂在冻结后的表上（读侧不再自己算第二遍）。
    let frozen = freeze(&rows, &[]).expect("相交不是缺口：第 4 条只报告");
    assert_eq!(
        frozen.overlaps(),
        vec![Overlap {
            rows: ["r1", "r2"],
            facts: vec!["f"],
        }]
    );
}

/// 第 4 条不报噪音：键不同、或等值约束互斥的行对永不同时命中。
#[test]
fn rows_that_cannot_collide_are_not_reported() {
    let rows = [
        row(
            "by-id",
            10,
            None,
            pattern("f", &["id"], &[]),
            &[pattern("g", &["id"], &[])],
            &[],
        ),
        row(
            "by-turn",
            20,
            None,
            pattern("f", &["turn"], &[]),
            &[pattern("g", &["turn"], &[])],
            &[],
        ),
        // 同一事实同一键，但 `x` 上两个不同字面量：不可能同时成立。
        row(
            "go",
            30,
            None,
            pattern("f", &["id"], &[("x", Lit::Str("go"))]),
            &[pattern("g", &["id"], &[])],
            &[],
        ),
    ];
    // `h` 在这一批行里没人读——它不归第 4 条管，补一行归宿让它别把装载判死。
    let mut b = declare(&[]);
    b.row(home("h"));
    for r in rows {
        b.row(r.clone());
    }
    let frozen = b.seal().expect("相交报告只在装载成功的表上算");
    assert_eq!(
        frozen.overlaps(),
        vec![Overlap {
            rows: ["by-id", "go"],
            facts: vec!["f"],
        }]
    );
}

/// 端口检查夹具：协议三格（`sent` / `response` / `failed`）按**端口字段**声明，
/// 再与这一用例的行一起 `seal`。
///
/// 端口的第 5 条不再单点调用 `check_ports`——它和其余六道一样只在 [`Builder::seal`]
/// 里跑，所以这里断言的缺口来自同一个装载出口（§3.3「检查的出口只有一个」）。
fn seal_protocol(rows: &[Decl], port: PortDecl) -> Vec<Gap> {
    let mut b = Builder::new();
    for name in [EVENT_SENT, EVENT_RESPONSE, EVENT_FAILED] {
        b.fact(FactDecl {
            name,
            port: Some("port"),
            key: &["id"],
            expand: &[],
            fields: &["port", "id", "attempt"],
        });
    }
    b.port(port);
    for r in rows {
        b.row(r.clone());
    }
    b.seal().err().unwrap_or_default()
}

fn port_x() -> PortDecl {
    PortDecl {
        port: "x",
        before: EVENT_SENT,
        lease_ms: 1_000,
        max_attempts: 1,
        concurrent: false,
    }
}

/// 第 5 条的端口侧：应答与失败两格都得落在某行的出口上。
///
/// 缺 `failed` 的形状是「这笔调用永远不会成功也永远不会判死」——`await` 行开着，
/// 谁都不派发它（`by = None`），而它挡住「本轮收束」，最后由停机判据兜掉。
#[test]
fn a_port_without_a_failure_exit_is_a_gap() {
    let rows = [row(
        "await-x",
        70,
        None,
        pattern(EVENT_SENT, &["id"], &[("port", Lit::Str("x"))]),
        &[pattern(EVENT_RESPONSE, &["id"], &[("port", Lit::Str("x"))])],
        &[],
    )];
    assert_eq!(
        seal_protocol(&rows, port_x()),
        vec![
            // 同一件事的两面：`failed` 没有行的出口读它 ⇒ 既没归宿（第 7 条），
            // 也没有还的通道（第 5 条）。装载一次，两个名字一起说。
            Gap::OrphanFact { fact: EVENT_FAILED },
            Gap::PortWithoutAnswerExit {
                port: "x",
                fact: EVENT_FAILED
            },
        ]
    );
}

/// 两格都覆盖 ⇒ 没有缺口（正例，防上一条把「有行就红」写死）。
#[test]
fn a_port_with_both_answer_exits_loads() {
    let rows = [row(
        "await-x",
        70,
        None,
        pattern(EVENT_SENT, &["id"], &[("port", Lit::Str("x"))]),
        &[
            pattern(EVENT_RESPONSE, &["id"], &[("port", Lit::Str("x"))]),
            pattern(EVENT_FAILED, &["id"], &[("port", Lit::Str("x"))]),
        ],
        &[],
    )];
    assert!(seal_protocol(&rows, port_x()).is_empty());
}

/// 覆盖必须是**这个端口**的：只写了别的端口的应答格不算。
#[test]
fn another_ports_answer_does_not_cover_this_one() {
    let rows = [row(
        "await-x",
        70,
        None,
        pattern(EVENT_SENT, &["id"], &[("port", Lit::Str("x"))]),
        &[
            pattern(EVENT_RESPONSE, &["id"], &[("port", Lit::Str("y"))]),
            pattern(EVENT_FAILED, &["id"], &[("port", Lit::Str("y"))]),
        ],
        &[],
    )];
    assert_eq!(
        seal_protocol(&rows, port_x()),
        vec![
            Gap::PortWithoutAnswerExit {
                port: "x",
                fact: EVENT_RESPONSE
            },
            Gap::PortWithoutAnswerExit {
                port: "x",
                fact: EVENT_FAILED
            },
        ]
    );
}

/// 派发前格（`before`）也必须是声明过的事实名——它是引擎写的，名字拼错就是
/// 「派出去了而日志里没有」。
#[test]
fn a_ports_before_cell_must_be_declared() {
    let rows = [row(
        "await-x",
        70,
        None,
        pattern(EVENT_SENT, &["id"], &[("port", Lit::Str("x"))]),
        &[
            pattern(EVENT_RESPONSE, &["id"], &[("port", Lit::Str("x"))]),
            pattern(EVENT_FAILED, &["id"], &[("port", Lit::Str("x"))]),
        ],
        &[],
    )];
    let mut port = port_x();
    port.before = "dispatched";
    assert_eq!(
        seal_protocol(&rows, port),
        vec![Gap::PortBeforeUndeclared {
            port: "x",
            fact: "dispatched"
        }]
    );
}
