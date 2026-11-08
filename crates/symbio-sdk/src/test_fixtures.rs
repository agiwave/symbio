//! core 用例的事件拼写与三张词汇表 —— **由 `scripts/facts/test-fixtures-rs.mjs` 生成，请勿手改**。
//!
//! 事实源：各插件 owner 提交的 `EventDecl`（值 / 格位 / 角色 / host）。同一个源还生成
//! `symbio-plugin-api/src/events.rs`（给生产读者）；本产物只服务 core 的测试读者。
//!
//! ## 为什么它在 core 里，却不是 core 的词典
//!
//! D-203 裁决三把业务事件名移出内核：名字只由 owner 声明，core 的折叠经**构造期形参**
//! （[`ChatVocab`] / [`MessageVocab`] / [`CommitmentVocab`]）收下拼写，一行都不读。
//! 生产拼写来自 `symbio_plugin_api::events`；core 不能依赖那个 crate——它反过来依赖 core。
//! 所以夹具的拼写由生成器从**同一批 owner 声明**投影一份，只在 `#[cfg(test)]` /
//! `feature = "test-fixtures"` 下存在。**测试也是调用方**，它交的拼写就是它要断言的语义。
//!
//! **本产物只供 `#[cfg(test)]` / `feature = "test-fixtures"` 使用，不进生产构建。**
//! 生产拼写只有一个出口——`symbio_plugin_api::events`；若把本生成器改成「顺带产出生产
//! 常量」，它就成了第二词汇面，与 D-392 的单一源前提直接冲突。
//!
//! 删掉本模块不影响任何生产代码；它也不参与 CS-005 / CS-006 / CS-007 的判定（那三条
//! 判的是内核名字表与清单的消费方分布）。
//!
//! ## 为什么还要一张 `EVENTS`
//!
//! core 的不变量判据（`is_stop_fact` / `check_all` / `closure_reason_closed`）的词汇面是
//! `&[EventDecl]`——它们问「这条名字扮演什么角色」，答案必须来自一张声明表。这张表由
//! 生成器按 owner 声明逐条投影；协议五格仍取内核常量（它们是语法闭集的一部分）。

use crate::event::{EVENT_FAILED, EVENT_REQUEST, EVENT_RESPONSE, EVENT_RETRY, EVENT_SENT};
use crate::projection::vocab::{ChatVocab, CommitmentVocab, MessageVocab};
use crate::{Entity, EventDecl, FactRole, Verb};

// ── 对话面（`turn`） ──
pub const EVENT_USER_MESSAGE: &str = "user.message";
pub const EVENT_CHAT_ASSISTANT_REPORTED: &str = "chat.assistant.reported";
pub const EVENT_CHAT_ASSISTANT_OPENING: &str = "chat.assistant.opening";
pub const EVENT_CHAT_ASSISTANT_FINAL: &str = "chat.assistant.final";
pub const EVENT_CHAT_ASSISTANT_FALLBACK: &str = "chat.assistant.fallback";
pub const EVENT_CHAT_ASSISTANT_PROGRESS: &str = "chat.assistant.progress";
pub const EVENT_TURN_BRANCH: &str = "turn.branch";
pub const EVENT_TURN_BRANCH_SELECTED: &str = "turn.branch_selected";
pub const EVENT_MODEL_SPEECH_DENIED: &str = "model.speech.denied";

// ── 工具派发面 ──
pub const EVENT_TOOL_REQUESTED: &str = "tool.requested";

// ── 产物与消息面 ──
pub const EVENT_ARTIFACT_ADDED: &str = "artifact.added";
pub const EVENT_MESSAGE_ADDED: &str = "message.added";
pub const EVENT_MESSAGE_EDITED: &str = "message.edited";
pub const EVENT_MESSAGE_REMOVED: &str = "message.removed";
pub const EVENT_SESSION_CONSOLIDATED: &str = "session.consolidated";

// ── 承诺面 ──
pub const EVENT_COMMITMENT_OPENED: &str = "commitment.opened";
pub const EVENT_COMMITMENT_RELEASED: &str = "commitment.released";
pub const EVENT_COMMITMENT_BROKEN: &str = "commitment.broken";
pub const EVENT_COMMITMENT_ASSERTED: &str = "commitment.asserted";

// ── 裁决面 ──
pub const EVENT_VERDICT_ASSERTED: &str = "verdict.asserted";

