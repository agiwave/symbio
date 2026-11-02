//! `decl.rs` 的单元测试 —— 格位、名字空间与视图折叠。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! 本 crate 的测试二进制只链接它自己的声明表（`explain.rationale` 的归宿行也在自己这里），
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
        events: &[],
        facts: &[],
        views: &[],
        ports: &[],
        actors: Vec::new(),
        rows: protocol_rows(),
    }
}

symbio_plugin_api::submit_declarations!(
    "crates/symbio-plugin-explain-test-harness",
    harness_declarations
);

fn rationale(decision: &str, why: &str, evidence: &str) -> Event {
    Event::pending(
        "e-1",
        EXPLAIN_RATIONALE,
        Entity::Domain,
        Verb::Asserted,
        1,
        "session/a",
    )
    .with_payload(json!({ "decision": decision, "why": why, "evidence": evidence }))
}

// ==================== 声明面 ====================

/// `explain.rationale` 落在 `Domain × Asserted`，owner 是本插件。
///
/// 格位是断言（不是开链）：一次决定的理由被记下，可以后来被更准确的记录压过。
#[test]
fn the_domain_event_resolves_through_the_process_assembly() {
    let frozen = symbio_plugin_api::sealed_declarations().expect("测试进程的装配表应能冻结");
    assert_eq!(
        frozen.event_cell(EXPLAIN_RATIONALE),
        Some((Entity::Domain, Verb::Asserted))
    );
    assert_eq!(frozen.event_owner(EXPLAIN_RATIONALE), Some(OWNER));
}

/// 视图名与事实名共用同一名字空间（视图是合成事实）——两者同名会让 `seal` 直接红，
/// 也会让读方分不清自己读到的是「折出来的」还是「记下来的」。
#[test]
fn the_view_name_does_not_collide_with_the_fact_name() {
    assert_ne!(EXPLAIN_RATIONALES, EXPLAIN_RATIONALE);
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
        assert_eq!(*fact, EXPLAIN_RATIONALE, "视图只读本插件自己的事实");
        assert!(declared.contains(field), "视图读了没声明的字段：{field}");
    }
}

// ==================== 视图折叠 ====================

/// 两条不同决定的记录 ⇒ 两条条目、两个决定，顺序按写入序。
#[test]
fn two_decisions_yield_two_entries_in_write_order() {
    let events = [
        rationale("切换到增量编译", "全量编译超过预算", ""),
        rationale("回退到上一版", "新版本回归失败", "回归日志"),
    ];
    let v = rationale_ledger(&events, i64::MAX);
    assert_eq!(v["count"], 2);
    assert_eq!(v["entries"][0]["decision"], "切换到增量编译");
    assert_eq!(v["entries"][0]["why"], "全量编译超过预算");
    assert_eq!(v["entries"][0]["evidence"], "");
    assert_eq!(v["entries"][1]["evidence"], "回归日志");
    assert_eq!(
        v["decisions"],
        json!(["切换到增量编译", "回退到上一版"]),
        "决定列表按首次出现去重"
    );
}

/// 同一个决定记两次不同的理由 ⇒ 两条条目，但 `decisions` 只留一次。
///
/// 它是**折出来的**判据：一个把每条事实原样吐出 `decision` 的实现会让长度为 2。
#[test]
fn repeated_decisions_are_deduplicated() {
    let events = [
        rationale("切换到增量编译", "全量编译超过预算", ""),
        rationale("切换到增量编译", "缓存命中率更高", ""),
    ];
    let v = rationale_ledger(&events, i64::MAX);
    assert_eq!(v["count"], 2, "理由条目各记一条");
    assert_eq!(v["decisions"], json!(["切换到增量编译"]));
}

/// 未来事件不参与折叠（`ts > now`）：视图的 `now` 是这一轮的时钟。
#[test]
fn future_events_are_ignored() {
    let mut e = rationale("a", "b", "");
    e.ts = 500;
    assert_eq!(rationale_ledger(&[e], 100)["count"], 0);
}

/// 缺字段的载荷静默跳过：视图不能因为一条坏事实就整个算不出来。
#[test]
fn malformed_payloads_are_skipped() {
    let broken = Event::pending(
        "e-2",
        EXPLAIN_RATIONALE,
        Entity::Domain,
        Verb::Asserted,
        1,
        "s",
    )
    .with_payload(json!({ "decision": "a" }));
    let v = rationale_ledger(&[broken, rationale("x", "y", "")], i64::MAX);
    assert_eq!(v["count"], 1);
}

/// 超出上限只截**输出长度**，保留最新的记录：早期记录被修正压过时，读方看到修正。
#[test]
fn the_output_is_capped_but_keeps_the_latest() {
    let events: Vec<Event> = (0..60)
        .map(|i| rationale(&format!("d{i}"), &format!("w{i}"), ""))
        .collect();
    let v = rationale_ledger(&events, i64::MAX);
    assert_eq!(v["count"], LIMIT);
    assert_eq!(v["entries"][0]["decision"], "d10", "留下的是最后 50 条");
    assert_eq!(v["entries"][LIMIT - 1]["decision"], "d59");
    assert_eq!(v["decisions"].as_array().map(Vec::len), Some(LIMIT));
}
