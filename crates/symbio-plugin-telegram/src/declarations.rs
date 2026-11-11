//! 本插件拥有的**世界端口**声明（DE-047 不变量 6 / D-302）。
//!
//! 端口是数据不是类型：环不持有端口清单，只按端口名问冻结表（`Frozen::ports`）。
//! 本插件服务的端口名与租约参数一起交在这里，加端口只改本文件——不改环的任何文件。

use symbio_plugin_api::ring::PortDecl;
use symbio_plugin_api::{PluginDeclarations, EVENT_SENT};

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
