//! 声明层（内核）的单元用例：装载 / 冻结 / 声明侧的三道检查。
//!
//! 控制面事实与视图的口径用例住在它们的 owner 那里
//! （`plugins::loop::control_decl`）；本文件只测**机制**：内核只声明协议五格，
//! 插件的六张表经 `Builder::declare` 一次交齐，`seal` 是唯一出口。

use serde_json::{json, Value};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::{Entity, Seq, Verb};

use super::*;

/// 未入库事件默认 `seq = None`；视图的幕锚 / 阶段折叠读的是确切的序号，
/// 因此用例里的每条事件都按**调用顺序**分配一个递增序号。
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

/// 一行**只为归宿**存在的行（§3.4 第 7 条的最小形状）：`by` 为 `None` 时它不由引擎
/// 跑，只回答「这个事实名有人管」；出口写成它自己，键与等值都不绑。
fn cover(fact: &'static str, by: Option<&'static str>) -> Decl {
    let exits: &'static [ring::Pattern] = Box::leak(Box::new([ring::Pattern {
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

/// `seal()` 报出的缺口（装载成功 ⇒ 空表）。
fn gaps_of(b: Builder) -> Vec<Gap> {
    b.seal().err().unwrap_or_default()
}

/// 只回答「签名是 `fn(&[Event], i64)`」的探针：机制用例不关心视图算什么。
fn probe(_: &[Event], _: i64) -> Value {
    json!([])
}

fn request_fact() -> FactDecl {
    FactDecl {
        name: EVENT_REQUEST,
        port: Some("port"),
        key: &["id"],
        expand: &[],
        fields: &["port", "id"],
    }
}

fn probe_view(name: &'static str, reads: &'static [(&'static str, &'static str)]) -> ViewDecl {
    ViewDecl {
        name,
        ver: 1,
        reads,
        evidence: &[EVENT_REQUEST],
        cost: 1,
        eval: probe,
    }
}

// ── 装载 ─────────────────────────────────────────────────────────────────

#[test]
fn kernel_declares_only_the_protocol_cells() {
    let mut b = Builder::new();
    for name in [
        EVENT_REQUEST,
        EVENT_SENT,
        EVENT_RESPONSE,
        EVENT_FAILED,
        EVENT_RETRY,
    ] {
        b.row(cover(name, None));
    }
    let frozen = b.seal().expect("协议五格必须能冻结");
    let facts: Vec<_> = frozen.facts().iter().map(|f| f.name).collect();
    assert_eq!(
        facts,
        vec![
            EVENT_REQUEST,
            EVENT_SENT,
            EVENT_RESPONSE,
            EVENT_FAILED,
            EVENT_RETRY,
        ]
    );
    assert!(frozen.views().is_empty(), "内核不声明任何业务视图");
    assert!(frozen.actors().is_empty(), "内核不声明任何执行体");
    for name in facts {
        assert_eq!(frozen.fact(name).unwrap().port, Some("port"), "{name}");
        assert_eq!(
            frozen.fact_owner(name),
            Some(KERNEL_OWNER),
            "协议五格归内核，owner 清单不允许出现无名条目"
        );
    }
}

/// 协议五格的事件行与内核网格同源：迁移期两份表并存，必须先钉住它们不相漂。
#[test]
fn protocol_events_match_the_kernel_grid() {
    let mut b = Builder::new();
    for name in [
        EVENT_REQUEST,
        EVENT_SENT,
        EVENT_RESPONSE,
        EVENT_FAILED,
        EVENT_RETRY,
    ] {
        b.row(cover(name, None));
    }
    let frozen = b.seal().expect("协议五格必须能冻结");
    for e in PROTOCOL_EVENTS {
        assert_eq!(frozen.event_cell(e.name), Some(e.cell));
        assert_eq!(
            frozen.event_owner(e.name),
            Some(KERNEL_OWNER),
            "协议五格归内核"
        );
    }
}

/// 插件声明的事实必须在事件表上有格位：否则写闸只能拒绝，且缺口在装载期就该红。
#[test]
fn a_plugin_fact_without_an_event_is_a_gap() {
    const NAME: &str = "fixture.no_event";
    let facts: &'static [FactDecl] = Box::leak(
        vec![FactDecl {
            name: NAME,
            port: None,
            key: &[],
            expand: &[],
            fields: &["id"],
        }]
        .into_boxed_slice(),
    );
    let rows: &'static [Decl] = Box::leak(vec![cover(NAME, None)].into_boxed_slice());
    let mut b = Builder::new();
    b.declare(
        "plugins/fixture",
        PluginDeclarations {
            events: &[],
            facts,
            views: &[],
            ports: &[],
            actors: Vec::new(),
            rows,
        },
    );
    let gaps = gaps_of(b);
    assert!(
        gaps.contains(&Gap::FactWithoutEvent { fact: NAME }),
        "{gaps:?}"
    );
}

