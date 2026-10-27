//! 本插件拥有的事件名与格位（DE-024 D-197 / DE-025 D-209）。
//!
//! `notes.created` 是本插件**唯一**的事实名：名字、格位与声明同在这里，写方
//! （[`super::behavior`]）与读方（[`super::context`]）都从本模块取，不存在第二份拼写。
//!
//! 格位是 `Domain × Opened`——`Domain` 是内核为「新领域」留的扩展位（D-209）；
//! `core` 对这条事实**不解释**，写闸只按本声明校验名字与格位（放行靠声明，理解靠 owner）。
//!
//! 事实 / 视图 / 端口 / 行都不在这里：本插件没有要参与义务环的事实，也没有控制面行。

use symbio_core::{Entity, EventDecl, PluginDeclarations, Verb};

/// 「记下一条便签」这一事实。落在 `domain × opened`。
pub(crate) const NOTES_CREATED: &str = "notes.created";

/// owner —— 与目录名同形（`crates/symbio-plugin-notes`），装配表按它排序与报缺口。
pub(crate) const OWNER: &str = "plugins/notes";

pub(crate) const EVENTS: &[EventDecl] = &[EventDecl::new(
    NOTES_CREATED,
    (Entity::Domain, Verb::Opened),
)];

// 装配口的提交：宿主链接本 crate 即生效。本插件不认识也不依赖任何兄弟插件。
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

#[cfg(test)]
#[path = "decl.test.rs"]
mod tests;
