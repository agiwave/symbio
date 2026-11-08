//! 会话插件的事件声明（DE-024 D-197 / D-201）。
//!
//! 本插件拥有 `system.*`（定时触发与健康自检）：名字、格位与声明同在这里，写方与读方
//! 都从本模块取名字——内核不再为它们保留常量。
//!
//! **`conation.expressed` 的 owner 是能力包**（`crates/symbio-plugin-conation`，站位见 `D-269`）：
//! 本插件只剩写方（自主触发的落格）。名字因此取自清单生成物
//! `symbio_plugin_api::events`——与任务族同一条通路，不是兄弟插件、也不是内核代答。
//!
//! 事实 / 视图不在本插件的控制面，因此大多不在这里——除本插件自己的收束格
//! `session.settled` / `session.terminated`，以及随 `chat.*` 词汇移交的对话面两条。

use symbio_kernel::decl::{FactDecl, PluginDeclarations};
use symbio_kernel::event::{Entity, EventDecl, FactRole, Verb};
use symbio_plugin_api::ring::{Decl, Pattern};
use symbio_plugin_api::rows::PLUGIN_ROW;
use symbio_plugin_api::LatencyTier;

/// 定时触发：没有用户消息时的自主行为起点。落在 `system × opened`。
/// **触发器产出事件，不是旁路**——自主行为同样走 I1 单通道、I2 带溯源。
pub(crate) const EVENT_SYSTEM_TRIGGERED: &str = "system.triggered";

/// 健康自检。落在 `system × progressed`。
pub(crate) const EVENT_SYSTEM_HEALTH: &str = "system.health";

/// 消息变更面里**只有本插件读写**的一条：写方在 `message_events` / `runtime`，
/// 读方是同一插件调用 core 的折叠。没有第二个插件在生产代码里引用它，
/// 因此不进 `PLUGIN.yml` 的共享表（CS-007：单消费方的名字随 owner 走）。
pub(crate) const EVENT_MESSAGE_EDITED: &str = "message.edited";

/// 对话面里**只有本插件读写**的三条（首响 / 进度 / 过程报告）。
///
/// 写方与读方都在本插件（`opening` / `progress` / `runtime`），没有第二个插件在生产
/// 代码里引用它们 ⇒ 名字随 owner 走，不进 `PLUGIN.yml` 的共享表、不出现在清单生成物里
/// （CS-007）。端到端折叠（`symbio-sdk` 的投影）只吃传入的词汇表，不 import 这些名字。
pub(crate) const EVENT_CHAT_ASSISTANT_OPENING: &str = "chat.assistant.opening";
pub(crate) const EVENT_CHAT_ASSISTANT_PROGRESS: &str = "chat.assistant.progress";
pub(crate) const EVENT_CHAT_ASSISTANT_REPORTED: &str = "chat.assistant.reported";

/// `message.removed` 与 `session.consolidated` 被 `model` / `compact` 按名字读，
/// 因此进 `PLUGIN.yml` 的 `shared_events`；消费方 import 生成常量（D-364）。
pub(crate) use symbio_plugin_api::events::{EVENT_MESSAGE_REMOVED, EVENT_SESSION_CONSOLIDATED};

/// 用户点切换器时的断言；同样只有本插件读写（`commands` 落格、折叠读回）。
pub(crate) const EVENT_TURN_BRANCH_SELECTED: &str = "turn.branch_selected";

/// 收束完成的断言：`session.close-turn` 行在终答之后落它，行据此关账（D-250 / D-252）。
///
/// 载荷只有 `root_id`：与 `chat.assistant.final` 的键**同名字段、同取值**，
/// 债键才能对上（`Key` 按字段名比）。
pub(crate) const EVENT_SESSION_SETTLED: &str = "session.settled";

/// 一个 scope 算完的断言（D-367）：终答已落 **且** 宿主报告 `NoDebt` 时，由本插件的
/// 簿记写方落它。与 `session.settled` 的区别是判据来源：`settled` 只问「收束贡献落完
/// 没有」，`terminated` 问「这一轮还有没有未还的债」——后者由宿主交回的 `exhausted`
/// 给出，写方不自行判环。
pub(crate) const EVENT_SESSION_TERMINATED: &str = "session.terminated";

