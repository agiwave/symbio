//! 五类扩展夹具的断言：每一类都只经 `PluginDeclarations` / 机制类型接入。

use serde_json::{json, Value};

use symbio_platform::kernel::adapters::LatencyTier;
use symbio_platform::kernel::capability::{CapabilityFamily, CapabilityRow};
use symbio_platform::kernel::decl::ring::Decl;
use symbio_platform::kernel::decl::ring::Pattern;
use symbio_platform::kernel::decl::{ActorDecl, Builder, FactDecl, PluginDeclarations, ViewDecl};
use symbio_platform::kernel::event::{
    Entity, Event, EventDecl, Verb, EVENT_FAILED, EVENT_REQUEST, EVENT_RESPONSE, EVENT_RETRY,
    EVENT_SENT,
};
use symbio_platform::kernel::projection::Projection;

const FIXTURE_FACT: &str = "fixture.fact";
const FIXTURE_VIEW: &str = "fixture.view";

fn pattern(source: &'static str) -> Pattern {
    Pattern {
        source,
        key: &[],
        eq: &[],
    }
}

fn cover(source: &'static str) -> Decl {
    let exits: &'static [Pattern] = Box::leak(Box::new([pattern(source)]));
    Decl {
        name: Box::leak(format!("cover-{source}").into_boxed_str()),
        reads: &[],
        expand: None,
        owes: exits[0],
        until: exits,
        unless: &[],
    }
}

fn strategy_row(fact: &'static str) -> Decl {
    let exit: &'static [Pattern] = Box::leak(Box::new([pattern(EVENT_SENT)]));
    Decl {
        name: "fixture.new-row",
        reads: &[],
        expand: None,
        owes: pattern(fact),
        until: exit,
        unless: &[],
    }
}