/// 用例交出去的声明表：格位与角色与各 owner 的 `EventDecl` 逐条相同。
///
/// 三档角色必须有人住（`seal` 的 `Gap::UnmannedRole`）：预算声明在开轮 / 派发，
/// 收束在定稿，停机在被拒。夹具表照抄这三处，否则守卫判据会在「角色没人住」上红。
#[rustfmt::skip]
pub const EVENTS: &[EventDecl] = &[
    // 协议五格：名字与格位是内核语法的一部分，夹具照内核常量走。
    EventDecl::new(EVENT_REQUEST, (Entity::Scope, Verb::Asserted)), // request
    EventDecl::new(EVENT_SENT, (Entity::Scope, Verb::Asserted)), // sent
    EventDecl::new(EVENT_RESPONSE, (Entity::Scope, Verb::Asserted)), // response
    EventDecl::new(EVENT_FAILED, (Entity::Scope, Verb::Asserted)), // failed
    EventDecl::new(EVENT_RETRY, (Entity::Scope, Verb::Asserted)), // retry
    EventDecl::host_with_roles(EVENT_USER_MESSAGE, (Entity::Scope, Verb::Opened), &[FactRole::DeclaresBudget]), // user.message ← session
    EventDecl::new(EVENT_CHAT_ASSISTANT_REPORTED, (Entity::Scope, Verb::Asserted)), // chat.assistant.reported ← session
    EventDecl::new(EVENT_CHAT_ASSISTANT_OPENING, (Entity::Scope, Verb::Asserted)), // chat.assistant.opening ← session
    EventDecl::host_with_roles(EVENT_CHAT_ASSISTANT_FINAL, (Entity::Scope, Verb::Closed), &[FactRole::FinalizesTurn]), // chat.assistant.final ← session
    EventDecl::new(EVENT_CHAT_ASSISTANT_FALLBACK, (Entity::Scope, Verb::Closed)), // chat.assistant.fallback ← session
    EventDecl::new(EVENT_CHAT_ASSISTANT_PROGRESS, (Entity::Scope, Verb::Progressed)), // chat.assistant.progress ← session
    EventDecl::new(EVENT_TURN_BRANCH, (Entity::Scope, Verb::Asserted)), // turn.branch ← session
    EventDecl::new(EVENT_TURN_BRANCH_SELECTED, (Entity::Scope, Verb::Asserted)), // turn.branch_selected ← session
    EventDecl::with_roles(EVENT_MODEL_SPEECH_DENIED, (Entity::Scope, Verb::Asserted), &[FactRole::StopsTurn]), // model.speech.denied ← model
    EventDecl::host(EVENT_TOOL_REQUESTED, (Entity::Scope, Verb::Asserted)), // tool.requested ← model
    EventDecl::host(EVENT_ARTIFACT_ADDED, (Entity::Artifact, Verb::Asserted)), // artifact.added ← tool
    EventDecl::new(EVENT_MESSAGE_ADDED, (Entity::Message, Verb::Opened)), // message.added ← session
    EventDecl::new(EVENT_MESSAGE_EDITED, (Entity::Message, Verb::Asserted)), // message.edited ← session
    EventDecl::new(EVENT_MESSAGE_REMOVED, (Entity::Message, Verb::Closed)), // message.removed ← session
    EventDecl::new(EVENT_SESSION_CONSOLIDATED, (Entity::Message, Verb::Progressed)), // session.consolidated ← session
    EventDecl::new(EVENT_COMMITMENT_OPENED, (Entity::Commitment, Verb::Opened)), // commitment.opened ← agent
    EventDecl::new(EVENT_COMMITMENT_RELEASED, (Entity::Commitment, Verb::Closed)), // commitment.released ← agent
    EventDecl::new(EVENT_COMMITMENT_BROKEN, (Entity::Commitment, Verb::Closed)), // commitment.broken ← agent
    EventDecl::new(EVENT_COMMITMENT_ASSERTED, (Entity::Commitment, Verb::Asserted)), // commitment.asserted ← agent
    EventDecl::new(EVENT_VERDICT_ASSERTED, (Entity::Verdict, Verb::Asserted)), // verdict.asserted ← reputation
];

/// 对话发言面（8 条事件 + 1 个端口名）。
pub const CHAT: ChatVocab = ChatVocab {
    user_message: EVENT_USER_MESSAGE,
    branch: EVENT_TURN_BRANCH,
    branch_selected: EVENT_TURN_BRANCH_SELECTED,
    reported: EVENT_CHAT_ASSISTANT_REPORTED,
    opening: EVENT_CHAT_ASSISTANT_OPENING,
    final_reply: EVENT_CHAT_ASSISTANT_FINAL,
    fallback: EVENT_CHAT_ASSISTANT_FALLBACK,
    progress: EVENT_CHAT_ASSISTANT_PROGRESS,
    model_port: "model",
};

/// 消息变更面（6 条）。
pub const MESSAGE: MessageVocab = MessageVocab {
    tool_requested: EVENT_TOOL_REQUESTED,
    artifact_added: EVENT_ARTIFACT_ADDED,
    message_added: EVENT_MESSAGE_ADDED,
    message_edited: EVENT_MESSAGE_EDITED,
    message_removed: EVENT_MESSAGE_REMOVED,
    consolidated: EVENT_SESSION_CONSOLIDATED,
};

/// 承诺面（4 条）。
pub const COMMITMENT: CommitmentVocab = CommitmentVocab {
    opened: EVENT_COMMITMENT_OPENED,
    released: EVENT_COMMITMENT_RELEASED,
    broken: EVENT_COMMITMENT_BROKEN,
    asserted: EVENT_COMMITMENT_ASSERTED,
};
