//! 五类扩展夹具的断言：每一类都只经 `PluginDeclarations` / 机制类型接入。
//!
//! 判据不是「代码好看」，而是「**内核零 diff**」：这些名字（`fixture.*`）只出现在
//! 本文件与内核的声明类型里——内核不认识它们，`seal` 只做形状检查。

use serde_json::{json, Value};

use symbio_core::ring::{Decl, Pattern};
use symbio_core::{
    ActorDecl, Builder, CapabilityFamily, CapabilityRow, Event, FactDecl, LatencyTier,
    PluginDeclarations, Projection, ViewDecl, EVENT_FAILED, EVENT_REQUEST, EVENT_RESPONSE,
    EVENT_RETRY, EVENT_SENT,
};

const FIXTURE_FACT: &str = "fixture.fact";
const FIXTURE_VIEW: &str = "fixture.view";

fn pattern(source: &'static str) -> Pattern {
    Pattern {
        source,
        key: &[],
        eq: &[],
    }
}

/// 一行「归宿」：某个事实被子句读到即算有归宿（判据 7）。
fn cover(source: &'static str, by: Option<&'static str>) -> Decl {
    let exits: &'static [Pattern] = Box::leak(Box::new([pattern(source)]));
    Decl {
        name: Box::leak(format!("cover-{source}-{}", by.unwrap_or("none")).into_boxed_str()),
        prio: 99,
        by,
        tier: LatencyTier::Reflex,
        expand: None,
        owes: exits[0],
        until: exits,
        unless: &[],
    }
}

/// 归宿行：协议五格 + 夹具自己的事实（装载期判据 7：每个事实名都要有归宿）。
fn rows(sources: &[&'static str]) -> &'static [Decl] {
    let mut all: Vec<&'static str> = vec![
        EVENT_REQUEST,
        EVENT_SENT,
        EVENT_RESPONSE,
        EVENT_FAILED,
        EVENT_RETRY,
    ];
    all.extend_from_slice(sources);
    let out: Vec<Decl> = all.iter().map(|s| cover(s, None)).collect();
    let leaked: &'static mut [Decl] = Box::leak(out.into_boxed_slice());
    leaked
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

/// 声明切片要 `'static`（装载期持有），测试里用泄漏给它一个稳定地址。
fn facts(names: &[&'static str]) -> &'static [FactDecl] {
    Box::leak(
        names
            .iter()
            .map(|n| fact(n))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    )
}

/// 视图求值函数：只拿事实切片与时间戳（F2 的纯净形状）。
fn eval_ids(events: &[Event], _now: i64) -> Value {
    json!(events
        .iter()
        .map(|e| e.event_id.clone())
        .collect::<Vec<_>>())
}

// ── 1. 事实 ──────────────────────────────────────────────────────────

#[test]
fn fact_extension_is_owner_local() {
    let mut b = Builder::new();
    b.declare(
        "fixture",
        PluginDeclarations {
            facts: facts(&[FIXTURE_FACT]),
            views: &[],
            ports: &[],
            actors: Vec::new(),
            rows: rows(&[FIXTURE_FACT]),
        },
    );
    let frozen = b.seal().expect("新增事实只经声明口接入，内核不认识它");
    assert!(frozen.fact(FIXTURE_FACT).is_some());
    assert_eq!(frozen.facts().len(), 6, "协议五格 + 夹具一格");
}

// ── 2. 视图 ──────────────────────────────────────────────────────────

#[test]
fn view_extension_is_owner_local() {
    let mut b = Builder::new();
    b.declare(
        "fixture",
        PluginDeclarations {
            facts: facts(&[FIXTURE_FACT]),
            views: &[ViewDecl {
                name: FIXTURE_VIEW,
                ver: 1,
                reads: &[(FIXTURE_FACT, "id")],
                evidence: &[],
                cost: 1,
                eval: eval_ids,
            }],
            ports: &[],
            actors: Vec::new(),
            rows: rows(&[FIXTURE_FACT]),
        },
    );
    let frozen = b.seal().expect("新增视图只经声明口接入");
    let view = frozen.view(FIXTURE_VIEW).expect("视图在冻结表里");
    assert_eq!(view.ver, 1);
    assert_eq!(
        frozen.view_owner(FIXTURE_VIEW),
        Some("fixture"),
        "视图的 owner 同样是交表方，不靠声明里多一个字段"
    );
    assert_eq!((view.eval)(&[], 0), json!([]));
}

// ── 3. 策略（一行 + 一个执行体）──────────────────────────────────────

#[test]
fn strategy_extension_is_owner_local() {
    let mut b = Builder::new();
    b.declare(
        "fixture",
        PluginDeclarations {
            facts: facts(&[FIXTURE_FACT]),
            views: &[],
            ports: &[],
            actors: vec![ActorDecl {
                name: "fixture-actor",
                mode: LatencyTier::Reflex,
                reads: &[],
                writes: &[FIXTURE_FACT],
            }],
            rows: {
                let mut out = rows(&[FIXTURE_FACT]).to_vec();
                out.push(cover(FIXTURE_FACT, Some("fixture-actor")));
                Box::leak(out.into_boxed_slice())
            },
        },
    );
    let frozen = b.seal().expect("新增策略行 = 一行声明 + 一个执行体");
    assert!(frozen.actor("fixture-actor").is_some());
    assert!(
        frozen.rows().iter().any(|r| r.by == Some("fixture-actor")),
        "行的 by 指向执行体，派发只按数据解析"
    );
}

// ── 4. 插件（一次 declare = 一个 owner 的全部声明）───────────────────

#[test]
fn plugin_extension_is_owner_local() {
    const A_FACT: &str = "fixture.owner_a";
    const B_FACT: &str = "fixture.owner_b";

    let mut b = Builder::new();
    b.declare(
        "fixture/owner_a",
        PluginDeclarations {
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
            facts: facts(&[B_FACT]),
            views: &[],
            ports: &[],
            actors: Vec::new(),
            rows: Box::leak(vec![cover(B_FACT, None)].into_boxed_slice()),
        },
    );
    let frozen = b
        .seal()
        .expect("两个 owner 各自交表即可共装，内核不登记插件名");
    assert!(frozen.fact(A_FACT).is_some());
    assert!(frozen.fact(B_FACT).is_some());
    assert_eq!(frozen.facts().len(), 7, "协议五格 + 两个 owner 各一格");
    assert_eq!(
        frozen.fact_owner(A_FACT),
        Some("fixture/owner_a"),
        "owner 清单逐条记下交表方，而不是把两个 owner 混成一个"
    );
    assert_eq!(frozen.fact_owner(B_FACT), Some("fixture/owner_b"));
}

// ── 5. 共享类型（插件本地族 + 开放载荷 + 机制构造器）────────────────

#[test]
fn shared_type_extension_is_owner_local() {
    /// 夹具自己的「共享类型」：定义在插件侧，内核不认识。
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

    // 投影同理：视图类型与实现在插件侧，内核只出机制构造器。
    let projection = Projection::new(|events: &[Event], _now| events.len());
    assert_eq!(projection.read_all(&[]).value, 0);
}
