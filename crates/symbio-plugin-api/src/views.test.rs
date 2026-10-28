//! 跨插件视图读口的判据：只读公开名、按声明求值、超界不静默。
//!
//! 小世界用 `Builder` 现装一张冻结表——本 crate 的测试二进制不链接任何 owner 插件，
//! 因此 `named_in` 的 `require_shared` 在「名字公开性」用例里显式传 `true`，其余用例
//! 直接按声明名读（`false`），两层判据各自可钉。

use super::*;

use std::sync::Arc;

use symbio_core::ring::{Decl, Pattern};
use symbio_core::{
    Builder, Entity, EventDecl, FactDecl, LatencyTier, PluginDeclarations, Verb, ViewDecl,
};

const ITEM_ADDED: &str = "test.item.added";
const RECENT: &str = "test.recent";

fn recent(events: &[Event], now: i64) -> Value {
    let texts: Vec<&str> = events
        .iter()
        .filter(|e| e.kind == ITEM_ADDED && e.ts <= now)
        .filter_map(|e| e.payload.get("text").and_then(Value::as_str))
        .collect();
    serde_json::json!({ "count": texts.len(), "items": texts })
}

const EVENTS: &[EventDecl] = &[EventDecl::new(ITEM_ADDED, (Entity::Domain, Verb::Opened))];
const FACTS: &[FactDecl] = &[FactDecl {
    name: ITEM_ADDED,
    port: None,
    key: &[],
    expand: &[],
    fields: &["text"],
}];
const VIEWS: &[ViewDecl] = &[ViewDecl {
    name: RECENT,
    ver: 7,
    reads: &[(ITEM_ADDED, "text")],
    evidence: &[],
    cost: 2,
    eval: recent,
}];

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
        prio: 50,
        by: None,
        tier: LatencyTier::Reflex,
        expand: None,
        owes: own[0],
        until: own,
        unless: &[],
    });
    for name in [
        symbio_core::EVENT_REQUEST,
        symbio_core::EVENT_SENT,
        symbio_core::EVENT_RESPONSE,
        symbio_core::EVENT_FAILED,
        symbio_core::EVENT_RETRY,
    ] {
        let pattern: &'static [Pattern] = Box::leak(Box::new([Pattern {
            source: name,
            key: &[],
            eq: &[],
        }]));
        out.push(Decl {
            name,
            prio: 99,
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
    builder.seal().expect("小世界应能冻结")
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

/// 超界是**可诊断的缺席**，不是静默截断出一个小一点的数。
#[test]
fn over_budget_is_reported_not_truncated() {
    let frozen = frozen();
    let handle = ViewRead::named_in(Some(&frozen), RECENT, false).unwrap();
    let events = [
        event("a", "甲", 1),
        event("b", "乙", 2),
        event("c", "丙", 3),
    ];
    assert_eq!(
        handle.read_in(Some(&frozen), &events, i64::MAX),
        Err(ViewUnavailable::OverBudget {
            name: RECENT,
            events: 3,
            cost: 2,
        })
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

    let over = ViewUnavailable::OverBudget {
        name: "a.b",
        events: 3,
        cost: 2,
    };
    assert_eq!(over.reason(), "over_budget");
    assert!(
        over.to_string().contains("3 条事实 / 上界 2"),
        "超界要报出实测条数与上界：{over}"
    );
}
