//! 会话插件的事件声明（DE-024 D-197 / D-201）。
//!
//! 本插件拥有 `system.*`（定时触发与健康自检）与 `conation.expressed`（欲的表达）：
//! 名字、格位与声明同在这里，写方与读方都从本模块取名字——内核不再为它们保留常量。
//!
//! 事实 / 视图 / 端口 / 行都不在这里：它们不属于本插件的控制面。

use symbio_core::{Entity, EventDecl, PluginDeclarations, Verb};

/// 定时触发：没有用户消息时的自主行为起点。落在 `system × opened`。
/// **触发器产出事件，不是旁路**——自主行为同样走 I1 单通道、I2 带溯源。
pub(crate) const EVENT_SYSTEM_TRIGGERED: &str = "system.triggered";
/// 健康自检。落在 `system × progressed`。
pub(crate) const EVENT_SYSTEM_HEALTH: &str = "system.health";
/// 「欲」的表达。落在 `conation × opened`——写方与读方（`conation` 投影）都在本插件。
pub(crate) const EVENT_CONATION_EXPRESSED: &str = "conation.expressed";

pub(crate) const OWNER: &str = "plugins/session";

pub(crate) const EVENTS: &[EventDecl] = &[
    EventDecl {
        name: EVENT_SYSTEM_TRIGGERED,
        cell: (Entity::System, Verb::Opened),
    },
    EventDecl {
        name: EVENT_SYSTEM_HEALTH,
        cell: (Entity::System, Verb::Progressed),
    },
    EventDecl {
        name: EVENT_CONATION_EXPRESSED,
        cell: (Entity::Conation, Verb::Opened),
    },
];

// 装配口的提交：宿主链接本 crate 即生效。装配方按 owner 排序后逐个 `declare`，
// 本插件不认识也不依赖任何兄弟插件。
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
#[path = "declarations.test.rs"]
mod tests;
