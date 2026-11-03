//! 本插件拥有的**世界端口**声明（DE-047 不变量 6）。
//!
//! 端口是数据不是类型：环不持有端口清单，只按端口名问冻结表（`Frozen::ports`）。
//! 本插件服务的端口名与租约参数一起交在这里，加端口只改本文件。

use symbio_plugin_api::events::EVENT_MODEL_STREAM;
use symbio_plugin_api::port::ports;
use symbio_plugin_api::ring::PortDecl;
use symbio_plugin_api::{Entity, EventDecl, PluginDeclarations, Verb, EVENT_SENT};

pub(crate) const OWNER: &str = "crates/symbio-plugin-model";

/// 本插件服务的端口：一次**模型往返**。
///
/// `max_attempts = 1`：租约到期只发生在进程重启之后，等这笔应答的 future 已经随
/// 进程一起没了；重试只会留下一条永不派发的 `request`（`D-138`）。
pub(crate) static PORTS: &[PortDecl] = &[PortDecl {
    port: ports::MODEL,
    before: EVENT_SENT,
    lease_ms: 600_000,
    max_attempts: 1,
    // 一次模型往返是一条调用；同批并发在它下面的工具端口。
    concurrency: 0,
}];

/// 一次往返的**流式增量**（`D-306` 裁决三）：执行体每收一帧就落一行，
/// 载荷是该帧投影出的 `ChatNode`。它是 model 自己的事实行——环不解析、
/// 展示层按行名投影；节点 id 与正文因此随事实入格，不再由环的桥记账。
pub(crate) static EVENTS: &[EventDecl] = &[EventDecl::new(
    EVENT_MODEL_STREAM,
    (Entity::Turn, Verb::Progressed),
)];

symbio_plugin_api::submit_declarations!(OWNER, submitted_declarations);

fn submitted_declarations() -> PluginDeclarations<'static> {
    PluginDeclarations {
        events: EVENTS,
        facts: &[],
        views: &[],
        ports: PORTS,
        actors: Vec::new(),
        rows: &[],
    }
}
