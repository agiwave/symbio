//! 探针端口的世界声明：一个端口 + 一行（`by = PORT_ACTOR`，环按通用链路执行）。
//!
//! 端口是数据不是类型：环不持有端口清单，只按端口名问冻结表。本文件是「新增一个
//! 世界端口」的全部声明成本——不出本 crate。

use symbio_plugin_api::ring::PortDecl;
use symbio_plugin_api::{PluginDeclarations, EVENT_SENT};

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

// 端口声明由环的通用出口行（`await-port`）承接：端口名是运行时字符串，不进声明表。
symbio_plugin_api::submit_declarations!(OWNER, submitted_declarations);

fn submitted_declarations() -> PluginDeclarations<'static> {
    PluginDeclarations {
        events: &[],
        facts: &[],
        views: &[],
        ports: PORTS,
        actors: Vec::new(),
        rows: &[],
    }
}
