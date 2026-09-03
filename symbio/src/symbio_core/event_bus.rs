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
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::LazyLock;
use tokio::sync::mpsc;

/// 每个 sessionId 最多保留的回放事件数
const PENDING_EVENTS_CAP: usize = 64;

/// 事件类型常量
pub const KIND_SESSION: &str = "session";
pub const KIND_EXPLORER: &str = "explorer";
pub const KIND_SYSTEM: &str = "system";
/// 资源状态实时事件（model / mcp / skill / agent / session 通用）
pub const KIND_RESOURCE: &str = "resource";

/// 事件 Bus 全局订阅者容器
///
/// Key 是 `connection_id`（每个前端连接一个），Value 是它的发送端。
static SUBSCRIBERS: LazyLock<DashMap<String, mpsc::Sender<PluginFrame>>> =
    LazyLock::new(DashMap::new);

/// 按 sessionId 缓存最近的事件（用于新订阅者拉取回放，避免切回时丢中间事件）
///
/// 设计目的：
/// 1. 解决"前端切换会话后，MODEL 仍在流式输出，新订阅者错过中间帧"的问题
/// 2. 解决"前端重载/重连时，正在进行的会话上下文丢失"的问题
/// 3. **不替代持久化**——持久化由 session 插件负责，这里只缓存最近的 64 帧
///
/// 工作方式：
/// - `publish` 时：每个事件 push 到 `pending_events[session_id]`
/// - 超过 `PENDING_EVENTS_CAP` 时从头部弹出最旧事件
/// - 订阅者通过 `pending/snapshot` RPC 取走并清空缓冲
static PENDING_EVENTS: LazyLock<DashMap<String, VecDeque<Value>>> = LazyLock::new(DashMap::new);

/// 事件 Bus 帧载荷
///
/// 前端收到的每一帧都是这种结构：
/// ```json
/// {
///   "type": "bus_event",
///   "data": {
///     "kind": "session",
///     "session_id": "abc123",
///     "data": { ... 原始业务数据 ... }
///   }
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusEvent {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub data: Value,
}

/// 订阅请求
#[derive(Debug, Clone, Deserialize)]
pub struct SubscribeRequest {
    /// 可选：限定只接收某些 kind 的事件（如 ["session"]）
    #[serde(default)]
    pub kinds: Option<Vec<String>>,
}

/// 取消订阅请求
#[derive(Debug, Clone, Deserialize)]
pub struct UnsubscribeRequest {
    pub connection_id: String,
}

/// 拉取回放事件请求
#[derive(Debug, Clone, Deserialize)]
pub struct PendingSnapshotRequest {
    /// 要拉取的会话 ID
    pub session_id: String,
}

/// 拉取回放事件响应
#[derive(Debug, Clone, Serialize)]
pub struct PendingSnapshotResponse {
    pub session_id: String,
    /// 该 sessionId 缓冲的所有事件（按时间正序），调用后清空缓冲
    pub events: Vec<Value>,
}

/// 注册一个订阅者发送端（由 `event_bus` 插件在建立连接时调用）
pub fn register_subscriber(connection_id: String, tx: mpsc::Sender<PluginFrame>) {
    SUBSCRIBERS.insert(connection_id, tx);
}

/// 反注册订阅者（连接断开时调用）
pub fn unregister_subscriber(connection_id: &str) {
    SUBSCRIBERS.remove(connection_id);
}

/// 取订阅者发送端（供 `event_bus` 插件在清理任务中使用）
pub fn subscriber_sender(connection_id: &str) -> Option<mpsc::Sender<PluginFrame>> {
    SUBSCRIBERS.get(connection_id).map(|e| e.clone())
}

/// 全局事件总线门面
///
/// 其他插件（session、explorer）调用静态方法 `publish` 推送事件。
pub struct EventBus;

impl EventBus {
    /// 推送事件到所有订阅者
    ///
    /// - `kind`: 事件类型（如 "session"、"explorer"）
    /// - `session_id`: 可选，关联到具体会话
    /// - `data`: 原始业务数据（任意 JSON）
    ///
    /// 同时把事件写入 `PENDING_EVENTS[session_id]` 缓冲，供新订阅者回放。
    /// 缓冲超过 `PENDING_EVENTS_CAP` 时丢弃最旧事件（环形）。
    pub async fn publish(kind: &str, session_id: Option<&str>, data: Value) {
        let envelope = build_envelope(kind, session_id, &data);
        let frame = PluginFrame::Data(envelope.clone());

        // 写入回放缓冲（仅当 session_id 存在；system 事件不需要回放）
        if let Some(sid) = session_id {
            append_pending(sid, envelope.clone());
        }

        // 收集失效的订阅者（is_closed 的）
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

    /// 同步版本（不等待），用于 watcher 回调等非异步上下文
    pub fn try_publish(kind: &str, session_id: Option<&str>, data: Value) {
        let envelope = build_envelope(kind, session_id, &data);
        let frame = PluginFrame::Data(envelope.clone());

        if let Some(sid) = session_id {
            append_pending(sid, envelope.clone());
        }

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

    /// 拉取并清空指定 sessionId 的回放缓冲
    ///
    /// - 返回该 sessionId 累积的所有事件（按时间正序）
    /// - 调用后清空该 sessionId 的缓冲
    /// - 用于前端挂载某个 sessionId 的 `useChatConnection` 时补齐中间帧
    pub fn drain_pending(session_id: &str) -> Vec<Value> {
        if let Some(mut entry) = PENDING_EVENTS.get_mut(session_id) {
            return entry.drain(..).collect();
        }
        Vec::new()
    }

    /// 发布一个资源状态变更事件（resource kind）
    ///
    /// 各资源插件在状态**运行时变化**（如会话 busy/idle、连接测试结果）时调用，
    /// 前端 `subscribe({ kind: 'resource' })` 即时刷新列表/详情状态角标。
    ///
    /// 无 session 关联，因此**不入回放缓冲**——重连后的最新状态由
    /// 各 `resources/list` / `resources/get` 的初始拉取兜底。
    pub async fn publish_resource_status(
        resource_type: &str,
        id: &str,
        status: &str,
        status_detail: Option<String>,
    ) {
        Self::publish(
            KIND_RESOURCE,
            None,
            json!({
                "resource_type": resource_type,
                "id": id,
                "status": status,
                "status_detail": status_detail,
            }),
        )
        .await;
    }

    /// `publish_resource_status` 的同步版本（用于非异步回调）
    pub fn try_publish_resource_status(
        resource_type: &str,
        id: &str,
        status: &str,
        status_detail: Option<String>,
    ) {
        Self::try_publish(
            KIND_RESOURCE,
            None,
            json!({
                "resource_type": resource_type,
                "id": id,
                "status": status,
                "status_detail": status_detail,
            }),
        );
    }

    /// 当前订阅者数量（用于调试）
    pub fn subscriber_count() -> usize {
        SUBSCRIBERS.len()
    }

    /// 当前 sessionId 缓冲中的事件数（用于调试）
    pub fn pending_count(session_id: &str) -> usize {
        PENDING_EVENTS.get(session_id).map(|q| q.len()).unwrap_or(0)
    }
}

/// 构建事件信封（BusEvent 序列化后的 JSON）
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

/// 追加事件到指定 sessionId 的回放缓冲
fn append_pending(session_id: &str, envelope: Value) {
    let mut entry = PENDING_EVENTS.entry(session_id.to_string()).or_default();
    if entry.len() >= PENDING_EVENTS_CAP {
        entry.pop_front();
    }
    entry.push_back(envelope);
}