/// `declare` 的第一个参数就是 owner：事实与视图逐条登记，同一张表不产生两个答案。
#[test]
fn declare_records_the_owner_of_each_fact_and_view() {
    let mut b = Builder::new();
    for name in [
        EVENT_REQUEST,
        EVENT_SENT,
        EVENT_RESPONSE,
        EVENT_FAILED,
        EVENT_RETRY,
    ] {
        b.row(cover(name, None));
    }
    b.declare(
        "plugins/example",
        PluginDeclarations {
            events: &[],
            facts: &[],
            views: Box::leak(
                vec![probe_view("owned-view", &[(EVENT_REQUEST, "id")])].into_boxed_slice(),
            ),
            ports: &[],
            actors: Vec::new(),
            rows: &[],
        },
    );
    let frozen = b.seal().expect("owner 登记不改变任何形状判据");
    assert_eq!(frozen.view_owner("owned-view"), Some("plugins/example"));
    for name in [
        EVENT_REQUEST,
        EVENT_SENT,
        EVENT_RESPONSE,
        EVENT_FAILED,
        EVENT_RETRY,
    ] {
        assert_eq!(frozen.fact_owner(name), Some(KERNEL_OWNER), "{name}");
    }
}

/// §3.2 的扩展面只有两个口：`declare` 交六张表，`seal` 消耗 Builder。
///
/// 断言取的是**方法指针**——类型必须逐字对上：`declare` 借 `&mut self`，`seal`
/// 收 `self`。后者是「运行期不能注册」真正落地的那一格：冻结之后连一个
/// `&mut Builder` 都造不出来。
#[test]
fn the_two_registration_ports_are_the_whole_surface() {
    let _declare: fn(&mut Builder, &'static str, PluginDeclarations<'_>) = Builder::declare;
    let _seal: fn(Builder) -> Result<Arc<Frozen>, Vec<Gap>> = Builder::seal;
}

