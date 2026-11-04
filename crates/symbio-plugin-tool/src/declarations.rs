//! `port.tool` 的世界声明（DE-047 不变量 6 / D-303）。
//!
//! 端口名与租约参数随本插件提交：环不持有端口清单，只按端口名问冻结表
//! （`Frozen::ports`）。加一个端口只改提供者的 `ports()`，不改环。

use symbio_plugin_api::ring::PortDecl;
use symbio_plugin_api::{Entity, EventDecl, PluginDeclarations, Verb, EVENT_SENT};

use super::port::TOOL;

pub(crate) const OWNER: &str = "crates/symbio-plugin-tool";

/// 本插件写的**单消费方**世界事实（不进共享表：单消费方的名字随 owner 走）。
pub(crate) const EVENTS: &[EventDecl] = &[EventDecl::host(
    "tool.calls.done",
    (Entity::Scope, Verb::Asserted),
)];

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

symbio_plugin_api::submit_declarations!(OWNER, submitted_declarations);

fn submitted_declarations() -> PluginDeclarations<'static> {
    PluginDeclarations {
        events: EVENTS,
        facts: &[],
        views: &[],
        ports: PORTS,
        actors: Vec::new(),
        // 行声明仍由环持有（步二搬走）；本插件先只交端口与提供者。
        rows: &[],
    }
}
