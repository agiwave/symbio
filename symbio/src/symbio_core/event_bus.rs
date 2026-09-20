//! 全局事件总线门面
//!
//! `EventBus` 是一个跨插件的全局发布设施，供 session、explorer 等插件推送事件，
//! 由前端通过 `event_bus` 插件建立的一条长连接统一订阅。
//!
//! 把它放在 `symbio_core`（而非某个具体插件）是为了遵循"插件互不可见"的分层原则：
//! 任何插件都通过 `crate::symbio_core::event_bus::EventBus` 访问，而非直接引用
//! `plugins::event_bus` 模块。

use crate::symbio_core::PluginFrame;
use dashmap::DashMap;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::LazyLock;
use tokio::sync::mpsc;

/// 事件类型（`kind`）词表 —— **发布方一律引本常量，不要写裸字面量**。
///
/// 为什么要有它：`kind` 是**跨进程边界的字符串**，改名不会编译失败，只会让消费方
/// 的分发静默失效（与 `symbio_core::paths` 同一类风险）。发布点曾直接写
/// `"session"` / `"system"`，于是这两个常量声明出来后无人引用——正是
/// `chat_message.rs` 那段「枚举改名时就会出现不一致」警告的形状。
///
/// **闭集只有一个家**：词表住在这里，哪怕发布方在别的插件（`KIND_VDFS` 由 vdfs 发）。
/// 文档见 `docs/architecture/PROTOCOLS.md` §事件总线频道——那一行带
/// `<!-- vocab:KIND_ -->` 标记，由 `plugin-entry-audit` 的 E-008 与代码**双向**比对；
/// 「发布点写裸字面量」由 `grep-audit` 的 S-009 拦。
///
/// 曾经的 `KIND_SESSION`（会话域 `StreamEvent` 频道）已随该频道一并废除：会话状态与
/// 转写都是 VDFS 节点，走 `KIND_VDFS`（见 `session/docs/node-state-streaming.md`）。
/// 常量留在词表里只会让「再发一条会话事件」看起来仍然合法。
pub const KIND_SYSTEM: &str = "system";

/// VDFS 变更频道 —— **资源实时的唯一频道**。
///
/// 不由本模块发布：一切资源的生命周期与状态变化都是 VDFS 变更，由 provider 写成功后
/// 调 `symbio_core::vdfs::host::notify_change`，经 `watch` 的 sink
/// （`plugins/vdfs/host.rs::event_bus_sink`）装进 `VdfsChangeEvent` 投到总线上（规范 §9）。
///
/// 常量放这里而非 vdfs 插件，是贯彻上面那条「闭集只有一个家」——发布方只是引用者。
/// 历史上并存的 `kind = "entity"` 频道（`publish_entity_changed` /
/// `publish_entity_status`）已随实体机制一并废除。
pub const KIND_VDFS: &str = "vdfs";

/// 事件 Bus 全局订阅者容器
///
/// Key 是 `connection_id`（每个前端连接一个），Value 是它的发送端。
static SUBSCRIBERS: LazyLock<DashMap<String, mpsc::Sender<PluginFrame>>> =
    LazyLock::new(DashMap::new);

/// 订阅请求
#[derive(Debug, Clone, Deserialize)]
pub struct SubscribeRequest {
    /// 可选：限定只接收某些 kind 的事件（如 ["vdfs"]）
    #[serde(default)]
    pub kinds: Option<Vec<String>>,
}

/// 注册一个订阅者发送端（由 `event_bus` 插件在建立连接时调用）
pub fn register_subscriber(connection_id: String, tx: mpsc::Sender<PluginFrame>) {
    SUBSCRIBERS.insert(connection_id, tx);
}

/// 反注册订阅者（连接断开时调用）
pub fn unregister_subscriber(connection_id: &str) {
    SUBSCRIBERS.remove(connection_id);
}

/// 全局事件总线门面
///
/// 其他插件（vdfs 等）调用静态方法 `publish` 推送事件。会话域曾经的 `kind = "session"`
/// 频道已废除——会话状态与转写都是 VDFS 节点变更，走 `KIND_VDFS`（见
/// `session/docs/node-state-streaming.md`）。
pub struct EventBus;

impl EventBus {
    /// 推送事件到所有订阅者（同步版，供 watcher 回调等非异步上下文使用）
    ///
    /// - `kind`: 事件类型（如 `KIND_VDFS`）
    /// - `session_id`: 可选，关联到具体会话（VDFS 变更不填，身份在载荷的地址里）
    /// - `data`: 原始业务数据（任意 JSON）
    pub fn try_publish(kind: &str, session_id: Option<&str>, data: Value) {
        let envelope = build_envelope(kind, session_id, &data);
        let frame = PluginFrame::Data(envelope);

        let mut to_remove: Vec<String> = Vec::new();
        for entry in SUBSCRIBERS.iter() {
            let (id, tx) = (entry.key(), entry.value());
            if tx.is_closed() || tx.try_send(frame.clone()).is_err() {
                to_remove.push(id.clone());
            }
        }
        for id in to_remove {
            SUBSCRIBERS.remove(&id);
        }
    }

    /// 当前订阅者数量（用于调试）
    pub fn subscriber_count() -> usize {
        SUBSCRIBERS.len()
    }
}

/// 构建事件信封
fn build_envelope(kind: &str, session_id: Option<&str>, data: &Value) -> Value {
    json!({
        "type": "bus_event",
        "data": {
            "kind": kind,
            "session_id": session_id,
            "data": data,
        }
    })
}