#[test]
fn every_protocol_reference_is_a_field() {
    // `port` / `key` / `expand` 都是字段引用；声明不能自相矛盾。
    let mut b = Builder::new();
    for name in [
        EVENT_REQUEST,
        EVENT_SENT,
        EVENT_RESPONSE,
        EVENT_FAILED,
        EVENT_RETRY,
    ] {
        b.row(cover(name, None));
    }
    let frozen = b.seal().unwrap();
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
// 混进同一份 `Err` 里，而这里断言的是**声明自己**的那一格。

#[test]
fn view_cannot_read_an_undeclared_field() {
    let mut b = Builder::empty();
    b.fact(request_fact());
    b.row(cover(EVENT_REQUEST, None));
    b.view(probe_view("probe", &[(EVENT_REQUEST, "attmpt")]));
    assert_eq!(
        gaps_of(b),
        vec![Gap::UndeclaredField {
            view: "probe",
            fact: EVENT_REQUEST,
            field: "attmpt",
        }]
    );
}

#[test]
fn view_cannot_read_an_undeclared_fact() {
    let mut b = Builder::empty();
    b.view(ViewDecl {
        name: "probe",
        ver: 1,
        reads: &[("ghost", "id")],
        evidence: &[],
        cost: 1,
        eval: probe,
    });
    assert_eq!(
        gaps_of(b),
        vec![Gap::UnknownFact {
            view: "probe",
            fact: "ghost",
        }]
    );
}

/// D-160：`evidence` 只能引用这个视图**读过**的事实——「它替谁关门」是「它读什么」的子集。
///
/// 这条包含关系拦的是**静默**的那一半：`evidence` 里写一个没读过的名字，依赖图就少
/// 一条边而装载照绿。假环至少红得看得见，少边要到真死锁那天才显形，所以只能在声明期拦。
#[test]
fn evidence_must_be_a_fact_the_view_reads() {
    let mut b = Builder::empty();
    b.fact(request_fact());
    b.fact(FactDecl {
        name: EVENT_SENT,
        port: Some("port"),
        key: &["id"],
        expand: &[],
        fields: &["port", "id"],
    });
    b.row(cover(EVENT_REQUEST, None));
    b.row(cover(EVENT_SENT, None));
    b.view(ViewDecl {
        name: "probe",
        ver: 1,
        reads: &[(EVENT_REQUEST, "id")],
        evidence: &[EVENT_SENT],
        cost: 1,
        eval: probe,
    });
    assert_eq!(
        gaps_of(b),
        vec![Gap::EvidenceNotRead {
            view: "probe",
            fact: EVENT_SENT
        }]
    );
}

#[test]
fn duplicate_fact_is_rejected() {
    let mut b = Builder::empty();
    b.fact(request_fact());
    b.fact(request_fact());
    b.row(cover(EVENT_REQUEST, None));
    assert_eq!(
        gaps_of(b),
        vec![Gap::DuplicateFact {
            name: EVENT_REQUEST
        }]
    );
}

#[test]
fn duplicate_view_is_rejected() {
    let mut b = Builder::empty();
    b.fact(request_fact());
    b.row(cover(EVENT_REQUEST, None));
    b.view(probe_view("probe", &[(EVENT_REQUEST, "id")]));
    b.view(probe_view("probe", &[(EVENT_REQUEST, "id")]));
    assert_eq!(gaps_of(b), vec![Gap::DuplicateView { name: "probe" }]);
}

#[test]
fn fact_and_view_share_one_name_space() {
    let mut b = Builder::empty();
    b.fact(request_fact());
    b.row(cover(EVENT_REQUEST, None));
    b.view(probe_view(EVENT_REQUEST, &[(EVENT_REQUEST, "id")]));
    assert_eq!(
        gaps_of(b),
        vec![Gap::NameCollision {
            name: EVENT_REQUEST
        }]
    );
}

#[test]
fn eval_respects_the_declared_cost() {
    // `cost` 是声明的扫描上界；超界返回 `None`，不静默算一半。
    let mut b = Builder::empty();
    let fields: &'static [&'static str] = &["port", "id"];
    b.fact(FactDecl {
        name: EVENT_SENT,
        port: Some("port"),
        key: &["id"],
        expand: &[],
        fields,
    });
    b.row(cover(EVENT_SENT, None));
    b.view(ViewDecl {
        name: "probe",
        ver: 1,
        reads: &[(EVENT_SENT, "id")],
        evidence: &[EVENT_SENT],
        cost: 1,
        eval: probe,
    });
    let frozen = b.seal().unwrap();
    let one = vec![ev(
        1,
        EVENT_SENT,
        json!({"port": "turn", "id": "a", "attempt": 1}),
    )];
    assert!(frozen.eval("probe", &one, 0).is_some());
    let two = vec![
        one[0].clone(),
        ev(
            1,
            EVENT_SENT,
            json!({"port": "turn", "id": "b", "attempt": 1}),
        ),
    ];
    assert_eq!(frozen.eval("probe", &two, 0), None);
}

// ── 切点：一份切片喂给每一条视图（`scan_bound` / `ViewCostMismatch`）─────────
//
// 这一段测的是 D-222 订正 1 的终态。判据是「`cost` 超界不许静默变成『没有欠账』」，
// 而 `eval` 回 `None`、`owed` 折成空集（`_ => Vec::new()`，字节冻结）。
// 所以缺口只能从两头收：**切点只有一个真源**（`scan_bound` 取最小值），
// **声明的分歧在装载期红**（`ViewCostMismatch`）。下面三条分别钉住这两头与
// 它们在同一个数上会合。

