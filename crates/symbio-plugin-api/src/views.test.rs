//! 跨插件视图读口的判据：只读公开名、按声明求值、读侧不设扫描上界。
//!
//! 小世界用 `Builder` 现装一张冻结表——本 crate 的测试二进制不链接任何 owner 插件，
//! 因此 `named_in` 的 `require_shared` 在「名字公开性」用例里显式传 `true`，其余用例
//! 直接按声明名读（`false`），两层判据各自可钉。

use super::*;

use std::sync::Arc;

use symbio_kernel::adapters::LatencyTier;
use symbio_kernel::decl::ring::Decl;
use symbio_kernel::decl::ring::Pattern;
use symbio_kernel::decl::{Builder, FactDecl, PluginDeclarations, ViewDecl};
use symbio_kernel::event::{Entity, EventDecl, Verb};

const ITEM_ADDED: &str = "test.item.added";
const RECENT: &str = "test.recent";
/// 带「多片合并」那一半的视图（`D-281`）。
const TOTAL: &str = "test.total";

fn recent(events: &[Event], now: i64) -> Value {
    let texts: Vec<&str> = events
        .iter()
        .filter(|e| e.kind == ITEM_ADDED && e.ts <= now)
        .filter_map(|e| e.payload.get("text").and_then(Value::as_str))
        .collect();
    serde_json::json!({ "count": texts.len(), "items": texts })
}

/// 合并那一半：N 份 `recent` 的产物 → 一个合计。
///
/// 形状由 owner 定（`sessions` / `total`），读口不认识它——这正是 `agg` 存在的理由：
/// 读方拿得到「N 个会话合起来是多少」，而不必自己认识 `count` 这个字段。
fn total(parts: &[Value]) -> Value {
    let sum: u64 = parts
        .iter()
        .filter_map(|p| p.get("count").and_then(Value::as_u64))
        .sum();
    serde_json::json!({ "sessions": parts.len(), "total": sum })
}

const EVENTS: &[EventDecl] = &[EventDecl::new(ITEM_ADDED, (Entity::Domain, Verb::Opened))];
const FACTS: &[FactDecl] = &[FactDecl {
    name: ITEM_ADDED,
    port: None,
    key: &[],
    expand: &[],
    fields: &["text"],
}];
const VIEWS: &[ViewDecl] = &[
    ViewDecl {
        name: RECENT,
        ver: 7,
        reads: &[(ITEM_ADDED, "text")],
        evidence: &[],
        cost: 2,
        eval: recent,
        agg: None,
    },
    ViewDecl {
        name: TOTAL,
        ver: 1,
        reads: &[(ITEM_ADDED, "text")],
        evidence: &[],
        cost: 2,
        eval: recent,
        agg: Some(total),
    },
];

fn protocol_rows() -> &'static [Decl] {
    let mut out = Vec::new();
    // 夹具事实也要有归宿：行表只认 `owes` 指向的事实有人接手。
    let own: &'static [Pattern] = Box::leak(Box::new([Pattern {
        source: ITEM_ADDED,
        key: &[],
        eq: &[],
    }]));
    out.push(Decl {
        name: ITEM_ADDED,
        phase: 1,
        anchor: 50,
        by: None,
        tier: LatencyTier::Reflex,
        expand: None,
        owes: own[0],
        until: own,
        unless: &[],
    });
    for name in [
        symbio_kernel::event::EVENT_REQUEST,
        symbio_kernel::event::EVENT_SENT,
        symbio_kernel::event::EVENT_RESPONSE,
        symbio_kernel::event::EVENT_FAILED,
        symbio_kernel::event::EVENT_RETRY,
    ] {
        let pattern: &'static [Pattern] = Box::leak(Box::new([Pattern {
            source: name,
            key: &[],
            eq: &[],
        }]));
        out.push(Decl {
            name,
            phase: 8,
            anchor: 99,
            by: None,
            tier: LatencyTier::Reflex,
            expand: None,
            owes: pattern[0],
            until: pattern,
            unless: &[],
        });
    }
    Box::leak(out.into_boxed_slice())
}

fn frozen() -> Arc<Frozen> {
    let mut builder = Builder::new();
    builder.declare(
        "tests/views",
        PluginDeclarations {
            events: EVENTS,
            facts: FACTS,
            views: VIEWS,
            ports: &[],
            actors: Vec::new(),
            rows: protocol_rows(),
        },
    );
    builder.seal(&[]).expect("小世界应能冻结")
}

fn event(id: &str, text: &str, ts: i64) -> Event {
    Event::pending(
        id,
        ITEM_ADDED,
        Entity::Domain,
        Verb::Opened,
        0,
        "tests/main",
    )
    .with_ts(ts)
    .with_payload(serde_json::json!({ "text": text }))
}

/// 合并那一半由 owner 交、经同一张冻结表被调用——读口不认识值的形状（`D-281`）。
#[test]
fn aggregate_uses_the_owners_half_through_the_same_table() {
    let frozen = frozen();
    let handle = ViewRead::named_in(Some(&frozen), TOTAL, false).expect("已声明");
    assert!(
        handle.aggregable_in(Some(&frozen)).expect("小世界已冻结"),
        "`TOTAL` 交了 `agg` 那一半"
    );

    // 三份「一个会话」的读数 → 一份汇总。形状是 owner 的（`sessions` / `total`），
    // 读方一个字都不认识——它只把 `read_in` 的产物原样交回去。
    let parts: Vec<Value> = [
        vec![event("a", "甲", 1), event("b", "乙", 2)],
        vec![event("c", "丙", 3)],
        Vec::new(),
    ]
    .iter()
    .map(|ev| handle.read_in(Some(&frozen), ev, i64::MAX).unwrap())
    .collect();

    assert_eq!(
        handle.aggregate_in(Some(&frozen), &parts).unwrap(),
        serde_json::json!({ "sessions": 3, "total": 3 })
    );
}

