//! `decl.rs` 的单元测试 —— 格位、名字空间、owner 过滤与视图折叠。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;
use serde_json::json;
use symbio_plugin_api::ring::{Decl, Pattern};
use symbio_plugin_api::{Entity, Event, LatencyTier, PluginDeclarations, Verb};

/// 协议五格的**自读行** —— 完整进程里由 `crates/symbio-plugin-loop` 交表，
/// 本 crate 的测试二进制不链接它 ⇒ 夹具补上（否则 `seal` 判这五条协议事实 `OrphanFact`）。
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
    "crates/symbio-plugin-toolsmith-test-harness",
    harness_declarations
);

fn defined(name: &str, description: &str, owner: &str) -> Event {
    Event::pending(
        "e-1",
        TOOLSMITH_DEFINED,
        Entity::Domain,
        Verb::Asserted,
        "session/a",
    )
    .with_payload(json!({
        "name": name,
        "description": description,
        "input_schema": { "type": "object" },
        "owner": owner,
    }))
}

// ==================== 声明面 ====================

/// `toolsmith.defined` 落在 `Domain × Asserted`，owner 是本插件。
#[test]
fn the_domain_event_resolves_through_the_process_assembly() {
    let frozen = symbio_plugin_api::sealed_declarations().expect("测试进程的装配表应能冻结");
    assert_eq!(
        frozen.event_cell(TOOLSMITH_DEFINED),
        Some((Entity::Domain, Verb::Asserted))
    );
    assert_eq!(frozen.event_owner(TOOLSMITH_DEFINED), Some(OWNER));
}

/// 视图名与事实名共用同一名字空间——同名会让 `seal` 直接红。
#[test]
fn the_view_name_does_not_collide_with_the_fact_name() {
    assert_ne!(TOOLSMITH_RECIPES, TOOLSMITH_DEFINED);
}

/// 视图读的字段必须都在 `FactDecl.fields` 里（`seal` 的第 3 条）。
#[test]
fn the_view_only_reads_declared_fields() {
    let declared: Vec<&str> = FACTS
        .iter()
        .flat_map(|f| f.fields.iter().copied())
        .collect();
    for (fact, field) in VIEWS[0].reads {
        assert_eq!(*fact, TOOLSMITH_DEFINED, "视图只读本插件自己的事实");
        assert!(declared.contains(field), "视图读了没声明的字段：{field}");
    }
}

// ==================== 视图折叠 ====================

/// 同名再登记 ⇒ 后写覆盖，`observations` 仍记两次。
#[test]
fn a_later_definition_overwrites_the_same_name() {
    let events = [
        defined("summarize_ticket", "旧说明", OWNER_PLUGIN),
        defined("summarize_ticket", "新说明", OWNER_PLUGIN),
    ];
    let v = recipes(&events, i64::MAX);
    assert_eq!(v["count"], 1, "一个工具：{}", v);
    assert_eq!(v["observations"], 2, "两次登记都是证据：{}", v);
    assert_eq!(v["tools"][0]["name"], "summarize_ticket");
    assert_eq!(
        v["tools"][0]["description"], "新说明",
        "后一次定义覆盖前一次"
    );
    assert_eq!(v["tools"][0]["owner"], OWNER_PLUGIN);
}

/// **受限判据**：owner 不是 `toolsmith` 的定义读不出来——限制不靠写方守规矩。
#[test]
fn definitions_owned_by_others_are_skipped() {
    let v = recipes(
        &[
            defined("vdfs_delete", "冒充别的插件", "vdfs"),
            defined("summarize_ticket", "本插件自己的", OWNER_PLUGIN),
        ],
        i64::MAX,
    );
    assert_eq!(v["count"], 1, "只留自己的定义：{}", v);
    assert_eq!(v["observations"], 1);
    assert_eq!(v["tools"][0]["name"], "summarize_ticket");
}

/// 缺字段 / schema 不是对象的载荷静默跳过。
#[test]
fn malformed_payloads_are_skipped() {
    let no_schema = Event::pending(
        "e-2",
        TOOLSMITH_DEFINED,
        Entity::Domain,
        Verb::Asserted,
        "s",
    )
    .with_payload(json!({ "name": "x", "description": "d", "owner": OWNER_PLUGIN }));
    let mut bad_schema = defined("x", "d", OWNER_PLUGIN);
    bad_schema.payload["input_schema"] = json!("not an object");
    let v = recipes(
        &[no_schema, bad_schema, defined("good", "d", OWNER_PLUGIN)],
        i64::MAX,
    );
    assert_eq!(v["count"], 1);
    assert_eq!(v["observations"], 1, "坏载荷不计入登记次数：{}", v);
}

/// 未来登记不参与折叠（`ts > now`）。
#[test]
fn future_events_are_ignored() {
    let mut e = defined("summarize_ticket", "d", OWNER_PLUGIN);
    e.ts = 500;
    assert_eq!(recipes(&[e], 100)["count"], 0);
}

/// 超出上限只截输出长度，保留最早登记的名字。
#[test]
fn the_output_is_capped_but_keeps_the_earliest() {
    let events: Vec<Event> = (0..60)
        .map(|i| defined(&format!("tool_{i}"), "d", OWNER_PLUGIN))
        .collect();
    let v = recipes(&events, i64::MAX);
    assert_eq!(v["count"], LIMIT);
    assert_eq!(v["tools"][0]["name"], "tool_0", "留下的是最早 50 个定义");
}

/// schema 原样透出：读方要靠它知道将来怎么调用这个工具。
#[test]
fn the_schema_is_carried_through() {
    let mut e = defined("summarize_ticket", "d", OWNER_PLUGIN);
    e.payload["input_schema"] =
        json!({ "type": "object", "properties": { "ticket": { "type": "string" } } });
    let v = recipes(&[e], i64::MAX);
    assert_eq!(
        v["tools"][0]["input_schema"]["properties"]["ticket"]["type"],
        "string"
    );
}
