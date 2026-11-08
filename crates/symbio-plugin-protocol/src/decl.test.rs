//! `decl.rs` 的单元测试 —— 格位、名字空间、视图折叠与「第二方」判据。
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
    "crates/symbio-plugin-protocol-test-harness",
    harness_declarations
);

fn proposed(name: &str, title: &str, messages: &[&str]) -> Event {
    Event::pending(
        "e-1",
        PROTOCOL_PROPOSED,
        Entity::Domain,
        Verb::Asserted,
        "session/a",
    )
    .with_payload(json!({
        "name": name,
        "title": title,
        "purpose": "p",
        "messages": messages,
    }))
}

fn joined(name: &str, participant: &str) -> Event {
    Event::pending(
        "e-2",
        PROTOCOL_JOINED,
        Entity::Domain,
        Verb::Asserted,
        "session/a",
    )
    .with_payload(json!({
        "name": name,
        "participant": participant,
    }))
}

// ==================== 声明面 ====================

/// 两条事实都落在 `Domain × Asserted`，owner 是本插件。
#[test]
fn both_domain_events_resolve_through_the_process_assembly() {
    let frozen = symbio_plugin_api::sealed_declarations().expect("测试进程的装配表应能冻结");
    for name in [PROTOCOL_PROPOSED, PROTOCOL_JOINED] {
        assert_eq!(
            frozen.event_cell(name),
            Some((Entity::Domain, Verb::Asserted)),
            "{name} 的格位"
        );
        assert_eq!(frozen.event_owner(name), Some(OWNER), "{name} 的 owner");
    }
}

/// 视图名与事实名共用同一名字空间——同名会让 `seal` 直接红。
#[test]
fn the_view_name_does_not_collide_with_the_fact_names() {
    assert_ne!(PROTOCOL_CATALOG, PROTOCOL_PROPOSED);
    assert_ne!(PROTOCOL_CATALOG, PROTOCOL_JOINED);
}

/// 视图读的字段必须都在 `FactDecl.fields` 里（`seal` 的第 3 条）。
#[test]
fn the_view_only_reads_declared_fields() {
    let declared: Vec<&str> = FACTS
        .iter()
        .flat_map(|f| f.fields.iter().copied())
        .collect();
    for (fact, field) in VIEWS[0].reads {
        assert!(
            *fact == PROTOCOL_PROPOSED || *fact == PROTOCOL_JOINED,
            "视图只读本插件自己的事实"
        );
        assert!(declared.contains(field), "视图读了没声明的字段：{field}");
    }
}

// ==================== 视图折叠 ====================

/// 只有提案、没有第二方 ⇒ `draft`。
#[test]
fn a_single_participant_is_still_draft() {
    let v = catalog(
        &[proposed("handoff", "任务交接", &["request", "done"])],
        i64::MAX,
    );
    assert_eq!(v["count"], 1, "{v}");
    assert_eq!(v["by_status"]["draft"], 1, "{v}");
    assert_eq!(v["by_status"]["established"], 0, "{v}");
    assert_eq!(v["protocols"][0]["status"], "draft");
    assert_eq!(v["protocols"][0]["messages"], json!(["request", "done"]));
}

/// 提案 + 第二方 ⇒ `established`；`by_status` 只计当前状态。
#[test]
fn a_second_participant_establishes_the_protocol() {
    let v = catalog(
        &[
            proposed("handoff", "任务交接", &["request", "done"]),
            proposed("review", "评审", &["submit", "verdict"]),
            joined("handoff", "reviewer"),
            joined("review", "main"),
            joined("review", "reviewer"),
        ],
        i64::MAX,
    );
    assert_eq!(v["count"], 2, "{v}");
    assert_eq!(v["by_status"]["draft"], 1, "{v}");
    assert_eq!(v["by_status"]["established"], 1, "{v}");

    let handoff = v["protocols"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "handoff")
        .expect("handoff 应在清单里");
    assert_eq!(handoff["status"], "draft");
    assert_eq!(handoff["participants"], json!(["reviewer"]));

    let review = v["protocols"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "review")
        .expect("review 应在清单里");
    assert_eq!(review["status"], "established");
    assert_eq!(review["participants"], json!(["main", "reviewer"]));
}

