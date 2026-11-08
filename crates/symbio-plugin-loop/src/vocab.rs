//! 世界事实的词汇持有（**临时**）。
//!
//! 旧引擎删除后，这些控制面事实暂时没有写方插件；名字与格位先由本插件继续登记，
//! 避免共享词汇表断裂。它们**不是环的机制**：环不读、不写、不按它们选债。
//!
//! 所有权按接入顺序逐条移交，**已完成**：会话簿记接管 `chat.*` / `user.message`
//! 与 `session.config` / `turn.branch` / `turn.dispatch`（`session::declarations`），
//! 记忆接管 `memory.*`（`memory::decl`），工具接管 `authz.granted` / `authz.denied`
//! / `task.controlled`（`tool::declarations`），代理接管 `commitment.*`
//! （`agent::declarations`）。本模块如今只剩两条**机制私有名**（`plugin.row.asked`
//! / `driver.stalled`）——名字的唯一定义处仍在 `plugin-api`，这里只登记格位。

use symbio_kernel::decl::{ActorDecl, PluginDeclarations};
use symbio_kernel::event::{Entity, EventDecl, Verb};

use symbio_plugin_api::drive::EVENT_DRIVER_STALLED;
use symbio_plugin_api::rows::{declared_writes, ASK_KIND, PLUGIN_ROW, PORT_ACTOR};
use symbio_plugin_api::EVENT_REQUEST;

pub(crate) const OWNER: &str = "crates/symbio-plugin-loop";

/// 旧引擎留下的机制私有名。**不参与选债 / 派发 / 配对**。
#[rustfmt::skip]
pub(crate) const EVENTS: &[EventDecl] = &[
    // 机制事实（D-338）：`plugin_row` 执行体在 Ask 后落的「这一行在等」。
    // 行名与提示在载荷里；种类只有一个名字，否则进不了声明表。
    EventDecl::new(ASK_KIND, (Entity::Scope, Verb::Asserted)),
    // 驱动安全阀出口（`D-393`）：`Exhausted` / `Stalled` 共用一种形状。
    // 写方是驱动入口（宿主装配层），不是插件；标 `host` 免得被当成「无人可写」的债。
    EventDecl::host(EVENT_DRIVER_STALLED, (Entity::Scope, Verb::Asserted)),
];

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
        facts: &[],
        views: &[],
        ports: &[],
        actors: actors(),
        rows: &[],
    }
}
