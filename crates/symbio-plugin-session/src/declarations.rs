//! 会话插件的事件声明（DE-024 D-197 / D-201）。
//!
//! 本插件拥有 `system.*`（定时触发与健康自检）：名字、格位与声明同在这里，写方与读方
//! 都从本模块取名字——内核不再为它们保留常量。
//!
//! **`conation.expressed` 的 owner 是能力包**（`crates/symbio-plugin-conation`，站位见 `D-269`）：
//! 本插件只剩写方（自主触发的落格）。名字因此取自清单生成物
//! `symbio_plugin_api::events`——与任务族同一条通路，不是兄弟插件、也不是内核代答。
//!
//! 事实 / 视图 / 行都不在这里：它们不属于本插件的控制面；**端口声明**例外——
//! 本插件是 `port.tool` 的世界提供者，端口名与租约参数随本文件提交（DE-047 不变量 6）。

use symbio_kernel::decl::ring::PortDecl;
use symbio_kernel::decl::PluginDeclarations;
use symbio_kernel::event::{Entity, EventDecl, Verb, EVENT_SENT};
use symbio_plugin_api::port::ports;

/// 定时触发：没有用户消息时的自主行为起点。落在 `system × opened`。
/// **触发器产出事件，不是旁路**——自主行为同样走 I1 单通道、I2 带溯源。
pub(crate) const EVENT_SYSTEM_TRIGGERED: &str = "system.triggered";
/// 健康自检。落在 `system × progressed`。
pub(crate) const EVENT_SYSTEM_HEALTH: &str = "system.health";

/// **读方侧**的视图名：主动性读数由能力包交（`crates/symbio-plugin-conation`），本插件按
/// 名字读。与 [`TASK_READYSET_VIEW`] 同一条通路——拼写只有一个来源（owner 的 `PLUGIN.yml`
/// `shared_views`），本常量只是让三个读点（`stats` / `stats_reading` / 自主触发）不各写
/// 一遍字面量。改名由 owner 决定，改完这里跟着改，读口不变。
pub(crate) const CONATION_INTENTS_VIEW: &str = "conation.intents";

/// 自主程度轴的名字：折叠由同一个能力包交（`conation.autonomy`），本插件按名字读。
///
/// 与 [`CONATION_INTENTS_VIEW`] 同一条通路：owner 的 `PLUGIN.yml` `shared_views` 是
/// 唯一来源，读点（`stats` 列 / `stats_reading` 实例级聚合）不各写一遍字面量。
pub(crate) const CONATION_AUTONOMY_VIEW: &str = "conation.autonomy";

/// 时延分层三列的名字：三条视图都由能力包 `crates/symbio-plugin-slo` 交，本插件按名字读。
///
/// 与 [`CONATION_INTENTS_VIEW`] 同一条纪律：拼写只有一个来源（owner 的 `PLUGIN.yml`
/// `shared_views`），这三个常量只是让读点不各写一遍字面量。
pub(crate) const SLO_LATENCY_VIEW: &str = "slo.latency";
pub(crate) const SLO_FALLBACK_VIEW: &str = "slo.fallback";
pub(crate) const SLO_SPAN_VIEW: &str = "slo.span";

/// 成本台账列的名字：折叠由能力包 `crates/symbio-plugin-cost` 交，本插件按名字读。
///
/// 与上面三条同一条纪律。它同时也是 `loop` 那侧熔断闸门读的名字——两个读者一个 owner，
/// 「闸门说还有余量、出口说已耗尽」这类漂移就没有立足点（D-044）。
pub(crate) const COST_LEDGER_VIEW: &str = "cost.ledger";

/// 校准列的名字：账由技能能力包 `crates/symbio-plugin-skill` 折（`memory.calibrated`
/// 的 owner 就是它），本插件按名字读。
///
/// 与上面几条同一条纪律：拼写只有一个来源（owner 的 `PLUGIN.yml` `shared_views`），
/// `confidence = 1 − 回退率` 的算式也不在本插件重算一遍。
pub(crate) const SKILL_CALIBRATION_VIEW: &str = "skill.calibration";

/// 声誉台账列的名字：账由声誉能力包 `crates/symbio-plugin-reputation` 折（承诺三格的
/// 读法住在那一侧），本插件按名字读。
///
/// 与上面几条同一条纪律：拼写只有一个来源（owner 的 `PLUGIN.yml` `shared_views`），
/// 「守约 − 违约」的算式也不在本插件重算一遍。它同时也是 `loop` 那侧淘汰回路读的名字
/// ——两个读者一个 owner（D-044）。
pub(crate) const REPUTATION_LEDGER_VIEW: &str = "reputation.ledger";

