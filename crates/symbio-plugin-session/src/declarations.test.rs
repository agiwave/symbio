//! `crates/symbio-plugin-session/src/declarations.rs` 的单元测试。
//!
//! ## 为什么本文件还要交一份「协议行」夹具
//!
//! 插件自己的单元测试二进制**只链接本 crate**：装配表里只有内核协议五格与本插件
//! 声明的三个事件，而协议事实的归宿行住在 `plugins/loop`——不在这里，就没有行读它们，
//! `seal` 判据 7 报 `OrphanFact`，整张表失败（D-198「失败是整体失败」），写闸于是
//! 退到共享表。`system.*` 由本插件声明，不在共享表里 ⇒ 写不进去。
//!
//! 夹具补的正是**完整进程里由 loop 提供的那一半**：五条自读行让协议事实有归宿，
//! 冻结表因此在本 crate 的测试进程里成立，端到端写闸用例才测得到 owner 声明。
//! 它与 `symbio-plugin-api/src/declarations.test.rs` 的夹具同形，只在 `cfg(test)` 生效。
//!
//! 生产二进制不受影响：那里的协议行来自 loop 的提交，本夹具不参与编译。

use symbio_core::ring::{Decl, Pattern};
use symbio_core::{LatencyTier, PluginDeclarations};

use super::{EVENT_CONATION_EXPRESSED, EVENT_SYSTEM_HEALTH, EVENT_SYSTEM_TRIGGERED};

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

symbio_plugin_api::submit_declarations!("plugins/session-test-harness", harness_declarations);

#[test]
fn owned_events_resolve_through_the_process_assembly() {
    let frozen = symbio_plugin_api::sealed_declarations().expect("测试进程的装配表应能冻结");
    assert_eq!(
        frozen.event_cell(EVENT_SYSTEM_TRIGGERED),
        Some((symbio_core::Entity::System, symbio_core::Verb::Opened))
    );
    assert_eq!(
        frozen.event_cell(EVENT_SYSTEM_HEALTH),
        Some((symbio_core::Entity::System, symbio_core::Verb::Progressed))
    );
    assert_eq!(
        frozen.event_cell(EVENT_CONATION_EXPRESSED),
        Some((symbio_core::Entity::Conation, symbio_core::Verb::Opened))
    );
    assert_eq!(
        frozen.event_owner(EVENT_SYSTEM_TRIGGERED),
        Some(super::OWNER)
    );
    assert_eq!(
        frozen.event_owner(EVENT_CONATION_EXPRESSED),
        Some(super::OWNER)
    );
}