/// 复刻 `probe_view` 但可指定 `cost`：分歧判据的夹具。
fn probe_view_costing(
    name: &'static str,
    reads: &'static [(&'static str, &'static str)],
    cost: u32,
) -> ViewDecl {
    ViewDecl {
        cost,
        ..probe_view(name, reads)
    }
}

/// 切点是**声明的那一个数**，而且它对每一条视图都成立。
///
/// 分歧由 [`Gap::ViewCostMismatch`] 在装载期挡掉（下一条用例），所以能冻结的表里
/// `cost` 只有一个值——`scan_bound` 的「最小值」在这种表上退化为「那个值」。
/// 两条合起来才是完整的判据：**不一致的表不存在**，一致的表只有一个切点。
///
/// 它钉的是「切点不是 `0`、不是某个默认值、不是第一条视图之外的东西」——
/// 取错的话调用方会把切片切空（`0`）或切得比声明更长（超界 ⇒ 静默空集）。
#[test]
fn the_scan_bound_is_the_cost_every_view_declares() {
    let mut b = Builder::empty();
    b.fact(request_fact());
    b.row(cover(EVENT_REQUEST, None));
    for name in ["alpha", "beta", "gamma"] {
        b.view(probe_view_costing(name, &[(EVENT_REQUEST, "id")], 64));
    }

    let frozen = b.seal().expect("三条视图同读一条事实、同一声明成本");
    assert_eq!(
        frozen.scan_bound(),
        Some(64),
        "一份切片喂给每一条视图 ⇒ 切点就是它们共同声明的那个上界"
    );
}

/// 一条视图都没有的表没有切点（`None`，不是 `0`）。
///
/// `0` 会让调用方把切片切空 ⇒ 每一个视图都看不见任何事实 ⇒ 「什么都没欠」。
/// 这与「没有视图可切」是两件事，返回值必须分得开。
#[test]
fn a_table_without_views_has_no_scan_bound() {
    let mut b = Builder::empty();
    b.fact(request_fact());
    b.row(cover(EVENT_REQUEST, None));
    let frozen = b.seal().expect("只有一行一事实也合法");
    assert_eq!(frozen.scan_bound(), None, "没有视图 ⇒ 没有可切的界");
}

/// 声明的 `cost` 分歧 ⇒ 装载期红，且报出**偏大的那一条**与实际的切点。
///
/// 这是 D-222 订正 1 的内核侧一半：把某条视图的声明调到与切片不同步，装载就拒绝。
/// 不留到运行期——运行期的两支（按大的切 / 按小的切）都是某一族行静默不开账。
#[test]
fn a_view_cost_that_disagrees_with_the_bound_is_rejected_at_load_time() {
    let mut b = Builder::empty();
    b.fact(request_fact());
    b.row(cover(EVENT_REQUEST, None));
    b.view(probe_view_costing("wide", &[(EVENT_REQUEST, "id")], 4096));
    b.view(probe_view_costing("narrow", &[(EVENT_REQUEST, "id")], 8));

    let gaps = gaps_of(b);
    assert!(
        gaps.contains(&Gap::ViewCostMismatch {
            view: "wide",
            cost: 4096,
            bound: 8,
        }),
        "偏大的那一条要指名道姓（切点按最小值走，它会每轮超界）：{gaps:?}"
    );
    assert!(
        !gaps
            .iter()
            .any(|g| matches!(g, Gap::ViewCostMismatch { view: "narrow", .. })),
        "切点自己那一条不该被报：它不是分歧的一方，它就是界"
    );
}

// ── 执行体的写入面 ───────────────────────────────────────────────────────

