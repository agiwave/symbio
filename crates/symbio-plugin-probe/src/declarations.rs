//! 探针端口的世界声明：一个端口 + 一行（`by = PORT_ACTOR`，环按通用链路执行）。
//!
//! 端口是数据不是类型：环不持有端口清单，只按端口名问冻结表。本文件是「新增一个
//! 世界端口」的全部声明成本——不出本 crate。

use symbio_plugin_api::ring::{Decl, Lit, Pattern, PortDecl};
use symbio_plugin_api::rows::PORT_ACTOR;
use symbio_plugin_api::{
    LatencyTier, PluginDeclarations, EVENT_FAILED, EVENT_REQUEST, EVENT_RESPONSE, EVENT_RETRY,
    EVENT_SENT,
};

use super::port::PROBE_PORT;

pub(crate) const OWNER: &str = "crates/symbio-plugin-probe";

/// 本插件服务的端口：**把一段文本回显**。
pub(crate) static PORTS: &[PortDecl] = &[PortDecl {
    port: PROBE_PORT,
    before: EVENT_SENT,
    lease_ms: 60_000,
    max_attempts: 1,
    concurrency: 0,
}];

/// 端口行：一笔已宣布的 `request(port=probe_echo)` → 落 `sent` → 执行体跑完 →
/// `response` / `failed`。`by` 写宿主词汇 [`PORT_ACTOR`]，环不认识端口名。
static ROWS: &[Decl] = &[Decl {
    name: "probe-echo",
    by: Some(PORT_ACTOR),
    tier: LatencyTier::Fast,
    expand: None,
    owes: Pattern {
        source: EVENT_REQUEST,
        key: &["port", "id", "attempt"],
        eq: &[("port", Lit::Str(PROBE_PORT))],
    },
    until: &[
        Pattern {
            source: EVENT_SENT,
            key: &["port", "id", "attempt"],
            eq: &[("port", Lit::Str(PROBE_PORT))],
        },
        Pattern {
            source: EVENT_RESPONSE,
            key: &["port", "id", "attempt"],
            eq: &[("port", Lit::Str(PROBE_PORT))],
        },
        Pattern {
            source: EVENT_FAILED,
            key: &["port", "id", "attempt"],
            eq: &[("port", Lit::Str(PROBE_PORT))],
        },
        Pattern {
            source: EVENT_RETRY,
            key: &["port", "id", "attempt"],
            eq: &[("port", Lit::Str(PROBE_PORT))],
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
