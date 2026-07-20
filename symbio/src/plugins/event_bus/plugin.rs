//! Event Bus 插件
//!
//! 设计目标：
//! 1. **单连接**：前端只建立一条 `core/event_bus` 长连接，订阅所有事件
//! 2. **标签分发**：每个事件 frame 都带 `{ kind, session_id, data }` 元数据
//! 3. **中央路由**：session、explorer 等插件通过 `EventBus::publish` 推送事件
//!
//! ## 用法
//!
//! ```ignore
//! // 在 session 插件中
//! EventBus::publish("session", Some(&session_id), stream_event_json).await;
//! ```
//!
//! `EventBus` 门面是 `symbio_core::event_bus::EventBus`（跨插件共享的核心设施），
//! 本插件仅负责建立订阅连接、转发 `pending/snapshot` 等 RPC。
//!
//! 前端订阅后，根据 `kind` 字段分发到不同模块；`session_id` 决定具体会话。

use crate::symbio_core::event_bus::{
    register_subscriber, unregister_subscriber, EventBus, PendingSnapshotRequest,
    PendingSnapshotResponse, SubscribeRequest,
};
use crate::symbio_core::schemas::common::SimpleResponse;
use crate::symbio_core::{
    InvokeRequest, InvokeRequestExt, InvokeResponse, Plugin, PluginChannel, PluginError,
    PluginFrame, PluginMeta, PluginPayload, PLUGIN_EVENT_BUS,
};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

/// Event Bus 插件
pub struct EventBusPlugin;

impl EventBusPlugin {
    /// 工厂方法（满足 `submit_object_creator!` 协议）
    pub fn build(_ctx: Arc<dyn InvokeRequest>) -> Arc<dyn Plugin> {
        Arc::new(EventBusPlugin) as Arc<dyn Plugin>
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_EVENT_BUS, "Event Bus")
            .with_description("统一事件总线：单连接订阅所有插件事件")
            .with_version("0.1.0")
    }

    /// 处理订阅请求（connect_v2）
    ///
    /// 分配一个 connection_id，建立 PluginChannel，
    /// 返回 peer_channel 供 transport 通过 mpsc 推送到前端。
    pub async fn handle_subscribe(
        _ctx: Arc<dyn InvokeRequest>,
        _req: SubscribeRequest,
    ) -> InvokeResponse<PluginPayload> {
        let (peer, mine) = PluginChannel::pair(2048);

        // 预留：基于 _req.kinds 过滤事件（暂未实现，所有事件都推送）
        let _ = _req.kinds;

        // 注册到全局表
        let connection_id = Uuid::new_v4().to_string();
        register_subscriber(connection_id.clone(), mine.tx.clone());

        // 异步清理：mine.rx 结束时自动反注册
        let conn_id_for_cleanup = connection_id.clone();
        tokio::spawn(async move {
            // 只持有 rx，不消费
            let _rx = mine.rx;
            // 等待 cancellation / 关闭信号（PluginChannel 的 cancel_token 会被断开时触发）
            mine.cancel_token.cancelled().await;
            unregister_subscriber(&conn_id_for_cleanup);
        });

        // 立即推送一个 connected 事件
        let _ = mine
            .tx
            .send(PluginFrame::Data(json!({
                "type": "bus_event",
                "data": {
                    "kind": "system",
                    "session_id": null,
                    "data": {
                        "event": "connected",
                        "connection_id": connection_id,
                    }
                }
            })))
            .await;

        Ok(PluginPayload::Session(peer))
    }
}

#[async_trait]
impl Plugin for EventBusPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn route(self: Arc<Self>, ctx: Arc<dyn InvokeRequest>) -> InvokeResponse<PluginPayload> {
        let path = ctx.get(crate::symbio_core::PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);

        match path {
            "subscribe" => {
                let req: SubscribeRequest = ctx.payload()?;
                EventBusPlugin::handle_subscribe(ctx, req).await
            },
            "pending/snapshot" => {
                let req: PendingSnapshotRequest = ctx.payload()?;
                let events = EventBus::drain_pending(&req.session_id);
                let resp = PendingSnapshotResponse {
                    session_id: req.session_id,
                    events,
                };
                Ok(PluginPayload::new(&resp))
            },
            "ping" => {
                let resp = SimpleResponse::success_with_message(format!(
                    "pong ({} subscribers)",
                    EventBus::subscriber_count()
                ));
                Ok(PluginPayload::new(&resp))
            },
            _ => Err(PluginError::NotFound(format!(
                "[event_bus] 未知子命令: {}",
                path
            ))),
        }
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _ctx: Arc<dyn InvokeRequest>,
    ) -> InvokeResponse<PluginPayload> {
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

crate::submit_object_creator!(PLUGIN_EVENT_BUS, EventBusPlugin::build, dyn Plugin);