/// 没交合并那一半 = **这一格不在汇总里**，不是恒等、不是空汇总（`D-281`）。
#[test]
fn not_aggregable_is_a_named_absence_not_an_identity() {
    let frozen = frozen();
    let handle = ViewRead::named_in(Some(&frozen), RECENT, false).unwrap();
    assert!(!handle.aggregable_in(Some(&frozen)).expect("小世界已冻结"));
    assert_eq!(
        handle.aggregate_in(Some(&frozen), &[]),
        Err(ViewUnavailable::NotAggregable { name: RECENT }),
        "缺席要报名，不许静默返回空对象"
    );
}

/// 名字、版本与算法都来自声明表——读口没有自己的副本。
#[test]
fn reads_a_declared_view_through_the_frozen_table() {
    let frozen = frozen();
    let handle = ViewRead::named_in(Some(&frozen), RECENT, false).expect("已声明");
    assert_eq!(handle.name(), RECENT);
    assert_eq!(handle.ver_in(Some(&frozen)).expect("小世界已冻结"), 7);

    let events = [event("a", "甲", 10), event("b", "乙", 20)];
    let got = handle.read_in(Some(&frozen), &events, i64::MAX).unwrap();
    assert_eq!(
        got,
        serde_json::json!({ "count": 2, "items": ["甲", "乙"] })
    );
}

/// as-of：`now` 之前的事件才算——读口把时间戳原样交给 owner 的求值函数。
#[test]
fn as_of_is_passed_through_unmodified() {
    let frozen = frozen();
    let handle = ViewRead::named_in(Some(&frozen), RECENT, false).unwrap();
    let events = [event("a", "甲", 10), event("b", "乙", 20)];
    assert_eq!(
        handle.read_in(Some(&frozen), &events, 15).unwrap(),
        serde_json::json!({ "count": 1, "items": ["甲"] })
    );
}

/// 读侧没有扫描上界（`D-284`）：给多长的切片就折多长，既不截断也不报缺席。
///
/// 同一个 `cost` 声明在环内那一侧照样生效——`Frozen::eval` 对同一片超界切片回 `None`。
/// 一条用例钉住两半：搬家的读数不变，而环的预算没有被放宽。
#[test]
fn the_read_side_has_no_scan_bound_while_the_ring_keeps_it() {
    let frozen = frozen();
    let handle = ViewRead::named_in(Some(&frozen), RECENT, false).unwrap();
    let events = [
        event("a", "甲", 1),
        event("b", "乙", 2),
        event("c", "丙", 3),
    ];
    assert_eq!(frozen.scan_bound(), Some(2), "夹具声明的上界就是 2");
    assert_eq!(
        handle.read_in(Some(&frozen), &events, i64::MAX).unwrap(),
        serde_json::json!({ "count": 3, "items": ["甲", "乙", "丙"] }),
        "超出声明 cost 的切片在读口这里照常折完"
    );
    assert_eq!(
        frozen.eval(RECENT, &events, i64::MAX),
        None,
        "同一份切片在环内仍然算不出来——`cost` 的判定面没有变宽"
    );
}

/// 三种缺席各自可辨：没公开 / 公开了没声明 / 装配表没冻结。
#[test]
fn absence_states_are_distinguishable() {
    let frozen = frozen();
    assert_eq!(
        ViewRead::named_in(Some(&frozen), RECENT, true),
        Err(ViewUnavailable::NotShared {
            name: RECENT.to_string()
        }),
        "RECENT 不在清单公开表里"
    );
    assert_eq!(
        ViewRead::named_in(Some(&frozen), "test.absent", false),
        Err(ViewUnavailable::Unknown {
            name: "test.absent".to_string()
        })
    );
    assert_eq!(
        ViewRead::named_in(None, RECENT, false),
        Err(ViewUnavailable::NotSealed)
    );
}

/// 生成的公开表确实进了本 crate：notes 的夹具视图是本仓当前唯一一条共享视图。
#[test]
fn the_generated_shared_table_is_linked() {
    assert!(
        shared().iter().any(|v| v.name == "notes.recent"),
        "notes.recent 应出现在清单生成的共享视图表里：{:?}",
        shared()
    );
}

/// 缺席态**自己报名**：`reason()` 是给机器分支的稳定词，`Display` 是给人看的话
/// （带名字与数字）。读数出口因此不必解析文案，也不必为每个 owner 写一份对照表。
#[test]
fn each_absence_state_names_itself() {
    assert_eq!(ViewUnavailable::NotSealed.reason(), "not_sealed");
    assert!(
        ViewUnavailable::NotSealed.to_string().contains("未冻结"),
        "没冻结要说清是装配表的事：{}",
        ViewUnavailable::NotSealed
    );

    let not_shared = ViewUnavailable::NotShared {
        name: "a.b".to_string(),
    };
    assert_eq!(not_shared.reason(), "not_shared");
    assert!(
        not_shared.to_string().contains("shared_views"),
        "未公开要指出登记处：{not_shared}"
    );

    let unknown = ViewUnavailable::Unknown {
        name: "a.b".to_string(),
    };
    assert_eq!(unknown.reason(), "unknown");
    assert!(unknown.to_string().contains("a.b"), "要带名字：{unknown}");

    let no_agg = ViewUnavailable::NotAggregable { name: "a.b" };
    assert_eq!(no_agg.reason(), "not_aggregable");
    assert!(
        no_agg.to_string().contains("a.b") && no_agg.to_string().contains("多片合并"),
        "要说清是哪一格、缺的是哪一半：{no_agg}"
    );
}
