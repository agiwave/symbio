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

pub(crate) const OWNER: &str = "crates/symbio-plugin-session";

pub(crate) const EVENTS: &[EventDecl] = &[
    EventDecl::new(EVENT_SYSTEM_TRIGGERED, (Entity::System, Verb::Opened)),
    EventDecl::new(EVENT_SYSTEM_HEALTH, (Entity::System, Verb::Progressed)),
    EventDecl::new(EVENT_CONATION_EXPRESSED, (Entity::Conation, Verb::Opened)),
    // ── 消息列表的变更与收束（DE-024 D-203 归零批）──
    //
    // 五条的写方全在本插件（`message_events` 与
    // `orchestrator::consolidate`），读方是 core 的折叠（`mutations` /
    // `prompt_fidelity` / `transcript` / `rebuild` / `branch`）——两个插件都调那些
    // 折叠 ⇒ owner 是写方（本插件），消费面经清单 `shared_events` 投影。
    //
    // 格位按 D-053 / D-054 的闭集四格登记：开 / 改 / 删 / 巩固。
    EventDecl::new("message.added", (Entity::Message, Verb::Opened)),
    EventDecl::new("message.edited", (Entity::Message, Verb::Asserted)),
    EventDecl::new("message.removed", (Entity::Message, Verb::Closed)),
    EventDecl::new("session.consolidated", (Entity::Message, Verb::Progressed)),
    // 用户点了切换器：一次「关于这一轮的断言」，不是收束也不是开轮。
    EventDecl::new("turn.branch_selected", (Entity::Turn, Verb::Asserted)),
];

/// **消费方**侧的任务族词汇表（DE-024 D-206）：这一族的 owner 是 `crates/symbio-plugin-task`，本插件只是
/// 读 `readyset` / `autonomy` 两列的第二个消费方，所以拼写取自清单生成物
/// `symbio_plugin_api::events`——不是兄弟插件（E-009 禁那条边），也不是 core 或内核代答。
/// 交出整族而不是本插件用到的那两格：折叠收的是同一张表，两张各读各的拼写迟早分岔。
pub(crate) const TASK_VOCAB: symbio_core::TaskVocab<'static> = symbio_core::TaskVocab {
    opened: symbio_plugin_api::events::EVENT_TASK_OPENED,
    progressed: symbio_plugin_api::events::EVENT_TASK_PROGRESS,
    held: symbio_plugin_api::events::EVENT_TASK_HELD,
    terminal: symbio_plugin_api::events::EVENT_TASK_ASSERTED,
    lease: symbio_plugin_api::events::EVENT_TASK_ASSIGNED,
};

/// 对话面 / 消息面 / 承诺面的三张词汇表（DE-024 D-203 归零批）。
///
/// 与 [`TASK_VOCAB`] 同一条通路、同一个理由：折叠（`transcript` / `rebuild` /
/// `prompt_fidelity` / `branch` / `mutations` / `slo` / `commitment_records` …）住在
/// core，两个插件都调它，所以拼写只能来自**清单生成物**——本插件是 `message.*` 与
/// `session.consolidated` / `turn.branch_selected` 五条的 owner，其余十一条的 owner 是
/// `crates/symbio-plugin-loop`（E-009 禁兄弟插件依赖，故不引用 loop 的 `control_decl`）；
/// 任务族五条已随能力迁到 `crates/symbio-plugin-task`——拼写同样只从清单生成物取。
///
/// ## 为什么是三张而不是一张
///
/// 按**面**切开，每张表的大小就是那个折叠算法真正的依赖：`reputation` 只认承诺四条，
/// `changes` 只认产物一条。交一张 20 字段的合并表会把耦合面说宽（DE-024 D-206）。
pub(crate) const CHAT_VOCAB: symbio_core::ChatVocab = symbio_core::ChatVocab {
    user_message: symbio_plugin_api::events::EVENT_USER_MESSAGE,
    dispatch: symbio_plugin_api::events::EVENT_TURN_DISPATCH,
    supplemented: symbio_plugin_api::events::EVENT_TURN_SUPPLEMENTED,
    branch: symbio_plugin_api::events::EVENT_TURN_BRANCH,
    branch_selected: symbio_plugin_api::events::EVENT_TURN_BRANCH_SELECTED,
    reported: symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_REPORTED,
    opening: symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_OPENING,
    final_reply: symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_FINAL,
    fallback: symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_FALLBACK,
    progress: symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_PROGRESS,
    denied: symbio_plugin_api::events::EVENT_AUTHZ_DENIED,
};

/// 消息变更面（5 条）：本插件是这五条里四条的 owner（`message.*` 与
/// `session.consolidated`），`artifact.added` 的 owner 是 `crates/symbio-plugin-loop`。
pub(crate) const MESSAGE_VOCAB: symbio_core::MessageVocab = symbio_core::MessageVocab {
    artifact_added: symbio_plugin_api::events::EVENT_ARTIFACT_ADDED,
    message_added: symbio_plugin_api::events::EVENT_MESSAGE_ADDED,
    message_edited: symbio_plugin_api::events::EVENT_MESSAGE_EDITED,
    message_removed: symbio_plugin_api::events::EVENT_MESSAGE_REMOVED,
    consolidated: symbio_plugin_api::events::EVENT_SESSION_CONSOLIDATED,
};

/// 承诺面（4 条）：`reputation` 与 `commitment_events` 是它仅有的两个消费方。
pub(crate) const COMMITMENT_VOCAB: symbio_core::CommitmentVocab = symbio_core::CommitmentVocab {
    opened: symbio_plugin_api::events::EVENT_COMMITMENT_OPENED,
    released: symbio_plugin_api::events::EVENT_COMMITMENT_RELEASED,
    broken: symbio_plugin_api::events::EVENT_COMMITMENT_BROKEN,
    asserted: symbio_plugin_api::events::EVENT_COMMITMENT_ASSERTED,
};

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
