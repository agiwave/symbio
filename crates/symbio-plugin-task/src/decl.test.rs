//! 装配表在**本 crate 的测试二进制**里也要能冻结。
//!
//! ## 为什么本文件还要交一份「协议行」夹具
//!
//! 本插件的测试二进制只链接自己：协议五格的归宿行住在
//! `crates/symbio-plugin-loop`，不在这里 ⇒ `seal` 报 `OrphanFact`、整张表失败
//! （DE-015 §3.4），于是 [`symbio_plugin_api::vocab`] 退回共享清单——而
//! `task.rework_created` 的 `CountsRework` 角色只在本插件自己的声明里。
//!
//! 夹具补的正是完整进程里由 loop 提供的那一半：五条协议自读行。它与
//! `crates/symbio-plugin-notes/src/decl.test.rs` 的同名夹具同形，只在 `cfg(test)` 生效；
//! 生产装配（全插件链接）不会走到这份提交——`cfg(test)` 不进 lib 的正常构建。

use super::{EVENT_TASK_REWORK_CREATED, OWNER};
use symbio_core::ring::{Decl, Pattern};
use symbio_core::{Entity, LatencyTier, PluginDeclarations};

/// 协议五格的自读行：完整进程里由 loop 交表，本二进制不链接它 ⇒ 夹具补上。
fn protocol_rows() -> &'static [Decl] {
    let mut out = Vec::new();
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
    "crates/symbio-plugin-task-test-harness",
    harness_declarations
);

/// 本 crate 交的表在装配后能按名字解析，且返工角色留在 `task.rework_created` 上——
/// 后者是 `rework_bounded` 唯一认它的依据（D-203：读侧不抄名字）。
#[test]
fn the_task_event_resolves_through_the_process_assembly() {
    let frozen = symbio_plugin_api::sealed_declarations().expect("测试进程的装配表应能冻结");
    assert_eq!(
        frozen.event_cell(EVENT_TASK_REWORK_CREATED),
        Some((Entity::Task, symbio_core::Verb::Asserted))
    );
    assert_eq!(frozen.event_owner(EVENT_TASK_REWORK_CREATED), Some(OWNER));
    assert!(frozen.holds_role(
        EVENT_TASK_REWORK_CREATED,
        symbio_core::FactRole::CountsRework
    ));
}
