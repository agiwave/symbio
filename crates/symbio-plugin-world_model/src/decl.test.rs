//! `decl.rs` 的单元测试 —— 格位、名字空间与视图折叠。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 本 crate 的测试二进制只链接它自己的声明表（`world.observed` 的归宿行也在自己这里），
//! 因此这些断言读的就是「新增能力包自己交的表」，没有别的插件代答。

use super::*;
use serde_json::json;
use symbio_plugin_api::ring::{Decl, Pattern};
use symbio_plugin_api::{Entity, Event, LatencyTier, PluginDeclarations, Verb};

/// 协议五格的**自读行** —— 完整进程里由 `crates/symbio-plugin-loop` 交表，
/// 本 crate 的测试二进制不链接它 ⇒ 夹具补上。
///
/// 不补的后果不是「少几个测试」：`seal` 会把这五条协议事实判成 `OrphanFact`
/// （每条事实都要有归宿行），整张装配表失败，于是「本插件的事实落在正确格位」这条
/// 断言根本无法执行。夹具与 `notes` / `session` 的同名实现同形，只在 `cfg(test)` 生效。
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
    "crates/symbio-plugin-world_model-test-harness",
    harness_declarations
);

fn observed(entity: &str, attribute: &str, value: &str, confidence: u64) -> Event {
    Event::pending(
        "e-1",
        WORLD_OBSERVED,
        Entity::Domain,
        Verb::Asserted,
        "session/a",
    )
    .with_payload(json!({
        "entity": entity,
        "attribute": attribute,
        "value": value,
        "confidence": confidence,
    }))
}

// ==================== 声明面 ====================

/// `world.observed` 落在 `Domain × Asserted`，owner 是本插件。
///
/// 格位是断言（不是开链）：一次观察可以被后来的观察压过。
#[test]
fn the_domain_event_resolves_through_the_process_assembly() {
    let frozen = symbio_plugin_api::sealed_declarations().expect("测试进程的装配表应能冻结");
    assert_eq!(
        frozen.event_cell(WORLD_OBSERVED),
        Some((Entity::Domain, Verb::Asserted))
    );
    assert_eq!(frozen.event_owner(WORLD_OBSERVED), Some(OWNER));
}

/// 视图名与事实名共用同一名字空间（视图是合成事实）——两者同名会让 `seal` 直接红，
/// 也会让读方分不清自己读到的是「折出来的」还是「记下来的」。
#[test]
fn the_view_name_does_not_collide_with_the_fact_name() {
    assert_ne!(WORLD_MODEL, WORLD_OBSERVED);
}

/// 视图读的字段必须都在 `FactDecl.fields` 里（`seal` 的第 3 条）——这里把它写成断言，
/// 免得「改字段忘了改视图」只在整轮门禁里才发现。
#[test]
fn the_view_only_reads_declared_fields() {
    let declared: Vec<&str> = FACTS
        .iter()
        .flat_map(|f| f.fields.iter().copied())
        .collect();
    for (fact, field) in VIEWS[0].reads {
        assert_eq!(*fact, WORLD_OBSERVED, "视图只读本插件自己的事实");
        assert!(declared.contains(field), "视图读了没声明的字段：{field}");
    }
}

// ==================== 视图折叠 ====================

/// 同一实体同一属性的两次观察 ⇒ 后写覆盖，`observations` 仍记两次证据。
///
/// 这是「世界模型回答现在是什么，不保存时间线」的可执行形式；把 `observations`
/// 也钉住，是因为一个直接按 `(entity, attribute)` 去重的实现会让 `count` 与后一条
/// 断言都绿，却悄悄丢掉「改过一次」这件事。
#[test]
fn a_later_observation_overwrites_the_same_attribute() {
    let events = [
        observed("服务器", "状态", "在线", 80),
        observed("服务器", "状态", "离线", 60),
    ];
    let v = world_model(&events, i64::MAX);
    assert_eq!(v["count"], 1, "一个实体：{}", v);
    assert_eq!(v["observations"], 2, "两次观察都是证据：{}", v);
    assert_eq!(v["entities"][0]["entity"], "服务器");
    assert_eq!(v["entities"][0]["attributes"]["状态"]["value"], "离线");
    assert_eq!(
        v["entities"][0]["attributes"]["状态"]["confidence"], 60,
        "置信度跟随后一条观察"
    );
}

/// 不同属性、不同实体各自成格：属性属于实体，不能被折成一张全局表。
#[test]
fn distinct_attributes_and_entities_are_kept_apart() {
    let events = [
        observed("服务器", "状态", "在线", 80),
        observed("服务器", "CPU", "8 核", 90),
        observed("数据库", "版本", "16", 95),
    ];
    let v = world_model(&events, i64::MAX);
    assert_eq!(v["count"], 2, "两个实体：{}", v);
    assert_eq!(v["observations"], 3);
    assert_eq!(v["entities"][0]["entity"], "服务器");
    assert_eq!(v["entities"][0]["attributes"]["状态"]["value"], "在线");
    assert_eq!(v["entities"][0]["attributes"]["CPU"]["value"], "8 核");
    assert_eq!(v["entities"][1]["entity"], "数据库");
    assert_eq!(v["entities"][1]["attributes"]["版本"]["value"], "16");
    assert!(
        v["entities"][1]["attributes"].get("状态").is_none(),
        "别的实体的属性不能漏进这一格：{}",
        v
    );
}

/// 未来事件不参与折叠（`ts > now`）：视图的 `now` 是这一轮的时钟。
#[test]
fn future_events_are_ignored() {
    let mut e = observed("服务器", "状态", "在线", 80);
    e.ts = 500;
    assert_eq!(world_model(&[e], 100)["count"], 0);
}

/// 缺字段 / 空值的载荷静默跳过：视图不能因为一条坏事实就整个算不出来。
#[test]
fn malformed_payloads_are_skipped() {
    let missing_attribute =
        Event::pending("e-2", WORLD_OBSERVED, Entity::Domain, Verb::Asserted, "s")
            .with_payload(json!({ "entity": "服务器", "value": "在线" }));
    let blank_value = observed("服务器", "状态", "  ", 80);
    let v = world_model(
        &[missing_attribute, blank_value, observed("x", "y", "z", 50)],
        i64::MAX,
    );
    assert_eq!(v["count"], 1);
    assert_eq!(v["observations"], 1, "坏载荷不计入证据量：{}", v);
}

/// 超出上限截**输出长度**，保留最早进入世界的实体（长期累积的读数，先来的更可能相关）。
#[test]
fn the_output_is_capped_but_keeps_the_earliest() {
    let events: Vec<Event> = (0..60)
        .map(|i| observed(&format!("e{i}"), "状态", "x", 50))
        .collect();
    let v = world_model(&events, i64::MAX);
    assert_eq!(v["count"], LIMIT);
    assert_eq!(v["entities"][0]["entity"], "e0", "留下的是最早 50 个实体");
    assert_eq!(v["entities"][LIMIT - 1]["entity"], "e49");
}

/// 置信度原样透出（不做归一化）：它是观察强度，换算成别的刻度就丢了原意。
#[test]
fn confidence_is_carried_through() {
    let v = world_model(&[observed("a", "b", "c", 87)], i64::MAX);
    assert_eq!(v["entities"][0]["attributes"]["b"]["confidence"], 87);
}