/// 就绪集列的名字：折叠由任务能力包 `crates/symbio-plugin-task` 交（`task.*` 五格的
/// owner 与调度者就是它），本插件按名字读。
///
/// 与上面几条同一条纪律：拼写只有一个来源（owner 的 `PLUGIN.yml` `shared_views`），
/// 「谁就绪、什么序」的算式也不在本插件重算一遍。本插件只交**本轮的容量**——那是读者
/// 的预算，不是折叠的一格（`D-289`）。
pub(crate) const TASK_READYSET_VIEW: &str = "task.readyset";

pub(crate) const OWNER: &str = "crates/symbio-plugin-session";

pub(crate) const EVENTS: &[EventDecl] = &[
    EventDecl::new(EVENT_SYSTEM_TRIGGERED, (Entity::System, Verb::Opened)),
    EventDecl::new(EVENT_SYSTEM_HEALTH, (Entity::System, Verb::Progressed)),
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

/// 对话面 / 消息面 / 承诺面的三张词汇表（DE-024 D-203 归零批）。
///
/// 与 [`CONATION_INTENTS_VIEW`] 同一条通路、同一个理由：折叠（`transcript` / `rebuild` /
/// `prompt_fidelity` / `branch` / `mutations` / `slo` / `commitment_records` …）住在
/// core，两个插件都调它，所以拼写只能来自**清单生成物**——本插件是 `message.*` 与
/// `session.consolidated` / `turn.branch_selected` 五条的 owner，其余十一条的 owner 是
/// `crates/symbio-plugin-loop`（E-009 禁兄弟插件依赖，故不引用 loop 的 `control_decl`）；
/// 任务族五条已随能力迁到 `crates/symbio-plugin-task`——拼写同样只从清单生成物取。
///
/// ## 为什么是三张而不是一张
///
/// 按**面**切开，每张表的大小就是那个折叠算法真正的依赖：`commitment_records` 只认承诺
/// 四条，`changes` 只认产物一条。交一张 20 字段的合并表会把耦合面说宽（DE-024 D-206）。
pub(crate) const CHAT_VOCAB: symbio_sdk::projection::vocab::ChatVocab =
    symbio_sdk::projection::vocab::ChatVocab {
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
pub(crate) const MESSAGE_VOCAB: symbio_sdk::projection::vocab::MessageVocab =
    symbio_sdk::projection::vocab::MessageVocab {
        artifact_added: symbio_plugin_api::events::EVENT_ARTIFACT_ADDED,
        message_added: symbio_plugin_api::events::EVENT_MESSAGE_ADDED,
        message_edited: symbio_plugin_api::events::EVENT_MESSAGE_EDITED,
        message_removed: symbio_plugin_api::events::EVENT_MESSAGE_REMOVED,
        consolidated: symbio_plugin_api::events::EVENT_SESSION_CONSOLIDATED,
    };

/// 承诺面（4 条）：本插件只剩 `commitment_records`（承诺列）一个消费方——声誉列改按
/// 名字读台账后，那条折叠搬进了声誉能力包，它的词汇面也跟着走（`D-288`）。
pub(crate) const COMMITMENT_VOCAB: symbio_sdk::projection::vocab::CommitmentVocab =
    symbio_sdk::projection::vocab::CommitmentVocab {
        opened: symbio_plugin_api::events::EVENT_COMMITMENT_OPENED,
        released: symbio_plugin_api::events::EVENT_COMMITMENT_RELEASED,
        broken: symbio_plugin_api::events::EVENT_COMMITMENT_BROKEN,
        asserted: symbio_plugin_api::events::EVENT_COMMITMENT_ASSERTED,
    };

// 装配口的提交：宿主链接本 crate 即生效。装配方按 owner 排序后逐个 `declare`，
// 本插件不认识也不依赖任何兄弟插件。
symbio_plugin_api::submit_declarations!(OWNER, submitted_declarations);

/// 本插件服务的端口：工具批次里的**单次调用**。
///
/// 上限是端口级的天花板，不是「每个调用都会重试」的承诺（D-138）：普通批次调用
/// 的等待方随草稿消失（`awaited = false`），只有唤醒调用能从事实重建等待方。
pub(crate) static PORTS: &[PortDecl] = &[PortDecl {
    port: ports::TOOL,
    before: EVENT_SENT,
    lease_ms: 600_000,
    max_attempts: 2,
    // 一次模型响应可带 N 个工具调用：同批至多四条在飞，具体哪几条仍由风险档决定。
    concurrency: 4,
}];

fn submitted_declarations() -> PluginDeclarations<'static> {
    PluginDeclarations {
        events: EVENTS,
        facts: &[],
        views: &[],
        ports: PORTS,
        actors: Vec::new(),
        rows: &[],
    }
}

#[cfg(test)]
#[path = "declarations.test.rs"]
mod tests;
