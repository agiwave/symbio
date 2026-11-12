//! Event Bus 插件
//!
//! 本插件做三件事，不多做：
//! 1. **单连接**：前端只建立一条 `core/event_bus` 长连接，订阅所有帧
//! 2. **广播**：帧原样投递给每个已注册连接，`{ kind, session_id, data }` 由消费方自己分派
//! 3. **握手**：新连接拿到一条 `system` 帧（`connected`）
//!
//! 发布方在本进程里只有一个：vdfs 宿主的变更投递（`crates/symbio-plugin-vdfs/src/host.rs`，一切资源与
//! 转写的实时变化都走它）。投递实现住 [`crate::bus`]（登记表 / 扇出 / 慢消费者处置 / resync
//! 补送，`D-487` 随订阅声明化从 sdk 随迁），经 crate 根的 `submit_event_bus_sink!` 声明进
//! sdk 的发布端口——sdk 侧只有线上契约与端口 trait，不持有订阅者登记表。本插件因此不是
//! 「按 `kind` 分流的路由器」——频道闭集、无服务端过滤，形状与判据见
//! `symbio_plugin_api::event_bus` 的模块文档。
//!
//! ## 用法
//!
//! `kind` 取 `symbio_plugin_api::event_bus::KIND_*` 常量（消费方按它分发，裸字面量改名不会
//! 编译失败）：
//!
//! ```ignore
//! // 在 vdfs 宿主中（一切资源的实时变更都走这一条）
//! EventBus::try_publish(EVENT_BUS_KIND_VDFS, None, change_json);
//! ```
//!
//! 订阅方拿到帧后按 `kind` 分派；`session_id` 只是 `system` 握手帧的关联信息，
//! **业务身份一律在载荷自己的地址里**（VDFS 变更的 `path`）——会话域的
//! `kind = "session"` 频道已废除，见 `docs/003-design/005-协议规范.md` §事件总线频道。
//!
//! ## `subscribe` 路由的请求契约
//!
//! 请求载荷是**空对象**（前端 `eventBus.ts` 传 `{}`、CLI 发空载荷、子智能体桥同款）。
//! 这里原本有一个 `kinds: Option<Vec<String>>`（「限定只接收某些 kind 的事件」），但
//! **没有任何生产者发它**：而后端也只是把它丢掉。一个「看起来能用、实际被静默忽略」
//! 的字段比没有更危险——下一个人会以为服务端过滤已生效，再去查「为什么事件还是全量
//! 到达」。故零字段请求类型（原 `EventBusSubscribeRequest`）随 `D-487` 撤销：契约由
//! `docs/003-design/005-协议规范.md` §事件总线频道与本注释承载，本路由忽略一切字段
//! （旧客户端发 `{"kinds":["vdfs"]}` 仍然成功，只是没有过滤效果）。
//!
//! 若将来真要做服务端过滤：`kind` 是**闭集**（`EVENT_BUS_KIND_SYSTEM` / `EVENT_BUS_KIND_VDFS`），
//! 按它过滤省不了多少流量。真正省流量的那一维是**路径**，而它已经实现了——`vdfs/watch`
//! 的登记表决定哪些变更进总线（未登记的路径根本不会走到这里）。
//!
//! 历史上的 `pending/snapshot` 路由已随会话事件频道一并废除：它缓冲的是按 `session_id`
//! 灌入的事件帧，而 VDFS 变更的发布方传 `session_id = None`（身份在地址里），缓冲永远
//! 为空——路由成了恒返回空数组的空壳。

use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use symbio_plugin_api::SimpleResponse;
use symbio_plugin_api::PLUGIN_ID_EVENT_BUS;
use symbio_plugin_api::{
    event_bus_build_envelope, EventBusPublisher, EVENT_BUS_KIND_SYSTEM, EVENT_BUS_SINK,
};
use symbio_plugin_api::{
    Plugin, PluginChannel, PluginError, PluginFrame, PluginInvokeRequest, PluginInvokeRequestExt,
    PluginInvokeResponse, PluginMeta, PluginPayload,
};
use uuid::Uuid;

use crate::bus;

/// Event Bus 插件
pub struct EventBusPlugin;

