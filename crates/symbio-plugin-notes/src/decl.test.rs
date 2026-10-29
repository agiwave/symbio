//! 领域扩展位的判据：本插件声明的事件在装配表里按自己的格位解析。
//!
//! ## 为什么本文件还要交一份「协议行」夹具
//!
//! 插件自己的测试二进制只链接本 crate：协议事实的归宿行住在 `crates/symbio-plugin-loop`，
//! `artifact.added` 的格位与事实声明也在那里。不在这里 ⇒ `seal` 报 `OrphanFact` /
//! `UnknownSource`，整张表失败（DE-015 §3.4）。夹具补的正是完整进程里由 loop 提供的
//! 那一半——五条协议自读行 + 一条产物自读行。它与
//! `symbio-plugin-session/src/declarations.test.rs` 的同名夹具同形，只在 `cfg(test)` 生效。

use super::{NOTES_CREATED, NOTES_INDEXED, NOTES_INSIGHT, OWNER};
use symbio_core::ring::{Decl, Pattern};
use symbio_core::{ActorDecl, Entity, EventDecl, FactDecl, LatencyTier, PluginDeclarations, Verb};

/// loop 拥有的产物格：`notes.index` 的 `owes.source`。完整进程里由 loop 交表，
/// 本二进制不链接它 ⇒ 夹具补上格位与载荷字段。
const HARNESS_EVENTS: &[EventDecl] = &[EventDecl::new(
    symbio_plugin_api::events::EVENT_ARTIFACT_ADDED,
    (Entity::Artifact, Verb::Asserted),
)];

const HARNESS_FACTS: &[FactDecl] = &[FactDecl {
    name: symbio_plugin_api::events::EVENT_ARTIFACT_ADDED,
    port: None,
    key: &[],
    expand: &[],
    fields: &["tool", "tool_call_id", "success", "args"],
}];

fn protocol_rows() -> &'static [Decl] {
    let mut out = Vec::new();
    for name in [
        symbio_core::EVENT_REQUEST,
        symbio_core::EVENT_SENT,
        symbio_core::EVENT_RESPONSE,
        symbio_core::EVENT_FAILED,
        symbio_core::EVENT_RETRY,
        symbio_plugin_api::events::EVENT_ARTIFACT_ADDED,
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
        events: HARNESS_EVENTS,
        facts: HARNESS_FACTS,
        views: &[],
        ports: &[],
        // `notes.index` 的 `by` 是宿主词汇 `plugin_row`：完整进程里这条 `ActorDecl`
        // 由 loop 的装配给出（写入名单取自实现），本二进制不链接 loop ⇒ 夹具补上。
        actors: vec![ActorDecl {
            name: symbio_plugin_api::rows::PLUGIN_ROW,
            mode: LatencyTier::Deep,
            reads: &[],
            writes: &[NOTES_INDEXED, NOTES_INSIGHT],
        }],
        rows: protocol_rows(),
    }
}

symbio_plugin_api::submit_declarations!(
    "crates/symbio-plugin-notes-test-harness",
    harness_declarations
);

/// `notes.created` 落在 `Domain × Opened`，owner 是本插件。
///
/// 本 crate 的测试二进制只链接它自己的声明表 ⇒ 这条断言读的就是「新增能力包
/// 自己交的表」，没有别的插件代答。
#[test]
fn the_domain_event_resolves_through_the_process_assembly() {
    let frozen = symbio_plugin_api::sealed_declarations().expect("测试进程的装配表应能冻结");
    assert_eq!(
        frozen.event_cell(NOTES_CREATED),
        Some((Entity::Domain, Verb::Opened))
    );
    assert_eq!(frozen.event_owner(NOTES_CREATED), Some(OWNER));
}

/// 扩展位不是「多了一个没人管的变体」：同一张表里它有自己的名字，不与内核协议名撞车。
#[test]
fn the_domain_event_does_not_collide_with_protocol_names() {
    for name in [
        symbio_core::EVENT_REQUEST,
        symbio_core::EVENT_SENT,
        symbio_core::EVENT_RESPONSE,
        symbio_core::EVENT_FAILED,
        symbio_core::EVENT_RETRY,
    ] {
        assert_ne!(name, NOTES_CREATED);
    }
}
