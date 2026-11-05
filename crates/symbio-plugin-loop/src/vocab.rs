//! 世界事实的词汇持有（**临时**）。
//!
//! 旧引擎删除后，这些对话 / 记忆 / 承诺 / 授权 / 控制面事实暂时没有写方插件；
//! 名字与格位先由本插件继续登记，避免共享词汇表断裂。它们**不是环的机制**：
//! 环不读、不写、不按它们选债。
//!
//! 所有权按接入顺序逐条移交：会话簿记接管 `chat.*` / `user.message` / `turn.branch` /
//! `session.config`，工具接管 `authz.*` / `task.controlled` 与产物格 `artifact.added`，
//! 记忆接管 `memory.*`，代理接管 `commitment.*`。
//! 移交完成后删除本模块与 `PLUGIN.yml` 的 `shared_events`。

use symbio_kernel::decl::{ActorDecl, FactDecl, PluginDeclarations};
use symbio_kernel::event::{Entity, EventDecl, FactRole, Verb};

use symbio_plugin_api::rows::{declared_writes, PLUGIN_ROW, PORT_ACTOR};
use symbio_plugin_api::{EVENT_REQUEST, PRINCIPAL_USER};

pub(crate) const OWNER: &str = "crates/symbio-plugin-loop";

/// 旧引擎留下的世界词汇。**不参与选债 / 派发 / 配对**。
#[rustfmt::skip]
pub(crate) const EVENTS: &[EventDecl] = &[
    EventDecl::with_roles("authz.denied", (Entity::Scope, Verb::Asserted), &[FactRole::StopsTurn]),
    EventDecl::new("authz.granted", (Entity::Scope, Verb::Asserted)),
    EventDecl::new("chat.assistant.fallback", (Entity::Scope, Verb::Closed)),
    EventDecl::host_with_roles(
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
    // 记忆事实归**认知归属人**，不是跑轮主体：召回视图按 `actor == viewer` 过滤，
    // 记忆要按人看见它（D-339）。
    EventDecl::new("memory.consolidated", (Entity::Memory, Verb::Progressed)).owned(PRINCIPAL_USER),
    EventDecl::new("memory.encoded", (Entity::Memory, Verb::Opened)).owned(PRINCIPAL_USER),
    EventDecl::new("memory.forgotten", (Entity::Memory, Verb::Closed)).owned(PRINCIPAL_USER),
    // 机制事实（D-338）：`plugin_row` 执行体在 Ask 后落的「这一行在等」。
    // 行名与提示在载荷里；种类只有一个名字，否则进不了声明表。
    EventDecl::new("plugin.row.asked", (Entity::Scope, Verb::Asserted)),
    EventDecl::new("session.config", (Entity::Scope, Verb::Asserted)),
    EventDecl::new("task.controlled", (Entity::Control, Verb::Opened)),
    EventDecl::new("turn.branch", (Entity::Scope, Verb::Asserted)),
    EventDecl::host_with_roles(
        "user.message",
        (Entity::Scope, Verb::Opened),
        &[FactRole::DeclaresBudget],
    ),
];

/// 旧引擎留下的世界词汇的**载荷形状**。只有被行模式引用的（`chat.assistant.final`
/// 的 `root_id`）才在这里登记；`port` / `key` / `expand` 不属这一族。
pub(crate) const FACTS: &[FactDecl] = &[FactDecl {
    name: "chat.assistant.final",
    port: None,
    key: &[],
    expand: &[],
    fields: &["root_id", "final_id", "text"],
}];

/// 两个**宿主通用执行体**的声明（D-296 / D-303）。
///
/// 它们不属任何插件：`plugin_row` 的实现是各插件 `submit_row!` 的并集，`port` 的实现
/// 是通用受理入口。名字由宿主词汇表定，声明在**装配侧**给出——部分装配（只链一个
/// 插件）没有这两个执行体，也就没有这两条声明，装不出来的 `by` 就是真的缺。
fn actors() -> Vec<ActorDecl> {
    // 写入名单取自实现：`submit_row!` 的并集（`rows.rs` 的 `declared_writes`）。
    // `ActorDecl` 要求 `'static`，而并集是每次装配算出来的 ⇒ 限一次泄漏，之后复用。
    static ROW_WRITES: std::sync::OnceLock<&'static [&'static str]> = std::sync::OnceLock::new();
    let row_writes = ROW_WRITES.get_or_init(|| Box::leak(declared_writes().into_boxed_slice()));
    vec![
        ActorDecl {
            name: PLUGIN_ROW,
            // 反射 / 快速 / 深度三档的插件行都走它（`notes.reflect` 是最宽档）。
            mode: symbio_kernel::LatencyTier::Deep,
            reads: &[],
            writes: row_writes,
        },
        ActorDecl {
            name: PORT_ACTOR,
            // 工具调用是秒–分钟级（`port.tool` 的租约 600s），最宽档。
            mode: symbio_kernel::LatencyTier::Deep,
            reads: &[],
            // 端口执行体可以**开下一笔债**（工具批次把模型应答展开成逐调用 `request`，
            // `port.rs::open_drafts`）——这是 `request` 在声明面上的生产者。
            writes: &[EVENT_REQUEST],
        },
    ]
}

symbio_plugin_api::submit_declarations!(OWNER, submitted_declarations);

fn submitted_declarations() -> PluginDeclarations<'static> {
    PluginDeclarations {
        events: EVENTS,
        facts: FACTS,
        views: &[],
        ports: &[],
        actors: actors(),
        rows: &[],
    }
}
