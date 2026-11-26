//! 装配级 sealed declarations：通用执行体与端口出口行。
//!
//! 这些声明不是任何能力插件的私有词汇：`plugin_row` / `port` 两个宿主通用执行体，
//! 以及端口债的 `await-port` 出口行，必须在只链接部分插件的装配里也成立。因此它们
//! 由平台提交，随 `assemble_declarations` 一起冻结；编排提供者搬出插件树后不再携带
//! 声明，避免出现“有人执行、没人声明”的缺口。
//!
//! 这里只登记格位，不实现环：实现仍由编排 provider 经 `DriveRunner` 注入。

use crate::kernel::decl::ring::{Decl, Pattern};
use crate::kernel::decl::{ActorDecl, PluginDeclarations};
use crate::kernel::event::{
    Entity, EventDecl, Verb, EVENT_FAILED, EVENT_REQUEST, EVENT_RESPONSE, EVENT_RETRY, EVENT_SENT,
};
use crate::sdk::drive::EVENT_DRIVER_STALLED;
use crate::sdk::plugin::rows::{declared_writes, ASK_KIND, PLUGIN_ROW, PORT_ACTOR};

const OWNER: &str = "platform/sealed-declarations";

/// 两条机制私有名：**不参与选债 / 派发 / 配对**。
#[rustfmt::skip]
const EVENTS: &[EventDecl] = &[
    // `plugin_row` 执行体在 Ask 后落的「这一行在等」；行名与提示在载荷里。
    EventDecl::new(ASK_KIND, (Entity::Scope, Verb::Asserted)),
    // 驱动安全阀出口：`Exhausted` / `Stalled` 共用一种形状，写方是宿主装配层。
    EventDecl::host(EVENT_DRIVER_STALLED, (Entity::Scope, Verb::Asserted)),
];

/// 两个**宿主通用执行体**的声明。
///
/// `plugin_row` 的写入名单取自实现（`submit_row!` 的并集）；`ActorDecl` 要求 `'static`，
/// 并集每次装配现算，因此只泄漏一次并复用。`port` 是通用受理入口，它还能开下一笔债
/// （工具批次把模型应答展开成逐调用 `request`），所以它是 `request` 的生产者。
fn actors() -> Vec<ActorDecl> {
    static ROW_WRITES: std::sync::OnceLock<&'static [&'static str]> = std::sync::OnceLock::new();
    let row_writes = ROW_WRITES.get_or_init(|| Box::leak(declared_writes().into_boxed_slice()));
    vec![
        ActorDecl {
            name: PLUGIN_ROW,
            mode: crate::kernel::LatencyTier::Deep,
            writes: row_writes,
        },
        ActorDecl {
            name: PORT_ACTOR,
            mode: crate::kernel::LatencyTier::Deep,
            writes: &[EVENT_REQUEST],
        },
    ]
}

/// 端口协议格的**通用出口行**：一笔 `request` 不论运行时端口名是什么，都按
/// `(port, id, attempt)` 开债并由 `sent` / `response` / `failed` / `retry` 收口。
/// `eq` 不含端口名字面量，插件增减只改提供者的 `ports()`，行表不动。
static PORT_ROWS: &[Decl] = &[Decl {
    name: "await-port",
    reads: &[],
    expand: None,
    owes: Pattern {
        source: EVENT_REQUEST,
        key: &["port", "id", "attempt"],
        eq: &[],
    },
    until: &[
        Pattern {
            source: EVENT_SENT,
            key: &["port", "id", "attempt"],
            eq: &[],
        },
        Pattern {
            source: EVENT_RESPONSE,
            key: &["port", "id", "attempt"],
            eq: &[],
        },
        Pattern {
            source: EVENT_FAILED,
            key: &["port", "id", "attempt"],
            eq: &[],
        },
        Pattern {
            source: EVENT_RETRY,
            key: &["port", "id", "attempt"],
            eq: &[],
        },
    ],
    unless: &[],
}];

crate::submit_declarations!(OWNER, submitted_declarations);

fn submitted_declarations() -> PluginDeclarations<'static> {
    PluginDeclarations {
        events: EVENTS,
        facts: &[],
        views: &[],
        ports: &[],
        actors: actors(),
        rows: PORT_ROWS,
    }
}