/// 同名再提案 ⇒ 后写覆盖（标题 / 消息序列按最后一次），`observations` 仍记两次。
#[test]
fn a_later_proposal_overwrites_the_same_name() {
    let v = catalog(
        &[
            proposed("handoff", "旧标题", &["request"]),
            proposed("handoff", "新标题", &["request", "accept", "done"]),
        ],
        i64::MAX,
    );
    assert_eq!(v["count"], 1, "{v}");
    assert_eq!(v["observations"], 2, "两次提案都是证据：{v}");
    assert_eq!(v["protocols"][0]["title"], "新标题");
    assert_eq!(
        v["protocols"][0]["messages"],
        json!(["request", "accept", "done"])
    );
}

/// 同一参与者重复加入只算一个人——「第二方」按**不同参与者**数，不按事件条数。
#[test]
fn repeated_joins_by_one_participant_do_not_establish() {
    let v = catalog(
        &[
            proposed("handoff", "任务交接", &["request"]),
            joined("handoff", "reviewer"),
            joined("handoff", "reviewer"),
        ],
        i64::MAX,
    );
    assert_eq!(v["protocols"][0]["participants"], json!(["reviewer"]));
    assert_eq!(v["protocols"][0]["status"], "draft");
}

/// 加入一个没有被提出的名字不产生条目（协议必须先被提出）。
#[test]
fn a_join_without_a_proposal_is_ignored() {
    let v = catalog(&[joined("ghost", "reviewer")], i64::MAX);
    assert_eq!(v["count"], 0, "{v}");
    assert_eq!(v["observations"], 0, "无效加入不计证据：{v}");
}

/// 缺字段 / 消息序列不是非空字符串数组的载荷静默跳过。
#[test]
fn malformed_payloads_are_skipped() {
    let no_title = Event::pending(
        "e-3",
        PROTOCOL_PROPOSED,
        Entity::Domain,
        Verb::Asserted,
        "s",
    )
    .with_payload(json!({ "name": "handoff", "purpose": "p", "messages": ["a"] }));
    let mut bad_messages = proposed("review", "评审", &["a"]);
    bad_messages.payload["messages"] = json!(["a", ""]);
    let v = catalog(
        &[no_title, bad_messages, proposed("good", "好", &["a"])],
        i64::MAX,
    );
    assert_eq!(v["count"], 1, "{v}");
    assert_eq!(v["observations"], 1, "坏载荷不计入证据：{v}");
    assert_eq!(v["protocols"][0]["name"], "good");
}

/// 消息序列去空白、按首次出现去重。
#[test]
fn messages_are_trimmed_and_deduplicated() {
    let v = catalog(
        &[proposed("handoff", "任务交接", &[" a ", "a", "b"])],
        i64::MAX,
    );
    assert_eq!(v["protocols"][0]["messages"], json!(["a", "b"]));
}

/// 未来事件不参与折叠（`ts > now`）。
#[test]
fn future_events_are_ignored() {
    let mut e = proposed("handoff", "任务交接", &["a"]);
    e.ts = 500;
    assert_eq!(catalog(&[e], 100)["count"], 0);
}

/// 超出上限只截输出长度，保留最早提出的名字。
#[test]
fn the_output_is_capped_but_keeps_the_earliest() {
    let events: Vec<Event> = (0..60)
        .map(|i| proposed(&format!("p_{i}"), "t", &["a"]))
        .collect();
    let v = catalog(&events, i64::MAX);
    assert_eq!(v["count"], LIMIT);
    assert_eq!(v["protocols"][0]["name"], "p_0", "留下的是最早 50 个提案");
}