fn rows(sources: &[&'static str]) -> &'static [Decl] {
    let mut all: Vec<&'static str> = vec![
        EVENT_REQUEST,
        EVENT_SENT,
        EVENT_RESPONSE,
        EVENT_FAILED,
        EVENT_RETRY,
    ];
    all.extend_from_slice(sources);
    let out: Vec<Decl> = all.iter().map(|s| cover(s)).collect();
    Box::leak(out.into_boxed_slice())
}

fn fact(name: &'static str) -> FactDecl {
    FactDecl {
        name,
        port: None,
        key: &[],
        expand: &[],
        fields: &["id", "why"],
    }
}

fn facts(names: &[&'static str]) -> &'static [FactDecl] {
    Box::leak(
        names
            .iter()
            .map(|n| fact(n))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    )
}

fn events(names: &[&'static str]) -> &'static [EventDecl] {
    Box::leak(
        names
            .iter()
            .map(|name| EventDecl::new(name, (Entity::Scope, Verb::Asserted)))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    )
}

fn eval_ids(events: &[Event], _now: i64) -> Value {
    json!(events
        .iter()
        .map(|e| e.event_id.clone())
        .collect::<Vec<_>>())
}

#[test]
fn fact_extension_is_owner_local() {
    let mut b = Builder::new();
    b.declare(
        "fixture",
        PluginDeclarations {
            events: events(&[FIXTURE_FACT]),
            facts: facts(&[FIXTURE_FACT]),
            views: &[],
            ports: &[],
            actors: Vec::new(),
            rows: rows(&[FIXTURE_FACT]),
        },
    );
    let frozen = b.seal(&[]).expect("新增事实只经声明口接入，内核不认识它");
    assert!(frozen.fact(FIXTURE_FACT).is_some());
    assert_eq!(frozen.facts().len(), 6, "协议五格 + 夹具一格");
}

#[test]
fn view_extension_is_owner_local() {
    let mut b = Builder::new();
    b.declare(
        "fixture",
        PluginDeclarations {
            events: events(&[FIXTURE_FACT]),
            facts: facts(&[FIXTURE_FACT]),
            views: &[ViewDecl {
                name: FIXTURE_VIEW,
                ver: 1,
                reads: &[(FIXTURE_FACT, "id")],
                evidence: &[],
                cost: 1,
                eval: eval_ids,
                agg: None,
            }],
            ports: &[],
            actors: Vec::new(),
            rows: rows(&[FIXTURE_FACT]),
        },
    );
    let frozen = b.seal(&[]).expect("新增视图只经声明口接入");
    let view = frozen.view(FIXTURE_VIEW).expect("视图在冻结表里");
    assert_eq!(view.ver, 1);
    assert_eq!(
        frozen.view_owner(FIXTURE_VIEW),
        Some("fixture"),
        "视图的 owner 同样是交表方，不靠声明里多一个字段"
    );
    assert_eq!((view.eval)(&[], 0), json!([]));
}

#[test]
fn strategy_extension_is_owner_local() {
    let mut b = Builder::new();
    b.declare(
        "fixture",
        PluginDeclarations {
            events: events(&[FIXTURE_FACT]),
            facts: facts(&[FIXTURE_FACT]),
            views: &[],
            ports: &[],
            actors: vec![ActorDecl {
                name: "fixture-actor",
                mode: LatencyTier::Reflex,
                writes: &[FIXTURE_FACT],
            }],
            rows: {
                let mut out = rows(&[FIXTURE_FACT]).to_vec();
                out.push(strategy_row(FIXTURE_FACT));
                Box::leak(out.into_boxed_slice())
            },
        },
    );
    let row_writes: &'static [(&'static str, &'static [&'static str])] =
        Box::leak(vec![("fixture.new-row", &[FIXTURE_FACT][..])].into_boxed_slice());
    let frozen = b
        .seal(row_writes)
        .expect("新增策略行 = 一行声明 + 一个行实现");
    assert!(frozen.actor("fixture-actor").is_some());
    assert!(
        frozen.rows().iter().any(|r| r.name == "fixture.new-row"),
        "行表里有这一行，执行体由注册表回答"
    );
}

#[test]
fn plugin_extension_is_owner_local() {
    const A_FACT: &str = "fixture.owner_a";
    const B_FACT: &str = "fixture.owner_b";

    let mut b = Builder::new();
    b.declare(
        "fixture/owner_a",
        PluginDeclarations {
            events: events(&[A_FACT]),
            facts: facts(&[A_FACT]),
            views: &[],
            ports: &[],
            actors: Vec::new(),
            rows: rows(&[A_FACT]),
        },
    );
    b.declare(
        "fixture/owner_b",
        PluginDeclarations {
            events: events(&[B_FACT]),
            facts: facts(&[B_FACT]),
            views: &[],
            ports: &[],
            actors: Vec::new(),
            rows: Box::leak(vec![cover(B_FACT)].into_boxed_slice()),
        },
    );
    let frozen = b
        .seal(&[])
        .expect("两个 owner 各自交表即可共装，内核不登记插件名");
    assert!(frozen.fact(A_FACT).is_some());
    assert_eq!(frozen.facts().len(), 7, "协议五格 + 两个 owner 各一格");
    assert_eq!(
        frozen.fact_owner(A_FACT),
        Some("fixture/owner_a"),
        "owner 清单逐条记下交表方，不把两个 owner 混成一个"
    );
    assert_eq!(frozen.fact_owner(B_FACT), Some("fixture/owner_b"));
}

#[test]
fn shared_type_extension_is_owner_local() {
    struct FixtureShared {
        label: &'static str,
    }

    const FIXTURE_FAMILY: CapabilityFamily = CapabilityFamily::single("fixture.family");

    let row = CapabilityRow::new(
        FIXTURE_FAMILY,
        "fixture.capability",
        FixtureShared {
            label: "owner-local",
        },
        true,
    );
    assert_eq!(row.family().name(), "fixture.family");
    assert!(row.family().is_single());
    assert!(row.active());
    assert_eq!(
        row.payload::<FixtureShared>().map(|s| s.label),
        Some("owner-local"),
        "载荷按具体类型取回是声明方与消费方的私有约定，表不参与校验"
    );

    let projection = Projection::new(|events: &[Event], _now| events.len());
    assert_eq!(projection.read_all(&[]).value, 0);
}