/// 轮首召回读数（见 `symbio_plugin_api::events::EVENT_SESSION_RECALL`）：本插件是
/// 事实源的 owner，跨会话枚举与可见域收放只能在这里做；读数落一格事实，能力包的
/// provider 按名字读它渲染段（不扩 `PortFactReader`、不把切片塞进开轮载荷）。
pub(crate) const EVENT_SESSION_RECALL: &str = symbio_plugin_api::events::EVENT_SESSION_RECALL;

/// 收束行：终答落格之后，把能力交出的收束贡献（任务调度 / 记忆编码 …）落成事实。
/// `by = plugin_row`，实现见 `close_turn`。
pub(crate) const SESSION_CLOSE_TURN: &str = "session.close-turn";

/// 首响的**归宿行**（`by = None`）：只登记「`chat.assistant.opening` 归本插件看护」，
/// 不参与派发。实际申报在运行时（`OpeningDebts`，开关读当轮 `session.config` 的
/// `opening_enabled`）——行名的字面量见 `opening.rs`。
pub(crate) const SESSION_OPENING_SEAT: &str = "session.opening-seat";

/// 终止事实的**归宿行**（`by = None`）：只给 `session.terminated` 一个声明期读方
/// （`OrphanFact`），不参与派发。实际写方是宿主侧簿记写方（`termination.rs`）。
pub(crate) const SESSION_TERMINATED_SEAT: &str = "session.terminated-seat";

// 读方侧的视图名：名字住 `plugin-api` 的词汇面（由 owner 的 `PLUGIN.yml`
// `shared_views` 生成，D-364），本插件按名字读。以下别名只保留调用点的旧名字，
// 读口不变；owner 改名时生成物与这里一起变，不再出现「各写一遍字面量」。
// `task.readyset` 只交本轮的容量，不是折叠的一格（D-289）。
pub(crate) use symbio_plugin_api::view_names::{
    VIEW_CONATION_AUTONOMY as CONATION_AUTONOMY_VIEW,
    VIEW_CONATION_INTENTS as CONATION_INTENTS_VIEW, VIEW_SLO_FALLBACK as SLO_FALLBACK_VIEW,
    VIEW_SLO_LATENCY as SLO_LATENCY_VIEW, VIEW_SLO_SPAN as SLO_SPAN_VIEW,
    VIEW_TASK_READYSET as TASK_READYSET_VIEW,
};

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
    EventDecl::new(EVENT_TURN_BRANCH_SELECTED, (Entity::Scope, Verb::Asserted)),
    // 用户答复（`kind ∈ answer|approve|deny`）：外部动作落事实，宿主再驱动一环
    // （D-121）。前缀 `user.` 是既有族（与 `user.message` 同前缀）。
    // 若将来出现**非用户**的答复（如超时自动拒绝），这个前缀会不够用——那属策略层，
    // 真出现再议，**现在不预留**。
    EventDecl::new("user.answer", (Entity::Scope, Verb::Asserted)),
    // 收束完成：`session.close-turn` 落完能力贡献之后写它，行据此关账。
    EventDecl::new(EVENT_SESSION_SETTLED, (Entity::Scope, Verb::Progressed)),
    // 一个 scope 算完：终答 ∧ 无未还债（后者由宿主的 `NoDebt` 交回，D-367）。
    EventDecl::new(EVENT_SESSION_TERMINATED, (Entity::Scope, Verb::Closed)),
    // 轮首召回读数：本插件（事实源的 owner）把跨会话可见切片折成的一格读数。
    // 写方是 `consume_inbox_turn`，读方是 `symbio-plugin-memory` 的 `RecallProvider`
    // ——provider 是纯函数拿不到别的事实源，所以读数由宿主落格、能力包按名字读。
    EventDecl::new(EVENT_SESSION_RECALL, (Entity::Scope, Verb::Asserted)),
    // ── 对话面词汇（自 `loop` 的临时词汇表移交）──
    //
    // 写方全在本插件（`runtime` / `close_turn` / `opening` / `progress`），读方是各
    // 消费插件（classify / compact / compose / cost / hook / model / tool …）的折叠与
    // 投影；名字经本插件 `PLUGIN.yml` 的 `shared_events` 投影成生成常量。
    EventDecl::new("chat.assistant.fallback", (Entity::Scope, Verb::Closed)),
    // 终答：`FinalizesTurn` 是「这一轮已经答完」的唯一判据（`is_turn_final`）。
    EventDecl::host_with_roles(
        "chat.assistant.final",
        (Entity::Scope, Verb::Closed),
        &[FactRole::FinalizesTurn],
    ),
    EventDecl::new("chat.assistant.opening", (Entity::Scope, Verb::Asserted)),
    EventDecl::new("chat.assistant.progress", (Entity::Scope, Verb::Progressed)),
    EventDecl::new("chat.assistant.reported", (Entity::Scope, Verb::Asserted)),
    // 用户发言。`tier` 的旧落点仍带 `DeclaresBudget`：新写入的档位只读 `turn.dispatch`，
    // 历史 WAL 里缺 `budget_ms` 时回退到 `tier`，`C5` 仍要读得到预算。
    EventDecl::host_with_roles(
        "user.message",
        (Entity::Scope, Verb::Opened),
        &[FactRole::DeclaresBudget],
    ),
];

