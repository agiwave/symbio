//! core 用例自己交的**事件拼写与三张词汇表**（DE-024 D-203 / D-205 / D-206）。
//!
//! ## 为什么它在 core 里，却不是 core 的词典
//!
//! D-203 裁决三把 21 条业务事件名移出内核：名字从此只由 owner 声明，core 的折叠经
//! **构造期形参**（[`ChatVocab`] / [`MessageVocab`] / [`CommitmentVocab`]）收下拼写，
//! 一行都不读。
//!
//! 生产拼写来自各 owner 的清单生成物 `symbio_plugin_api::events`（D-204）。core 不能
//! 依赖那个 crate——它反过来依赖 core——所以本模块把同一批拼写**抄一份字面量**，只在
//! `#[cfg(test)]` 下存在。这正是「调用方交拼写」这条纪律在测试侧的落地：**测试也是
//! 调用方**，它交的拼写就是它要断言的语义，core 只是照着折叠。
//!
//! 删掉本模块不影响任何生产代码；它也不参与 CS-005 / CS-006 / CS-007 的判定（那三条
//! 判的是内核名字表与清单的消费方分布）。
//!
//! ## 为什么还要一张 `EVENTS`
//!
//! core 的不变量判据（`is_turn_final` / `is_stop_fact` / `check_all` / `closure_reason_closed`）
//! 的词汇面是 `&[EventDecl]`——它们问「这条名字扮演什么角色」，答案必须来自一张声明表。
//! 这张表由用例自己交：格位与角色与各 owner 的 `EventDecl` 逐条相同；协议五格仍取
//! 内核常量（它们是语法闭集的一部分）。

use crate::event::{EVENT_FAILED, EVENT_REQUEST, EVENT_RESPONSE, EVENT_RETRY, EVENT_SENT};
use crate::{ChatVocab, CommitmentVocab, Entity, EventDecl, FactRole, MessageVocab, Verb};

// ── 对话面（`turn`）──
pub(crate) const EVENT_USER_MESSAGE: &str = "user.message";
pub(crate) const EVENT_TURN_DISPATCH: &str = "turn.dispatch";
pub(crate) const EVENT_TURN_SUPPLEMENTED: &str = "turn.supplemented";
pub(crate) const EVENT_CHAT_ASSISTANT_REPORTED: &str = "chat.assistant.reported";
pub(crate) const EVENT_CHAT_ASSISTANT_OPENING: &str = "chat.assistant.opening";
pub(crate) const EVENT_CHAT_ASSISTANT_FINAL: &str = "chat.assistant.final";
pub(crate) const EVENT_CHAT_ASSISTANT_FALLBACK: &str = "chat.assistant.fallback";
pub(crate) const EVENT_CHAT_ASSISTANT_PROGRESS: &str = "chat.assistant.progress";
pub(crate) const EVENT_TURN_BRANCH: &str = "turn.branch";
pub(crate) const EVENT_TURN_BRANCH_SELECTED: &str = "turn.branch_selected";
pub(crate) const EVENT_AUTHZ_DENIED: &str = "authz.denied";

// ── 产物与消息面 ──
pub(crate) const EVENT_ARTIFACT_ADDED: &str = "artifact.added";
pub(crate) const EVENT_MESSAGE_ADDED: &str = "message.added";
pub(crate) const EVENT_MESSAGE_EDITED: &str = "message.edited";
pub(crate) const EVENT_MESSAGE_REMOVED: &str = "message.removed";
pub(crate) const EVENT_SESSION_CONSOLIDATED: &str = "session.consolidated";

// ── 承诺面 ──
pub(crate) const EVENT_COMMITMENT_OPENED: &str = "commitment.opened";
pub(crate) const EVENT_COMMITMENT_RELEASED: &str = "commitment.released";
pub(crate) const EVENT_COMMITMENT_BROKEN: &str = "commitment.broken";
pub(crate) const EVENT_COMMITMENT_ASSERTED: &str = "commitment.asserted";

// ── 裁决面 ──
pub(crate) const EVENT_VERDICT_ASSERTED: &str = "verdict.asserted";

