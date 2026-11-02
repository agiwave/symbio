//! `decl.rs` 的单元测试 —— 格位、名字空间与视图折叠。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 本 crate 的测试二进制只链接它自己的声明表（`idempotency.recorded` 的归宿行也在自己
//! 这里），因此这些断言读的就是「新增能力包自己交的表」，没有别的插件代答。

use super::*;
use serde_json::json;
use symbio_plugin_api::ring::{Decl, Pattern};
use symbio_plugin_api::{Entity, Event, LatencyTier, PluginDeclarations, Verb};

/// 协议五格的**自读行** —— 完整进程里由 `crates/symbio-plugin-loop` 交表，
/// 本 crate 的测试二进制不链接它 ⇒ 夹具补上。
///
/// 不补的后果不是「少几个测试」：`seal` 会把这五条协议事实判成 `OrphanFact`
/// （每条事实都要有归宿行），整张装配表失败。夹具与 `notes` / `explain` 的同名
/// 实现同形，只在 `cfg(test)` 生效。
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

fn harness_declarations() -> PluginDeclarations<'static> {
    PluginDeclarations {
        events: &[],
        facts: &[],
        views: &[],
        ports: &[],
        actors: Vec::new(),
        rows: protocol_rows(),
    }
}

symbio_plugin_api::submit_declarations!(
    "crates/symbio-plugin-idempotency-test-harness",
    harness_declarations
);

fn recorded(key: &str, value: &str) -> Event {
    Event::pending(
        "i-1",
        IDEMPOTENCY_RECORDED,
        Entity::Domain,
        Verb::Asserted,
        1,
        "session/a",
    )
    .with_payload(json!({ "key": key, "value": value }))
}

// ==================== 声明面 ====================

/// `idempotency.recorded` 落在 `Domain × Asserted`，owner 是本插件。
#[test]
fn the_domain_event_resolves_through_the_process_assembly() {
    let frozen = symbio_plugin_api::sealed_declarations().expect("测试进程的装配表应能冻结");
    assert_eq!(
        frozen.event_cell(IDEMPOTENCY_RECORDED),
        Some((Entity::Domain, Verb::Asserted))
    );
    assert_eq!(frozen.event_owner(IDEMPOTENCY_RECORDED), Some(OWNER));
}

/// 视图名与事实名共用同一名字空间（视图是合成事实）——两者同名会让 `seal` 直接红。
#[test]
fn the_view_name_does_not_collide_with_the_fact_name() {
    assert_ne!(IDEMPOTENCY_LEDGER, IDEMPOTENCY_RECORDED);
}

/// 视图读的字段必须都在 `FactDecl.fields` 里（`seal` 的第 3 条）。
#[test]
fn the_view_only_reads_declared_fields() {
    let declared: Vec<&str> = FACTS
        .iter()
        .flat_map(|f| f.fields.iter().copied())
        .collect();
    for (fact, field) in VIEWS[0].reads {
        assert_eq!(*fact, IDEMPOTENCY_RECORDED, "视图只读本插件自己的事实");
        assert!(declared.contains(field), "视图读了没声明的字段：{field}");
    }
}

// ==================== 视图折叠 ====================

/// 两个不同的键 ⇒ 两格台账，顺序按写入序。
#[test]
fn two_keys_yield_two_entries_in_write_order() {
    let events = [recorded("build-1", "ok"), recorded("build-2", "failed")];
    let v = idempotency_ledger(&events, i64::MAX);
    assert_eq!(v["count"], 2);
    assert_eq!(v["entries"][0]["key"], "build-1");
    assert_eq!(v["entries"][0]["value"], "ok");
    assert_eq!(v["entries"][1]["key"], "build-2");
    assert_eq!(v["entries"][1]["value"], "failed");
    assert_eq!(v["keys"], json!(["build-1", "build-2"]));
}

/// 同一个键记两次 ⇒ 台账仍只有一格，且**首写为准**。
///
/// 它是**折出来的**判据：一个把每条事实原样吐出的实现会让 `count` 为 2。
#[test]
fn a_repeated_key_is_folded_to_the_first_record() {
    let events = [recorded("build-1", "ok"), recorded("build-1", "other")];
    let v = idempotency_ledger(&events, i64::MAX);
    assert_eq!(v["count"], 1);
    assert_eq!(v["entries"][0]["value"], "ok", "首写为准");
    assert_eq!(v["keys"], json!(["build-1"]));
}

/// 未来事件不参与折叠（`ts > now`）。
#[test]
fn future_events_are_ignored() {
    let mut e = recorded("build-1", "ok");
    e.ts = 500;
    assert_eq!(idempotency_ledger(&[e], 100)["count"], 0);
}

/// 缺字段的载荷静默跳过：视图不能因为一条坏事实就整个算不出来。
#[test]
fn malformed_payloads_are_skipped() {
    let broken = Event::pending(
        "i-2",
        IDEMPOTENCY_RECORDED,
        Entity::Domain,
        Verb::Asserted,
        1,
        "s",
    )
    .with_payload(json!({ "key": "build-1" }));
    let v = idempotency_ledger(&[broken, recorded("build-2", "ok")], i64::MAX);
    assert_eq!(v["count"], 1);
    assert_eq!(v["keys"], json!(["build-2"]));
}

/// 超出上限只截**输出长度**，保留最新的记录。
#[test]
fn the_output_is_capped_but_keeps_the_latest() {
    let events: Vec<Event> = (0..60).map(|i| recorded(&format!("k{i}"), "v")).collect();
    let v = idempotency_ledger(&events, i64::MAX);
    assert_eq!(v["count"], LIMIT);
    assert_eq!(v["entries"][0]["key"], "k10", "留下的是最后 50 条");
    assert_eq!(v["entries"][LIMIT - 1]["key"], "k59");
}
