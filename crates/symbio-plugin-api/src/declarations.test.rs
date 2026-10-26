//! `crates/symbio-plugin-api/src/declarations.rs` 的单元测试。
//!
//! 判据：链接期提交进来的声明表**能被装配方看到**，且冻结后的格位与 owner 与提交一致。
//! 端到端那一半（写闸按冻结表收 / 拒）住在 `symbio-plugin-session` 的
//! `injected_declarations_are_the_grid_the_gate_reads`（DE-024 D-200）：本 crate 只验
//! 收集通道，不复制写闸实现。

use super::*;
use symbio_core::ring::{Decl, Pattern};
use symbio_core::{EventDecl, LatencyTier};

/// 协议五格的归宿行：装配表必须能通过 `seal` 的判据 7，否则收集通道的用例
/// 会先被「没人消费的事实」挡住，测不到收集本身。
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

fn fixture() -> PluginDeclarations<'static> {
    PluginDeclarations {
        events: &[EventDecl {
            name: "fixture.declared",
            cell: (Entity::Turn, Verb::Asserted),
        }],
        facts: &[],
        views: &[],
        ports: &[],
        actors: Vec::new(),
        rows: protocol_rows(),
    }
}

inventory::submit! {
    DeclarationsDecl {
        owner: "fixture/declarations",
        make: fixture,
    }
}

#[test]
fn assemble_collects_submitted_declarations() {
    let frozen = assemble_declarations().expect("夹具声明应能冻结");
    assert_eq!(
        frozen.event_cell("fixture.declared"),
        Some((Entity::Turn, Verb::Asserted))
    );
    assert_eq!(
        frozen.event_owner("fixture.declared"),
        Some("fixture/declarations")
    );
    // 内核协议五格始终在表上，不依赖任何插件的提交。
    assert!(frozen.event_cell(symbio_core::EVENT_REQUEST).is_some());
}
