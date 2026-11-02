//! 本插件拥有的**世界端口**声明（DE-047 不变量 6 / D-302）。
//!
//! 端口是数据不是类型：环不持有端口清单，只按端口名问冻结表（`Frozen::ports`）。
//! 本插件服务的端口名与租约参数一起交在这里，加端口只改本文件——不改环的任何文件。

use symbio_plugin_api::ring::{Decl, Lit, Pattern, PortDecl};
use symbio_plugin_api::rows::PORT_ACTOR;
use symbio_plugin_api::{
    LatencyTier, PluginDeclarations, EVENT_FAILED, EVENT_REQUEST, EVENT_RESPONSE, EVENT_RETRY,
    EVENT_SENT,
};

use super::port::NOTIFY;

pub(crate) const OWNER: &str = "crates/symbio-plugin-telegram";

/// 本插件服务的端口：**把一条消息发到外部会话**。
///
/// `max_attempts = 1`：发送是副作用动作，崩溃后自动重放会重复推送；重试交给租约看护
/// 之外的调用方（它掌握语义上的去重）。
pub(crate) static PORTS: &[PortDecl] = &[PortDecl {
    port: NOTIFY,
    before: EVENT_SENT,
    lease_ms: 600_000,
    max_attempts: 1,
    // 单一 Bot 通道：串行。
    concurrency: 0,
}];

/// 端口行：一笔已宣布的 `request(port=notify)` → 落 `sent` → 执行体跑完 → `response` / `failed`。
///
/// `by` 写宿主词汇 [`PORT_ACTOR`]：动作是环的通用受理入口，本插件只声明端口与行。
/// 出口含 `sent`（该端口的 `before`）——装载期据此认「这一行够得到端口协议」。
static ROWS: &[Decl] = &[Decl {
    name: "notify-send",
    // 等待阶段的末位（`await-tool` 72 之后）：通知不挡任何轮内进程。
    phase: 7,
    anchor: 73,
    by: Some(PORT_ACTOR),
    // 一次外部 HTTP 发送，不调模型：快速档。
    tier: LatencyTier::Fast,
    expand: None,
    owes: Pattern {
        source: EVENT_REQUEST,
        key: &["port", "id", "attempt"],
        eq: &[("port", Lit::Str(NOTIFY))],
    },
    until: &[
        Pattern {
            source: EVENT_SENT,
            key: &["port", "id", "attempt"],
            eq: &[("port", Lit::Str(NOTIFY))],
        },
        Pattern {
            source: EVENT_RESPONSE,
            key: &["port", "id", "attempt"],
            eq: &[("port", Lit::Str(NOTIFY))],
        },
        Pattern {
            source: EVENT_FAILED,
            key: &["port", "id", "attempt"],
            eq: &[("port", Lit::Str(NOTIFY))],
        },
        Pattern {
            source: EVENT_RETRY,
            key: &["port", "id", "attempt"],
            eq: &[("port", Lit::Str(NOTIFY))],
        },
    ],
    unless: &[],
}];

symbio_plugin_api::submit_declarations!(OWNER, submitted_declarations);

fn submitted_declarations() -> PluginDeclarations<'static> {
    PluginDeclarations {
        events: &[],
        facts: &[],
        views: &[],
        ports: PORTS,
        actors: Vec::new(),
        rows: ROWS,
    }
}
