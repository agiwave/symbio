//! `decl.rs` 的单元测试 —— 格位、名字空间与视图折叠。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 本 crate 的测试二进制只链接它自己的声明表，因此这些断言读的就是「新增能力包自己交
//! 的表」，没有别的插件代答。

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
    "crates/symbio-plugin-reversibility-test-harness",
    harness_declarations
);

fn assessed(action: &str, level: &str, undo: &str) -> Event {
    Event::pending(
        "e-1",
        REVERSIBILITY_ASSESSED,
        Entity::Domain,
        Verb::Asserted,
        "session/a",
    )
    .with_payload(json!({
        "action": action,
        "level": level,
        "undo": undo,
    }))
}

// ==================== 声明面 ====================

/// `reversibility.assessed` 落在 `Domain × Asserted`，owner 是本插件。
#[test]
fn the_domain_event_resolves_through_the_process_assembly() {
    let frozen = symbio_plugin_api::sealed_declarations().expect("测试进程的装配表应能冻结");
    assert_eq!(
        frozen.event_cell(REVERSIBILITY_ASSESSED),
        Some((Entity::Domain, Verb::Asserted))
    );
    assert_eq!(frozen.event_owner(REVERSIBILITY_ASSESSED), Some(OWNER));
}

/// 视图名与事实名共用同一名字空间——同名会让 `seal` 直接红。
#[test]
fn the_view_name_does_not_collide_with_the_fact_name() {
    assert_ne!(REVERSIBILITY_REGISTER, REVERSIBILITY_ASSESSED);
}

/// 视图读的字段必须都在 `FactDecl.fields` 里（`seal` 的第 3 条）。
#[test]
fn the_view_only_reads_declared_fields() {
    let declared: Vec<&str> = FACTS
        .iter()
        .flat_map(|f| f.fields.iter().copied())
        .collect();
    for (fact, field) in VIEWS[0].reads {
        assert_eq!(*fact, REVERSIBILITY_ASSESSED, "视图只读本插件自己的事实");
        assert!(declared.contains(field), "视图读了没声明的字段：{field}");
    }
}

// ==================== 视图折叠 ====================

/// 同一动作的两次判断 ⇒ 后写覆盖，`observations` 仍记两次证据。
///
/// 这里把「可补偿 → 不可逆」的升级钉住：登记表必须能反映**更保守的**后一次判断，
/// 否则读方会继续按「有后路」行动。
#[test]
fn a_later_assessment_overwrites_the_same_action() {
    let events = [
        assessed("改配置", "compensable", "把旧配置写回"),
        assessed("改配置", "irreversible", ""),
    ];
    let v = reversibility_register(&events, i64::MAX);
    assert_eq!(v["count"], 1, "一个动作：{}", v);
    assert_eq!(v["observations"], 2, "两次判断都是证据：{}", v);
    assert_eq!(v["actions"][0]["action"], "改配置");
    assert_eq!(
        v["actions"][0]["level"], "irreversible",
        "后一次判断覆盖前一次"
    );
    assert_eq!(v["actions"][0]["undo"], "", "不可逆动作没有补偿做法");
    assert_eq!(v["by_level"]["irreversible"], 1);
    assert_eq!(v["by_level"]["compensable"], 0);
}

/// 三个等级各自计数：`by_level` 只统计当前登记的动作。
#[test]
fn by_level_counts_only_current_entries() {
    let events = [
        assessed("删缓存", "reversible", ""),
        assessed("改配置", "compensable", "把旧配置写回"),
        assessed("发邮件", "irreversible", ""),
    ];
    let v = reversibility_register(&events, i64::MAX);
    assert_eq!(v["count"], 3);
    assert_eq!(v["observations"], 3);
    for level in LEVELS {
        assert_eq!(v["by_level"][level], 1, "{level} 各一个：{}", v);
    }
    assert_eq!(v["actions"][0]["action"], "删缓存", "按首次标注序");
}

/// `compensable` 没有补偿做法 ⇒ 不登记：它会让执行方误以为后路存在。
#[test]
fn compensable_without_undo_is_skipped() {
    let v = reversibility_register(
        &[
            assessed("改配置", "compensable", "  "),
            assessed("删缓存", "reversible", ""),
        ],
        i64::MAX,
    );
    assert_eq!(v["count"], 1, "只有可逆动作登记：{}", v);
    assert_eq!(v["observations"], 1);
}

/// 不认识的等级静默跳过：视图不能因为一条坏事实就整个算不出来。
#[test]
fn unknown_levels_are_skipped() {
    let v = reversibility_register(&[assessed("x", "maybe", "")], i64::MAX);
    assert_eq!(v["count"], 0);
    assert_eq!(v["observations"], 0);
}

/// 未来判断不参与折叠（`ts > now`）。
#[test]
fn future_events_are_ignored() {
    let mut e = assessed("删缓存", "reversible", "");
    e.ts = 500;
    assert_eq!(reversibility_register(&[e], 100)["count"], 0);
}

/// 超出上限只截输出长度，保留最早登记的动作。
#[test]
fn the_output_is_capped_but_keeps_the_earliest() {
    let events: Vec<Event> = (0..60)
        .map(|i| assessed(&format!("a{i}"), "reversible", ""))
        .collect();
    let v = reversibility_register(&events, i64::MAX);
    assert_eq!(v["count"], LIMIT);
    assert_eq!(v["actions"][0]["action"], "a0", "留下的是最早 50 个动作");
    assert_eq!(v["actions"][LIMIT - 1]["action"], "a49");
}

/// 补偿做法原样透出：读方要靠它决定「执行前先确认什么」。
#[test]
fn undo_is_carried_through() {
    let v = reversibility_register(
        &[assessed("改配置", "compensable", "把旧配置写回")],
        i64::MAX,
    );
    assert_eq!(v["actions"][0]["undo"], "把旧配置写回");
}
