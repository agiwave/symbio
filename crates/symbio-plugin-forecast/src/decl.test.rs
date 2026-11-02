//! `decl.rs` 的单元测试 —— 视图只读公开的那两条事件、折叠口径与装载期解析。
//!
//! 本 crate 的测试二进制只链接它自己的声明表，任务插件不在场 ⇒ 夹具补上
//! `task.opened` / `task.asserted` 的两条 `EventDecl`（与 `task` 生产装配里交的同形）。
//! 不补的后果不是「少几个测试」：`seal` 会把视图 `reads` 判成 `UnknownFact`，整张表
//! 失败（`D-282` 的第二种读法要求名字在事件表上）。夹具只在 `cfg(test)` 生效。

use super::*;
use serde_json::json;
use symbio_plugin_api::ring::{Decl, Pattern};
use symbio_plugin_api::{Entity, Event, EventDecl, LatencyTier, PluginDeclarations, Verb};

/// 任务插件公开的两条事件（`PLUGIN.yml` 的 `shared_events`）。
const TASK_EVENTS: &[EventDecl] = &[
    EventDecl::new(EVENT_TASK_OPENED, (Entity::Task, Verb::Opened)),
    EventDecl::new(EVENT_TASK_ASSERTED, (Entity::Task, Verb::Asserted)),
];

/// 协议五格的**自读行** —— 完整进程里由 `crates/symbio-plugin-loop` 交表，
/// 本 crate 的测试二进制不链接它 ⇒ 夹具补上，否则 `seal` 判 `OrphanFact`。
fn protocol_rows() -> &'static [Decl] {
    let mut out = Vec::new();
    for name in [
        symbio_plugin_api::EVENT_REQUEST,
        symbio_plugin_api::EVENT_SENT,
        symbio_plugin_api::EVENT_RESPONSE,
        symbio_plugin_api::EVENT_FAILED,
        symbio_plugin_api::EVENT_RETRY,
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

fn harness_declarations() -> PluginDeclarations<'static> {
    PluginDeclarations {
        events: TASK_EVENTS,
        facts: &[],
        views: &[],
        ports: &[],
        actors: Vec::new(),
        rows: protocol_rows(),
    }
}

symbio_plugin_api::submit_declarations!(
    "crates/symbio-plugin-forecast-test-harness",
    harness_declarations
);

fn ev(kind: &str, task: &str) -> Event {
    let verb = if kind == EVENT_TASK_OPENED {
        Verb::Opened
    } else {
        Verb::Asserted
    };
    Event::pending("e", kind, Entity::Task, verb, 1, "session/a")
        .with_payload(json!({ "task_id": task }))
}

// ==================== 声明面 ====================

/// 装配后按名字解析得到本 owner 的视图，且版本随 `ViewDecl` 走。
#[test]
fn the_view_resolves_through_the_process_assembly() {
    let frozen = symbio_plugin_api::sealed_declarations().expect("测试进程的装配表应能冻结");
    assert_eq!(frozen.view_owner(FORECAST_REMAINING), Some(OWNER));
    assert_eq!(frozen.view(FORECAST_REMAINING).map(|v| v.ver), Some(1));
}

/// 视图只读**公开**的任务事件，不碰任何私有事实；两条都带 `task_id` 字段。
#[test]
fn the_view_reads_only_shared_task_events() {
    for (fact, field) in VIEWS[0].reads {
        assert!(
            *fact == EVENT_TASK_OPENED || *fact == EVENT_TASK_ASSERTED,
            "视图读了不在 shared_events 里的名字：{fact}"
        );
        assert_eq!(*field, "task_id");
    }
}

/// `D-282` 的第二种读法：事件没有 `FactDecl` ⇒ 装载期把它们登记进报告名单，
/// 而不是静默跳过、也不是判成缺口。
#[test]
fn reads_on_event_only_cells_are_reported_once() {
    let frozen = symbio_plugin_api::sealed_declarations().expect("测试进程的装配表应能冻结");
    let reported: Vec<&str> = frozen
        .assertion_reads()
        .iter()
        .filter(|r| r.view == FORECAST_REMAINING)
        .map(|r| r.fact)
        .collect();
    assert_eq!(reported, vec![EVENT_TASK_OPENED, EVENT_TASK_ASSERTED]);
}

// ==================== 折叠口径 ====================

/// 开两条、断言一条 ⇒ 剩一条；计数是事件数，剩余是集合大小。
#[test]
fn remaining_is_opened_minus_asserted() {
    let events = [
        ev(EVENT_TASK_OPENED, "t1"),
        ev(EVENT_TASK_OPENED, "t2"),
        ev(EVENT_TASK_ASSERTED, "t1"),
    ];
    let v = remaining(&events, i64::MAX);
    assert_eq!(v["opened"], 2);
    assert_eq!(v["closed"], 1);
    assert_eq!(v["remaining"], 1);
    assert_eq!(v["outstanding"], json!(["t2"]));
}

/// 同一任务重复开只算一次剩余（集合语义）：重复落格由任务表的幂等键挡住，
/// 但视图不假设上游一定守规矩。
#[test]
fn a_repeated_open_is_not_double_counted() {
    let events = [
        ev(EVENT_TASK_OPENED, "t1"),
        ev(EVENT_TASK_OPENED, "t1"),
        ev(EVENT_TASK_ASSERTED, "t1"),
    ];
    let v = remaining(&events, i64::MAX);
    assert_eq!(v["opened"], 2, "事件计数原样透出");
    assert_eq!(v["remaining"], 0, "集合里只剩一条，断言后清空");
}

/// 未来事件不参与折叠（`ts > now`）：视图的 `now` 是这一轮的时钟。
#[test]
fn future_events_are_ignored() {
    let mut e = ev(EVENT_TASK_OPENED, "t1");
    e.ts = 500;
    let v = remaining(&[e], 100);
    assert_eq!(v["opened"], 0);
    assert_eq!(v["remaining"], 0);
}

/// 缺 `task_id` 的载荷静默跳过：视图不能因为一条坏事实就整个算不出来。
#[test]
fn malformed_payloads_are_skipped() {
    let broken = Event::pending("e-2", EVENT_TASK_OPENED, Entity::Task, Verb::Opened, 1, "s")
        .with_payload(json!({ "goal": "没有 id" }));
    let v = remaining(&[broken, ev(EVENT_TASK_OPENED, "t1")], i64::MAX);
    assert_eq!(v["opened"], 1);
    assert_eq!(v["outstanding"], json!(["t1"]));
}
