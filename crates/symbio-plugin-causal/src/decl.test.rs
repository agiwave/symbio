//! `decl.rs` 的单元测试 —— 格位、名字空间与视图折叠。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 本 crate 的测试二进制只链接它自己的声明表（`causal.link` 的归宿行也在自己这里），
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
    "crates/symbio-plugin-causal-test-harness",
    harness_declarations
);

fn link(cause: &str, effect: &str, confidence: u64) -> Event {
    Event::pending(
        "e-1",
        CAUSAL_LINK,
        Entity::Domain,
        Verb::Asserted,
        "session/a",
    )
    .with_payload(json!({ "cause": cause, "effect": effect, "confidence": confidence }))
}

// ==================== 声明面 ====================

/// `causal.link` 落在 `Domain × Asserted`，owner 是本插件。
///
/// 格位是断言（不是开链）：一件事被断言过，可以后来被更强的证据压过。
#[test]
fn the_domain_event_resolves_through_the_process_assembly() {
    let frozen = symbio_plugin_api::sealed_declarations().expect("测试进程的装配表应能冻结");
    assert_eq!(
        frozen.event_cell(CAUSAL_LINK),
        Some((Entity::Domain, Verb::Asserted))
    );
    assert_eq!(frozen.event_owner(CAUSAL_LINK), Some(OWNER));
}

/// 视图名与事实名共用同一名字空间（视图是合成事实）——两者同名会让 `seal` 直接红，
/// 也会让读方分不清自己读到的是「折出来的」还是「记下来的」。
#[test]
fn the_view_name_does_not_collide_with_the_fact_name() {
    assert_ne!(CAUSAL_LINKS, CAUSAL_LINK);
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
        assert_eq!(*fact, CAUSAL_LINK, "视图只读本插件自己的事实");
        assert!(declared.contains(field), "视图读了没声明的字段：{field}");
    }
}

// ==================== 视图折叠 ====================

/// 一条链 `a → b → c`：两条边，根因只有 `a`（`b` 有入边）。
#[test]
fn a_chain_yields_one_root_cause() {
    let events = [
        link("内存不足", "任务超时", 70),
        link("任务超时", "重试排队", 60),
    ];
    let v = causal_graph(&events, i64::MAX);
    assert_eq!(v["count"], 2);
    assert_eq!(v["edges"][0]["from"], "内存不足");
    assert_eq!(v["edges"][1]["to"], "重试排队");
    assert_eq!(v["causes"], json!(["内存不足"]), "只有起点没有上游");
}

/// 两条互不相连的边 ⇒ 两个根因；顺序按断言序（写入序），不替读方排序。
#[test]
fn independent_edges_yield_two_root_causes_in_write_order() {
    let events = [link("a", "b", 50), link("c", "d", 50)];
    let v = causal_graph(&events, i64::MAX);
    assert_eq!(v["count"], 2);
    assert_eq!(v["causes"], json!(["a", "c"]));
}

/// 方向相反的断言各自成立（`a → b` 与 `b → a`）：读方看得见「互为因果」这件事，
/// 本层不替它裁决哪一条对。
#[test]
fn opposite_directions_both_survive() {
    let events = [link("a", "b", 50), link("b", "a", 40)];
    let v = causal_graph(&events, i64::MAX);
    assert_eq!(v["count"], 2);
    assert_eq!(v["causes"], json!([]), "互为上游 ⇒ 没有「只出不进」的节点");
}

/// 未来事件不参与折叠（`ts > now`）：视图的 `now` 是这一轮的时钟。
#[test]
fn future_events_are_ignored() {
    let mut e = link("a", "b", 50);
    e.ts = 500;
    assert_eq!(causal_graph(&[e], 100)["count"], 0);
}

/// 缺字段 / 自环的载荷静默跳过：视图不能因为一条坏事实就整个算不出来。
#[test]
fn malformed_payloads_are_skipped() {
    let broken = Event::pending("e-2", CAUSAL_LINK, Entity::Domain, Verb::Asserted, "s")
        .with_payload(json!({ "cause": "a" }));
    let v = causal_graph(&[broken, link("x", "y", 50)], i64::MAX);
    assert_eq!(v["count"], 1);
}

/// 超出上限只截**输出长度**，保留最新的断言：早期断言被修正压过时，读方看到修正。
#[test]
fn the_output_is_capped_but_keeps_the_latest() {
    let events: Vec<Event> = (0..60)
        .map(|i| link(&format!("c{i}"), &format!("e{i}"), 50))
        .collect();
    let v = causal_graph(&events, i64::MAX);
    assert_eq!(v["count"], LIMIT);
    assert_eq!(v["edges"][0]["from"], "c10", "留下的是最后 50 条");
    assert_eq!(v["edges"][LIMIT - 1]["from"], "c59");
}

/// 置信度原样透出（不做归一化）：它是断言强度，换算成别的刻度就丢了原意。
#[test]
fn confidence_is_carried_through() {
    let v = causal_graph(&[link("a", "b", 87)], i64::MAX);
    assert_eq!(v["edges"][0]["confidence"], 87);
}
