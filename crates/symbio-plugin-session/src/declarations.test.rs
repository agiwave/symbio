//! `crates/symbio-plugin-session/src/declarations.rs` 的单元测试。
//!
//! ## 为什么本文件还要交一份「协议行」夹具
//!
//! 插件自己的单元测试二进制**只链接本 crate**：平台提交 sealed declarations
//! （通用执行体与 `await-port` 行），本插件声明的事件（`system.*` / `session.*` /
//! 对话面）也在表里。协议五格的归宿行仍由本夹具补——没有行读它们时 `seal` 判据 7
//! 报 `OrphanFact`，整张表失败（「失败是整体失败」），写闸于是退到共享表。
//! `system.*` 由本插件声明，不在共享表里 ⇒ 写不进去。
//!
//! 夹具只在 `cfg(test)` 生效；生产二进制的平台声明与插件声明照常冻结。
//!
//! ## 为什么这里**不**给能力视图交替身
//!
//! 主动性读数的 owner 是主动性能力包，本插件的读方按名字读它。
//! 本二进制不链接那个包（E-009），于是 `conation.intents` 在这里是**可诊断的缺席**——
//! 这正是要测的形状之一（`capability_views` 的用例逐条断言「本进程没链接 owner ⇒ 必须出声」）。
//! 交一份「算法一样」的替身会把那条用例变成假绿，也会让「卸载能力包之后系统还完整」这句话
//! 失去判据：替身不是 owner，它只是把红改绿。
//!
//! 读数的**数字**归能力包自己的用例与 e2e——真实装配里跑的是 owner 那一份。

use symbio_platform::kernel::decl::ring::{Decl, Pattern};
use symbio_platform::kernel::decl::PluginDeclarations;
use symbio_platform::kernel::event::{
    Entity, EventDecl, Verb, EVENT_FAILED, EVENT_REQUEST, EVENT_RESPONSE, EVENT_RETRY, EVENT_SENT,
};

use super::{EVENT_SYSTEM_HEALTH, EVENT_SYSTEM_TRIGGERED};

/// 技能观测的**格位**夹具：owner 是 `crates/symbio-plugin-skill`，它的折叠也住
/// 在那里，而这条名字除 owner 之外没有生产读方 ⇒ 不在清单生成物里（`CS-007`）。
///
/// 写闸只认声明过的格位（fail-closed：三份表都不认识的名字答 `OffGrid`），而 `stats` 的
/// 校准列用例要**把真实观测写进环里**才试得出「owner 不在本进程」这一种缺席。所以这里补
/// 一条与 owner 同构的 `EventDecl`——**只补格位，不补视图**：`skill.calibration` 保持读
/// 不到，那正是被测的形状（数字归 owner 的单测与 e2e）。
const HARNESS_EVENTS: &[EventDecl] = &[EventDecl::new(
    "memory.calibrated",
    (Entity::Memory, Verb::Asserted),
)];

/// 协议五格的**覆盖行**：内核无条件声明协议事实（`PROTOCOL_EVENTS` /
/// `PROTOCOL_FACTS`），而 `seal` 要求每个被声明的事实至少有一行读到它——否则报
/// `OrphanFact`。本插件的单测二进制不链接 `loop`，那五行生产读者不在场，夹具因此
/// 补五行同名的自读行（`owes` 与 `until` 同源 ⇒ 开不出账，这里只补「这一格有人读」）。
fn protocol_rows() -> &'static [Decl] {
    let mut out = Vec::new();
    for name in [
        EVENT_REQUEST,
        EVENT_SENT,
        EVENT_RESPONSE,
        EVENT_FAILED,
        EVENT_RETRY,
    ] {
        let pattern: &'static [Pattern] = Box::leak(Box::new([Pattern {
            source: name,
            key: &[],
            eq: &[],
        }]));
        out.push(Decl {
            name,
            reads: &[],
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
        facts: &[],
        views: &[],
        ports: &[],
        // 通用执行体由平台 sealed declarations 提供，夹具只补协议五格的读者。
        actors: Vec::new(),
        rows: protocol_rows(),
    }
}

symbio_plugin_api::submit_declarations!(
    "crates/symbio-plugin-session-test-harness",
    harness_declarations
);

#[test]
fn owned_events_resolve_through_the_process_assembly() {
    let frozen = symbio_plugin_api::sealed_declarations().expect("测试进程的装配表应能冻结");
    assert_eq!(
        frozen.event_cell(EVENT_SYSTEM_TRIGGERED),
        Some((
            symbio_platform::kernel::event::Entity::System,
            symbio_platform::kernel::event::Verb::Opened
        ))
    );
    assert_eq!(
        frozen.event_cell(EVENT_SYSTEM_HEALTH),
        Some((
            symbio_platform::kernel::event::Entity::System,
            symbio_platform::kernel::event::Verb::Progressed
        ))
    );
    assert_eq!(
        frozen.event_owner(EVENT_SYSTEM_TRIGGERED),
        Some(super::OWNER)
    );
    assert_eq!(frozen.event_owner(EVENT_SYSTEM_HEALTH), Some(super::OWNER));
}

/// `conation.expressed` 的 owner **不在本插件**（能力包 `conation`）：本插件只剩写方。
///
/// 这一条钉的是「迁移之后不留两份」：名字的 owner 归能力包，本插件只在生产代码里按
/// 生成物拼写。真实 owner 由能力包自己的用例与 e2e 钉。
#[test]
fn the_intent_event_is_no_longer_owned_here() {
    assert!(
        super::EVENTS
            .iter()
            .all(|e| e.name != symbio_plugin_api::events::EVENT_CONATION_EXPRESSED),
        "本插件的事件表里不再有 conation.expressed（迁移不留两份）"
    );
}