/// 收束行（D-250 / D-252）：终答落格之后，把能力交出的收束贡献落成事实。
///
/// `owes` 与 `until` 同键（`root_id`）：一有终答就欠一笔，落 `session.settled` 即还清。
/// 没有这一行，`task.ready` / `task.assigned`（就绪视图的输入）永远不会在收束时落格。
pub(crate) const ROWS: &[Decl] = &[
    Decl {
        name: SESSION_CLOSE_TURN,
        by: Some(PLUGIN_ROW),
        tier: LatencyTier::Fast,
        expand: None,
        owes: Pattern {
            source: symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_FINAL,
            key: &["root_id"],
            eq: &[],
        },
        until: &[Pattern {
            source: EVENT_SESSION_SETTLED,
            key: &["root_id"],
            eq: &[],
        }],
        unless: &[],
    },
    // 首响格的归宿（`by = None`，不派发）：`owes` / `until` 同键、同源，一有首响就
    // 「既欠又还」，只为在装载期给这条事实一个读方（`OrphanFact` 的归宿行）。
    Decl {
        name: SESSION_OPENING_SEAT,
        by: None,
        tier: LatencyTier::Reflex,
        expand: None,
        owes: Pattern {
            source: EVENT_CHAT_ASSISTANT_OPENING,
            key: &["id"],
            eq: &[],
        },
        until: &[Pattern {
            source: EVENT_CHAT_ASSISTANT_OPENING,
            key: &["id"],
            eq: &[],
        }],
        unless: &[],
    },
    // 终止格的归宿（`by = None`，不派发）：与首响同一手法。写方是宿主侧簿记写方，
    // 不在债表里；这里只负责让装载期查得到「谁看护这条事实」。
    Decl {
        name: SESSION_TERMINATED_SEAT,
        by: None,
        tier: LatencyTier::Reflex,
        expand: None,
        owes: Pattern {
            source: EVENT_SESSION_TERMINATED,
            key: &["root_id"],
            eq: &[],
        },
        until: &[Pattern {
            source: EVENT_SESSION_TERMINATED,
            key: &["root_id"],
            eq: &[],
        }],
        unless: &[],
    },
];

