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

/// 工具侧的授权拒因：**本插件私有**（D-390）。
///
/// 拆名前它与 model 的语音键拒因共用 `authz.denied`——一个名字两个写方。拆开后
/// 本插件这份只描述「这一笔调用没放行」：它**不是停轮事实**，因此不带 `StopsScope`
/// （C6 不再向它要 `reason`）。会话投影按格位 + 语音键认停，不按这个名字取。
pub(crate) const EVENT_AUTHZ_DENIED: &str = "authz.denied";

/// 熔断器落下的控制事实：**本插件私有**（D-389）。
///
/// 今天只有本插件的熔断写它，没有第二个插件在生产代码里按名字读它 ⇒ 不进
/// `PLUGIN.yml` 的 `shared_events`，格位在本文件的 [`EVENTS`] 里登记。
pub(crate) const EVENT_TASK_CONTROLLED: &str = "task.controlled";

/// 本插件写的事实格位（含两条私有名）。
///
/// `authz.granted` 的写方在本插件（`port.rs` 的授权草稿），会话按名字读回批准结论；
/// 它登记在本插件 `PLUGIN.yml` 的 `shared_events` 里，消费方 import 生成常量
/// （D-364）。`tool.calls.done` 是工具调用收束，session 的进度面按名字读；
/// `artifact.added` 是本次调用的产物格，被 model / 会话投影 / 转写多处按名字读。
/// `authz.denied` / `task.controlled` 是单消费方私有名，只在本表登记。
pub(crate) const EVENTS: &[EventDecl] = &[
    // 授权结论：owner 随写方移交到本插件（D-389 批二）。格位照旧。
    EventDecl::new("authz.granted", (Entity::Scope, Verb::Asserted)),
    EventDecl::new(EVENT_AUTHZ_DENIED, (Entity::Scope, Verb::Asserted)),
    EventDecl::new(EVENT_TASK_CONTROLLED, (Entity::Control, Verb::Opened)),
    EventDecl::host("tool.calls.done", (Entity::Scope, Verb::Asserted)),
    // 产物格：载荷来自执行体交出的草稿，落行仍走宿主写口 ⇒ 与 `tool.calls.done` 同一口径。
    EventDecl::host("artifact.added", (Entity::Artifact, Verb::Asserted)),
];

/// 与 [`EVENTS`] 同源的事实形状：`artifact.added` 的消费者按 `tool` / `success` /
/// `tool_call_id` 读产物格，没有形状声明就无法校验这些字段（`UnknownSource`）。
///
/// 形状只登记**确定会写**的字段；`annotations` 一类富化字段随工具声明走，不在此列。
///
/// **恢复的第二次落格**：`user.answer` 的 `answer` / `approve` / `deny` 恢复落的是
/// **同一调用的第二次落格**，幂等键带 `:resume` 后缀（`artifact:{wire}:resume`）。
/// 它**不是新事实**——事件名仍是 `artifact.added`、幂等维度仍是 `tool_call_id`，
/// 投影按 `{wire}-result` 覆盖同一张卡；带后缀只是为了让事实源按 `(kind, key)`
/// 去重时不把「新状态」当成重复吞掉。别按这个后缀再造新的事件名或新键维度。
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
static ROWS: &[Decl] = &[
    Decl {
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
    },
    // 产物格的**归宿行**（`by = None`，不派发）。
    //
    // `artifact.added` 的读者是 model 的续接债与会话投影：它们不是环里的行前件，
    // 因此这张行只声明「这一格有人看护」，让装载期的不变量有一个落点。写成
    // `by = Some(...)` 会凭空给每条产物开一笔债；撤回了记录类索引之后也不该再有
    // 行把产物格当前件派发。
    //
    // `owes` 与 `until` 同键（`tool_call_id`）：既满足 `OrphanFact`，也满足
    // `KeyMismatch`，而 `by = None` 让它不进任何一轮的候选集。
    Decl {
        name: "tool.artifact",
        by: None,
        tier: LatencyTier::Reflex,
        expand: None,
        owes: Pattern {
            source: "artifact.added",
            key: &["tool_call_id"],
            eq: &[],
        },
        until: &[Pattern {
            source: "artifact.added",
            key: &["tool_call_id"],
            eq: &[],
        }],
        unless: &[],
    },
];

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
