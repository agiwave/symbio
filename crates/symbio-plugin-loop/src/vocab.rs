//! 世界事实的词汇持有（**临时**）。
//!
//! 旧引擎删除后，这些对话 / 记忆 / 承诺 / 授权 / 工件 / 控制面事实暂时没有写方插件；
//! 名字与格位先由本插件继续登记，避免共享词汇表断裂。它们**不是环的机制**：
//! 环不读、不写、不按它们选债。
//!
//! 所有权按接入顺序逐条移交：会话簿记接管 `chat.*` / `user.message` / `turn.branch` /
//! `session.config`，工具接管 `authz.*` / `task.controlled`，记忆接管 `memory.*`，
//! 代理接管 `commitment.*`，VDFS 接管 `artifact.added`。
//! 移交完成后删除本模块与 `PLUGIN.yml` 的 `shared_events`。

use symbio_kernel::decl::PluginDeclarations;
use symbio_kernel::event::{Entity, EventDecl, FactRole, Verb};

pub(crate) const OWNER: &str = "crates/symbio-plugin-loop";

/// 旧引擎留下的世界词汇。**不参与选债 / 派发 / 配对**。
#[rustfmt::skip]
pub(crate) const EVENTS: &[EventDecl] = &[
    EventDecl::new("artifact.added", (Entity::Artifact, Verb::Asserted)),
    EventDecl::with_roles("authz.denied", (Entity::Scope, Verb::Asserted), &[FactRole::StopsTurn]),
    EventDecl::new("authz.granted", (Entity::Scope, Verb::Asserted)),
    EventDecl::new("chat.assistant.fallback", (Entity::Scope, Verb::Closed)),
    EventDecl::with_roles(
        "chat.assistant.final",
        (Entity::Scope, Verb::Closed),
        &[FactRole::FinalizesTurn],
    ),
    EventDecl::new("chat.assistant.opening", (Entity::Scope, Verb::Asserted)),
    EventDecl::new("chat.assistant.progress", (Entity::Scope, Verb::Progressed)),
    EventDecl::new("chat.assistant.reported", (Entity::Scope, Verb::Asserted)),
    EventDecl::new("commitment.asserted", (Entity::Commitment, Verb::Asserted)),
    EventDecl::new("commitment.broken", (Entity::Commitment, Verb::Closed)),
    EventDecl::new("commitment.opened", (Entity::Commitment, Verb::Opened)),
    EventDecl::new("commitment.released", (Entity::Commitment, Verb::Closed)),
    EventDecl::new("memory.consolidated", (Entity::Memory, Verb::Progressed)),
    EventDecl::new("memory.encoded", (Entity::Memory, Verb::Opened)),
    EventDecl::new("memory.forgotten", (Entity::Memory, Verb::Closed)),
    EventDecl::new("session.config", (Entity::Scope, Verb::Asserted)),
    EventDecl::new("task.controlled", (Entity::Control, Verb::Opened)),
    EventDecl::new("turn.branch", (Entity::Scope, Verb::Asserted)),
    EventDecl::host_with_roles(
        "user.message",
        (Entity::Scope, Verb::Opened),
        &[FactRole::DeclaresBudget],
    ),
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