impl EventBusPlugin {
    /// 工厂方法（满足 `submit_object_creator!` 协议）
    pub fn build(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn Plugin> {
        Arc::new(EventBusPlugin) as Arc<dyn Plugin>
    }

    /// 发布端口工厂（`D-487`；满足 `submit_object_creator!` 协议）。
    ///
    /// `BusPublisher` 无状态（登记表是进程级静态），构造廉价——满足 creator 的隐含契约。
    pub fn build_publisher(_ctx: Arc<dyn PluginInvokeRequest>) -> Arc<dyn EventBusPublisher> {
        Arc::new(bus::BusPublisher) as Arc<dyn EventBusPublisher>
    }

    pub fn metadata() -> PluginMeta {
        PluginMeta::new(PLUGIN_ID_EVENT_BUS, "Event Bus")
            .with_description("统一事件总线：单连接订阅所有插件事件")
            .with_version("0.1.0")
    }

    /// 处理订阅请求（connect_v2）
    ///
    /// 分配一个 connection_id，建立 PluginChannel，
    /// 返回 peer_channel 供 transport 通过 mpsc 推送到前端。
    pub async fn handle_subscribe(
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        // 容量 4096。
        //
        // 这条频道承载**全部**实时面：消息正文的逐帧增量（`VdfsChange.delta`，热路径）
        // 与会话 / 资源变更（低频）共用它。原先的 2048 是「只有资源变更」时的取值，
        // 承接消息增量后偏小——满了虽不会丢（投递端会补 resync 指令，
        // 消费端重读作用域后自愈），但每次 resync 都换来一次整份重读，代价远大于
        // 多留 2048 个槽位。
        let (peer, mine) = PluginChannel::pair(4096);

        // 注册到本插件的订阅表（`D-487`：表归插件本体，sdk 不再持有）
        let connection_id = Uuid::new_v4().to_string();
        bus::register_subscriber(connection_id.clone(), mine.tx.clone());

        // 立即推送一个 connected 事件（先于清理任务 spawn：任务会捕获 `mine` 的字段）。
        //
        // 信封走 `event_bus_build_envelope`（形状的唯一构建入口），不自己拼 `json!`：
        // 手拼的那份在形状漂移时不会有任何编译错误。
        let _ = mine
            .tx
            .send(PluginFrame::data(event_bus_build_envelope(
                EVENT_BUS_KIND_SYSTEM,
                None,
                json!({
                    "event": "connected",
                    "connection_id": connection_id,
                }),
            )))
            .await;

        // 异步清理：连接结束（cancel_token 触发）**或**对端收件全断
        // （`tx.closed()`——子智能体桥按路由订阅、按轮摘除，不走 cancel）时反注册。
        let conn_id_for_cleanup = connection_id.clone();
        tokio::spawn(async move {
            // 只持有 rx，不消费
            let _rx = mine.rx;
            let tx = mine.tx;
            tokio::select! {
                // PluginChannel 的 cancel_token 会被断开时触发
                _ = mine.cancel_token.cancelled() => {}
                // 对端收件端全部 drop ⇒ 发送端再无用处，摘除防泄漏
                _ = tx.closed() => {}
            }
            bus::unregister_subscriber(&conn_id_for_cleanup);
        });

        Ok(PluginPayload::Session(peer))
    }
}

#[async_trait]
impl Plugin for EventBusPlugin {
    fn meta(&self) -> PluginMeta {
        Self::metadata()
    }

    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = ctx.get(symbio_plugin_api::PATH).unwrap_or_default();
        let path = path.strip_prefix('/').unwrap_or(&path);

        match path {
            // 建立进程内帧订阅连接（前端实时面与 CLI 都走它收会话帧）。
            // 请求契约（空对象，一切字段忽略）见本文件头「subscribe 路由的请求契约」。
            "subscribe" => {
                let _: serde_json::Value = ctx.payload()?;
                EventBusPlugin::handle_subscribe(ctx).await
            }
            // 存活探测（回一个 pong 与当前订阅数）。
            "ping" => {
                let resp = SimpleResponse::success_with_message(format!(
                    "pong ({} subscribers)",
                    bus::SUBSCRIBERS.len()
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
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Ok(PluginPayload::new(&Vec::<serde_json::Value>::new()))
    }
}

symbio_plugin_api::submit_object_creator!(PLUGIN_ID_EVENT_BUS, EventBusPlugin::build, dyn Plugin);
// 发布端口声明（`D-487`）：sdk 的 `EventBus::try_publish` 按 `EVENT_BUS_SINK` 装出
// 本实现，投到本插件的订阅表（creator 的第四个类型族）。
symbio_plugin_api::submit_object_creator!(
    EVENT_BUS_SINK,
    EventBusPlugin::build_publisher,
    dyn EventBusPublisher
);

#[cfg(test)]
#[path = "plugin.test.rs"]
mod tests;