/// 引擎独占的四格（`sent` / `response` / `failed` / `retry`）不许进执行体的 `writes`：
/// 放上 `sent` 允许它自己声明「我派出去了」，放上 `response` 允许它自己声明「我回来了」。
#[test]
fn an_actor_cannot_claim_an_engine_cell() {
    for cell in [EVENT_SENT, EVENT_RESPONSE, EVENT_FAILED, EVENT_RETRY] {
        // `ActorDecl.writes` 要 `'static`；用例进程内泄漏可忽略（同 `ring.test.rs` 的出口切片）。
        let writes: &'static [&'static str] = Box::leak(Box::new([cell]));
        let mut b = Builder::empty();
        b.fact(FactDecl {
            name: cell,
            port: Some("port"),
            key: &["id"],
            expand: &[],
            fields: &["port", "id"],
        });
        b.actor(ActorDecl {
            name: "carrier",
            mode: LatencyTier::Deep,
            reads: &[],
            writes,
        });
        b.row(cover(cell, Some("carrier")));
        assert_eq!(
            gaps_of(b),
            vec![Gap::ActorWritesEngineCell {
                actor: "carrier",
                fact: cell
            }],
            "{cell} 是引擎独占的格子"
        );
    }
}

/// `request` 不在引擎独占名单里：它是**决策者**写的——「这一轮欠一次调用」是判断，
/// 不是执行（§2.3）。把这条钉住，免得下次收口时顺手把它一起封掉。
#[test]
fn the_request_cell_belongs_to_the_decider() {
    let mut b = Builder::empty();
    b.fact(request_fact());
    b.actor(ActorDecl {
        name: "decider",
        mode: LatencyTier::Deep,
        reads: &[],
        writes: &[EVENT_REQUEST],
    });
    b.row(cover(EVENT_REQUEST, Some("decider")));
    assert_eq!(gaps_of(b), vec![]);
}

/// 每一档守卫角色都要有人居住：装配口把「判据空转」报成缺口（DE-024 D-203）。
///
/// 这条用例钉的是**方向**——owner 删掉一行 `roles`，表现不是拒绝而是「看不见」
/// （守卫从拦人变成全绿）。所以它在冻结之后由装配口逐档问一遍，不靠人记得。
#[test]
fn unmanned_roles_are_reported_per_variant() {
    // 只住一档：其余三档在表上无人。
    const PARTIAL: &[EventDecl] = &[EventDecl::with_roles(
        "fixture.final",
        (Entity::Turn, Verb::Closed),
        &[FactRole::FinalizesTurn],
    )];
    let mut b = Builder::empty();
    b.declare(
        "plugins/fixture",
        PluginDeclarations {
            events: PARTIAL,
            facts: &[],
            views: &[],
            ports: &[],
            actors: Vec::new(),
            rows: &[],
        },
    );
    let frozen = b.seal().expect("只交事件行也能冻结");
    assert_eq!(
        frozen.unmanned_roles(),
        vec![
            FactRole::StopsTurn,
            FactRole::DeclaresBudget,
            FactRole::CountsRework
        ],
        "缺口名单按 `FactRole::ALL` 的顺序，逐档有名有姓"
    );
    assert!(frozen.holds_role("fixture.final", FactRole::FinalizesTurn));
    assert!(
        !frozen.holds_role("fixture.final", FactRole::StopsTurn),
        "角色取自这一行，不是「同格就算」"
    );
    assert!(
        !frozen.holds_role("fixture.absent", FactRole::FinalizesTurn),
        "表上没有的名字不扮演任何东西"
    );

    // 四档住满（一格多名、一名多档都允许）⇒ 空名单。
    const FULL: &[EventDecl] = &[
        EventDecl::with_roles(
            "fixture.a",
            (Entity::Turn, Verb::Closed),
            &[FactRole::FinalizesTurn],
        ),
        EventDecl::with_roles(
            "fixture.b",
            (Entity::Turn, Verb::Asserted),
            &[FactRole::StopsTurn, FactRole::DeclaresBudget],
        ),
        EventDecl::new("fixture.c", (Entity::Task, Verb::Asserted)),
        EventDecl::with_roles(
            "fixture.d",
            (Entity::Task, Verb::Asserted),
            &[FactRole::CountsRework],
        ),
    ];
    let mut b = Builder::empty();
    b.declare(
        "plugins/fixture",
        PluginDeclarations {
            events: FULL,
            facts: &[],
            views: &[],
            ports: &[],
            actors: Vec::new(),
            rows: &[],
        },
    );
    let frozen = b.seal().expect("角色不影响形状判据");
    assert!(
        frozen.unmanned_roles().is_empty(),
        "{:?}",
        frozen.unmanned_roles()
    );
}
