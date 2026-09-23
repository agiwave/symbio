//! Event Bus 插件
//!
//! 设计目标：
//! 1. **单连接**：前端只建立一条 `core/event_bus` 长连接，订阅所有事件
//! 2. **标签分发**：每个事件 frame 都带 `{ kind, session_id, data }` 元数据
//! 3. **中央路由**：vdfs 等插件通过 `EventBus::try_publish` 推送事件
//!
//! ## 用法
//!
//! `kind` 取 `symbio_core::event_bus::KIND_*` 常量（消费方按它分发，裸字面量改名不会
//! 编译失败）：
//!
//! ```ignore
//! // 在 vdfs 宿主中（一切资源的实时变更都走这一条）
//! EventBus::try_publish(KIND_VDFS, None, change_json);
//! ```
//!
//! `EventBus` 门面是 `symbio_core::event_bus::EventBus`（跨插件共享的核心设施），
//! 本插件仅负责建立订阅连接。
//!
//! 订阅方拿到帧后按 `kind` 分派；`session_id` 只是 `system` 握手帧的关联信息，
//! **业务身份一律在载荷自己的地址里**（VDFS 变更的 `path`）——会话域曾经的
//! `kind = "session"` 频道已废除，见 `docs/architecture/PROTOCOLS.md` §事件总线频道。
//!
//! 历史上的 `pending/snapshot` 路由已随会话事件频道一并废除：它缓冲的是按 `session_id`
//! 灌入的事件帧，而 VDFS 变更的发布方传 `session_id = None`（身份在地址里），缓冲永远
//! 为空——路由成了恒返回空数组的空壳。

use crate::symbio_core::event_bus::{
    build_envelope, register_subscriber, unregister_subscriber, EventBus, SubscribeRequest,
    KIND_SYSTEM,
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
        // 容量与 `session/stream` 对等（4096）。
        //
        // 这条频道承载**全部**实时面：消息正文的逐帧增量（`VdfsChange.delta`，热路径）
        // 与会话 / 资源变更（低频）共用它。原先的 2048 是「只有资源变更」时的取值，
        // 承接消息增量后偏小——满了虽不会丢（`try_publish` 会补 resync 指令，
        // 消费端重读作用域后自愈），但每次 resync 都换来一次整份重读，代价远大于
        // 多留 2048 个槽位。
        let (peer, mine) = PluginChannel::pair(4096);

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

        // 立即推送一个 connected 事件。
        //
        // 信封走 `build_envelope`（形状的唯一构建入口），不自己拼 `json!`：
        // 手拼的那份在形状漂移时不会有任何编译错误。
        let _ = mine
            .tx
            .send(PluginFrame::data(build_envelope(
                KIND_SYSTEM,
                None,
                json!({
                    "event": "connected",
                    "connection_id": connection_id,
                }),
            )))
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
            }
            "ping" => {
                let resp = SimpleResponse::success_with_message(format!(
                    "pong ({} subscribers)",
                    EventBus::subscriber_count()
                ));
                Ok(PluginPayload::new(&resp))
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::SimpleRequest;

    fn ctx(path: &str) -> Arc<dyn InvokeRequest> {
        let req = SimpleRequest::new(None, None);
        req.set(crate::symbio_core::PATH, path.to_string());
        Arc::new(req)
    }

    fn data_of(p: PluginPayload) -> serde_json::Value {
        match p {
            PluginPayload::Data(d) => d.serialize().unwrap(),
            other => panic!(
                "expected data payload, got {:?}",
                std::mem::discriminant(&other)
            ),
        }
    }

    #[tokio::test]
    async fn unknown_subcommand_is_not_found() {
        let r = Arc::new(EventBusPlugin).route(ctx("bogus")).await;
        assert!(matches!(r, Err(PluginError::NotFound(_))));
    }

    #[tokio::test]
    async fn ping_reports_subscriber_count() {
        let p = Arc::new(EventBusPlugin)
            .route(ctx("ping"))
            .await
            .expect("ping 必须成功");
        let s = serde_json::to_string(&data_of(p)).unwrap();
        assert!(s.contains("pong"), "ping 响应应含 pong：{s}");
    }

    /// 总线插件不贡献工具——它在能力树里是个纯连接入口
    #[tokio::test]
    async fn traverse_contributes_no_tools() {
        let p = Arc::new(EventBusPlugin)
            .traverse(String::new(), ctx(""))
            .await
            .expect("traverse 必须成功");
        assert_eq!(data_of(p), serde_json::json!([]));
    }
}