/// 用例交出去的声明表：格位与角色与各 owner 的 `EventDecl` 逐条相同。
///
/// 三档角色必须有人住（`seal` 的 `Gap::UnmannedRole`）：预算声明在开轮 / 派发，
/// 收束在定稿，停机在被拒。用例表照抄这三处，否则守卫判据会在「角色没人住」上红。
pub(crate) const EVENTS: &[EventDecl] = &[
    EventDecl::with_roles(
        EVENT_USER_MESSAGE,
        (Entity::Turn, Verb::Opened),
        &[FactRole::DeclaresBudget],
    ),
    EventDecl::with_roles(
        EVENT_TURN_DISPATCH,
        (Entity::Turn, Verb::Asserted),
        &[FactRole::DeclaresBudget],
    ),
    EventDecl::new(EVENT_TURN_SUPPLEMENTED, (Entity::Turn, Verb::Asserted)),
    EventDecl::new(
        EVENT_CHAT_ASSISTANT_REPORTED,
        (Entity::Turn, Verb::Asserted),
    ),
    EventDecl::new(EVENT_CHAT_ASSISTANT_OPENING, (Entity::Turn, Verb::Asserted)),
    EventDecl::with_roles(
        EVENT_CHAT_ASSISTANT_FINAL,
        (Entity::Turn, Verb::Closed),
        &[FactRole::FinalizesTurn],
    ),
    EventDecl::new(EVENT_CHAT_ASSISTANT_FALLBACK, (Entity::Turn, Verb::Closed)),
    EventDecl::new(
        EVENT_CHAT_ASSISTANT_PROGRESS,
        (Entity::Turn, Verb::Progressed),
    ),
    EventDecl::new(EVENT_TURN_BRANCH, (Entity::Turn, Verb::Asserted)),
    EventDecl::new(EVENT_TURN_BRANCH_SELECTED, (Entity::Turn, Verb::Asserted)),
    // 协议五格：名字与格位是内核语法的一部分，用例照内核常量走。
    EventDecl::new(EVENT_REQUEST, (Entity::Turn, Verb::Asserted)),
    EventDecl::new(EVENT_SENT, (Entity::Turn, Verb::Asserted)),
    EventDecl::new(EVENT_RESPONSE, (Entity::Turn, Verb::Asserted)),
    EventDecl::new(EVENT_FAILED, (Entity::Turn, Verb::Asserted)),
    EventDecl::new(EVENT_RETRY, (Entity::Turn, Verb::Asserted)),
    EventDecl::with_roles(
        EVENT_AUTHZ_DENIED,
        (Entity::Turn, Verb::Asserted),
        &[FactRole::StopsTurn],
    ),
    EventDecl::new(EVENT_ARTIFACT_ADDED, (Entity::Artifact, Verb::Asserted)),
    EventDecl::new(EVENT_MESSAGE_ADDED, (Entity::Message, Verb::Opened)),
    EventDecl::new(EVENT_MESSAGE_EDITED, (Entity::Message, Verb::Asserted)),
    EventDecl::new(EVENT_MESSAGE_REMOVED, (Entity::Message, Verb::Closed)),
    EventDecl::new(
        EVENT_SESSION_CONSOLIDATED,
        (Entity::Message, Verb::Progressed),
    ),
    EventDecl::new(EVENT_COMMITMENT_OPENED, (Entity::Commitment, Verb::Opened)),
    EventDecl::new(
        EVENT_COMMITMENT_RELEASED,
        (Entity::Commitment, Verb::Closed),
    ),
    EventDecl::new(EVENT_COMMITMENT_BROKEN, (Entity::Commitment, Verb::Closed)),
    EventDecl::new(
        EVENT_COMMITMENT_ASSERTED,
        (Entity::Commitment, Verb::Asserted),
    ),
    EventDecl::new(EVENT_VERDICT_ASSERTED, (Entity::Verdict, Verb::Asserted)),
];

/// 对话发言面（11 条）。
pub(crate) const CHAT: ChatVocab = ChatVocab {
    user_message: EVENT_USER_MESSAGE,
    dispatch: EVENT_TURN_DISPATCH,
    supplemented: EVENT_TURN_SUPPLEMENTED,
    branch: EVENT_TURN_BRANCH,
    branch_selected: EVENT_TURN_BRANCH_SELECTED,
    reported: EVENT_CHAT_ASSISTANT_REPORTED,
    opening: EVENT_CHAT_ASSISTANT_OPENING,
    final_reply: EVENT_CHAT_ASSISTANT_FINAL,
    fallback: EVENT_CHAT_ASSISTANT_FALLBACK,
    progress: EVENT_CHAT_ASSISTANT_PROGRESS,
    denied: EVENT_AUTHZ_DENIED,
};

/// 消息变更面（5 条）。
pub(crate) const MESSAGE: MessageVocab = MessageVocab {
    artifact_added: EVENT_ARTIFACT_ADDED,
    message_added: EVENT_MESSAGE_ADDED,
    message_edited: EVENT_MESSAGE_EDITED,
    message_removed: EVENT_MESSAGE_REMOVED,
    consolidated: EVENT_SESSION_CONSOLIDATED,
};

/// 承诺面（4 条）。
pub(crate) const COMMITMENT: CommitmentVocab = CommitmentVocab {
    opened: EVENT_COMMITMENT_OPENED,
    released: EVENT_COMMITMENT_RELEASED,
    broken: EVENT_COMMITMENT_BROKEN,
    asserted: EVENT_COMMITMENT_ASSERTED,
};
