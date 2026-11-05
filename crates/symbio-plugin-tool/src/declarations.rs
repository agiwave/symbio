//! `port.tool` 的世界声明（DE-047 不变量 6 / D-303）。
//!
//! 端口名与租约参数随本插件提交：环不持有端口清单，只按端口名问冻结表
//! （`Frozen::ports`）。加一个端口只改提供者的 `ports()`，不改环。

use symbio_plugin_api::ring::{Decl, Lit, Pattern, PortDecl};
use symbio_plugin_api::rows::PORT_ACTOR;
use symbio_plugin_api::{
    Entity, EventDecl, FactDecl, LatencyTier, PluginDeclarations, Verb, EVENT_FAILED,
    EVENT_REQUEST, EVENT_RESPONSE, EVENT_RETRY, EVENT_SENT,
};

use super::port::TOOL;

pub(crate) const OWNER: &str = "crates/symbio-plugin-tool";

/// 本插件写的世界事实。
///
/// `tool.calls.done` 是**单消费方**的事实（不进共享表：名字随 owner 走）；
/// `artifact.added` 是本次调用的产物格，被 notes / model / 转写投影多处按名字读，
/// 因此同时登记在本插件 `PLUGIN.yml` 的 `shared_events` 里。
pub(crate) const EVENTS: &[EventDecl] = &[
    EventDecl::host("tool.calls.done", (Entity::Scope, Verb::Asserted)),
    // 产物格：载荷来自执行体交出的草稿，落行仍走宿主写口 ⇒ 与 `tool.calls.done` 同一口径。
    EventDecl::host("artifact.added", (Entity::Artifact, Verb::Asserted)),
];

/// 与 [`EVENTS`] 同源的事实形状：`notes.index` 按 `tool` / `success` / `tool_call_id`
/// 读产物格，没有形状声明就无法校验这些字段（`UnknownSource`）。
///
/// 形状只登记**确定会写**的字段；`annotations` 一类富化字段随工具声明走，不在此列。
pub(crate) const FACTS: &[FactDecl] = &[FactDecl {
    name: "artifact.added",
    port: None,
    key: &["tool_call_id"],
    expand: &[],
    fields: &[
        "tool",
        "text",
        "tool_call_id",
        "args",
        "success",
        "needs_user_action",
        "prompt",
        "failure_kind",
    ],
}];

/// 本插件服务的端口：**一次工具调用**。
///
/// 上限是端口级的天花板，不是「每个调用都会重试」的承诺（D-138）：普通批次调用
/// 的等待方随草稿消失（`awaited = false`），只有唤醒调用能从事实重建等待方。
pub(crate) static PORTS: &[PortDecl] = &[PortDecl {
    port: TOOL,
    before: EVENT_SENT,
    lease_ms: 600_000,
    max_attempts: 2,
    // 一次模型响应可带 N 个工具调用：同批至多四条在飞。
    concurrency: 4,
}];

/// 端口行：一笔已宣布的 `request(port=tool)` → 落 `sent` → 执行体跑完 → `response` / `failed`。
///
/// `by` 写宿主词汇 [`PORT_ACTOR`]：动作是环的通用受理入口，本插件只声明端口与行
/// （D-303 的「端口所属插件声明通用行」）。
static ROWS: &[Decl] = &[Decl {
    name: "await-tool",
    by: Some(PORT_ACTOR),
    // 工具调用可能跑秒–分钟（`PORTS` 的租约 600s）：最宽档。
    tier: LatencyTier::Deep,
    expand: None,
    owes: Pattern {
        source: EVENT_REQUEST,
        key: &["port", "id", "attempt"],
        eq: &[("port", Lit::Str(TOOL))],
    },
    until: &[
        Pattern {
            source: EVENT_SENT,
            key: &["port", "id", "attempt"],
            eq: &[("port", Lit::Str(TOOL))],
        },
        Pattern {
            source: EVENT_RESPONSE,
            key: &["port", "id", "attempt"],
            eq: &[("port", Lit::Str(TOOL))],
        },
        Pattern {
            source: EVENT_FAILED,
            key: &["port", "id", "attempt"],
            eq: &[("port", Lit::Str(TOOL))],
        },
        Pattern {
            source: EVENT_RETRY,
            key: &["port", "id", "attempt"],
            eq: &[("port", Lit::Str(TOOL))],
        },
    ],
    unless: &[],
}];

symbio_plugin_api::submit_declarations!(OWNER, submitted_declarations);

fn submitted_declarations() -> PluginDeclarations<'static> {
    PluginDeclarations {
        events: EVENTS,
        facts: FACTS,
        views: &[],
        ports: PORTS,
        actors: Vec::new(),
        rows: ROWS,
    }
}