/// 本插件声明的事实形状：`session.*` 两条，以及随 `chat.*` 词汇移交过来的对话面两条。
///
/// `chat.assistant.reported` 不登记形状：没有行以它为前提或出口；格位仍在本文件的
/// `EVENTS` 里，登记成事实只会多出一条 `OrphanFact` 需要归宿行，而不是多一条检查。
pub(crate) const FACTS: &[FactDecl] = &[
    FactDecl {
        name: EVENT_SESSION_SETTLED,
        port: None,
        key: &[],
        expand: &[],
        fields: &["root_id"],
    },
    FactDecl {
        name: EVENT_SESSION_TERMINATED,
        port: None,
        key: &[],
        expand: &[],
        fields: &["root_id", "reason"],
    },
    FactDecl {
        name: "chat.assistant.final",
        port: None,
        key: &[],
        expand: &[],
        fields: &["root_id", "final_id", "text"],
    },
    // 首响格：内容由 compose 的模板产出、由会话的行落格（D-344 的「跨插件只走 route」）。
    // 它是一轮里**先于**终答/工具轮出现的对话面事实，排除出上下文（旧的 `escalate`
    // 断言靠它把「我先看一下」与真正的答话分开）。
    FactDecl {
        name: "chat.assistant.opening",
        port: None,
        key: &[],
        expand: &[],
        fields: &["id", "reason", "text", "exclude_from_context"],
    },
];

/// 对话面 / 消息面 / 承诺面的三张词汇表（DE-024 D-203 归零批）。
///
/// 与 [`CONATION_INTENTS_VIEW`] 同一条通路、同一个理由：折叠（`transcript` / `rebuild` /
/// `prompt_fidelity` / `branch` / `mutations` / `slo` / `commitment_records` …）住在
/// core，两个插件都调它，所以拼写只能来自**清单生成物**——本插件是 `message.*`、
/// `session.consolidated` / `turn.branch_selected` 与 `chat.*` / `user.message`
/// 的 owner；`authz.*` 与 `commitment.*` 的 owner 仍是 `crates/symbio-plugin-loop`
/// （E-009 禁兄弟插件依赖，故不引用 loop 的 `vocab`）；任务族五条已随能力迁到
/// `crates/symbio-plugin-task`——拼写同样只从清单生成物取。
///
/// `model_port` 是**端口名**，不属事件清单：它从共享词汇面
/// `symbio_plugin_api::vocab::PORT_MODEL` 取，折叠据此认「本轮自己的模型往返」。
///
/// ## 为什么是三张而不是一张
///
/// 按**面**切开，每张表的大小就是那个折叠算法真正的依赖：`commitment_records` 只认承诺
/// 四条，`changes` 只认产物一条。交一张 20 字段的合并表会把耦合面说宽（DE-024 D-206）。
pub(crate) const CHAT_VOCAB: symbio_sdk::projection::vocab::ChatVocab =
    symbio_sdk::projection::vocab::ChatVocab {
        user_message: symbio_plugin_api::events::EVENT_USER_MESSAGE,
        branch: symbio_plugin_api::events::EVENT_TURN_BRANCH,
        branch_selected: EVENT_TURN_BRANCH_SELECTED,
        reported: EVENT_CHAT_ASSISTANT_REPORTED,
        opening: EVENT_CHAT_ASSISTANT_OPENING,
        final_reply: symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_FINAL,
        fallback: symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_FALLBACK,
        progress: EVENT_CHAT_ASSISTANT_PROGRESS,
        denied: symbio_plugin_api::events::EVENT_AUTHZ_DENIED,
        model_port: symbio_plugin_api::vocab::PORT_MODEL,
    };

/// 消息变更面（5 条）：本插件是这五条里四条的 owner（`message.*` 与
/// `session.consolidated`），`artifact.added` 的 owner 是 `crates/symbio-plugin-tool`。
pub(crate) const MESSAGE_VOCAB: symbio_sdk::projection::vocab::MessageVocab =
    symbio_sdk::projection::vocab::MessageVocab {
        artifact_added: symbio_plugin_api::events::EVENT_ARTIFACT_ADDED,
        message_added: symbio_plugin_api::events::EVENT_MESSAGE_ADDED,
        message_edited: EVENT_MESSAGE_EDITED,
        message_removed: EVENT_MESSAGE_REMOVED,
        consolidated: EVENT_SESSION_CONSOLIDATED,
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

fn submitted_declarations() -> PluginDeclarations<'static> {
    PluginDeclarations {
        events: EVENTS,
        facts: FACTS,
        views: &[],
        ports: &[],
        actors: Vec::new(),
        rows: ROWS,
    }
}

#[cfg(test)]
#[path = "declarations.test.rs"]
mod tests;
