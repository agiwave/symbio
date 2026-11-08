//! 承诺面事件名与格位的 owner（D-389 批二）。
//!
//! 写方在本插件的 `host::behavior`（`agent_run` 的委派以 `commitment_events` 成格）；
//! 所有者原先是环（`crates/symbio-plugin-loop`），随「owner 跟着写方走」移交到这里。
//! 四个名字都进 `PLUGIN.yml` 的 `shared_events`：会话 / 声誉按名字读回承诺列，
//! 消费方 import 清单生成物（D-364），不引用本插件。

use symbio_plugin_api::{Entity, EventDecl, PluginDeclarations, Verb};

pub(crate) const OWNER: &str = "crates/symbio-plugin-agent";

/// 承诺面四格：立约 / 履约 / 违约 / 对等宣告。
///
/// 格位与角色逐条照旧（迁移只改 owner，不改格子）：立约开、履约与违约收、
/// 宣告是一次断言。名字只在本文件出现一份，写方经 `symbio_plugin_api::events`
/// 的生成常量取用。
#[rustfmt::skip]
pub(crate) const EVENTS: &[EventDecl] = &[
    EventDecl::new("commitment.asserted", (Entity::Commitment, Verb::Asserted)),
    EventDecl::new("commitment.broken", (Entity::Commitment, Verb::Closed)),
    EventDecl::new("commitment.opened", (Entity::Commitment, Verb::Opened)),
    EventDecl::new("commitment.released", (Entity::Commitment, Verb::Closed)),
];

symbio_plugin_api::submit_declarations!(OWNER, submitted_declarations);

fn submitted_declarations() -> PluginDeclarations<'static> {
    PluginDeclarations {
        events: EVENTS,
        facts: &[],
        views: &[],
        ports: &[],
        actors: Vec::new(),
        rows: &[],
    }
}
