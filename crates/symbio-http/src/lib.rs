//! Symbio 共享入站传输：HTTP/1.1、WebSocket 与 SSE。
//!
//! 本 crate 只认识线路协议与 [`Ingress`] 端口，不依赖 gateway、server 或宿主实现。
//! 路由策略、调用上下文与订阅来源都由 ingress 实现决定。

use async_trait::async_trait;
use serde::Serialize;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

pub use symbio_kernel::plugin::PluginPayloadWire;
use symbio_plugin_api::{PluginChannel, PluginMessageWire, PluginPayload};

mod server;

pub use server::{start, stop, ServerHandle};
pub use tokio_util::sync::CancellationToken;

/// 将插件载荷按一次性 HTTP 语义交付。
///
/// `Data` / `Empty` 立即成为 JSON 响应；`Session` 消费到 EOF（单帧等待 30 秒），
/// 保留最后一帧数据并主动取消订阅。WebSocket 与 SSE 使用同一份载荷分类。
pub use server::payload_response;

/// 入站监听配置。
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind: String,
    pub port: u16,
    pub token: String,
    pub max_connections: usize,
    pub cors: bool,
    /// 显式允许的浏览器 Origin；`None` 时只允许同源，`cors = true` 保留 Gateway 的通配策略。
    pub allowed_origin: Option<String>,
}

/// 已解析的 HTTP 请求。头名统一为小写。
#[derive(Debug, Clone)]
pub struct HttpRequest {
    pub method: String,
    pub path: String,
    pub headers: HashMap<String, String>,
    pub body: Vec<u8>,
}

impl HttpRequest {
    pub fn is_json(&self) -> bool {
        self.headers
            .get("content-type")
            .and_then(|value| value.split(';').next())
            .is_some_and(|media_type| media_type.trim().eq_ignore_ascii_case("application/json"))
    }
}

/// 由 ingress 返回的 HTTP 响应。
#[derive(Debug, Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub content_type: String,
    pub body: Vec<u8>,
    pub headers: Vec<(String, String)>,
}

impl HttpResponse {
    /// JSON 响应；序列化失败时仍返回合法 JSON，绝不写出半截文档。
    pub fn json(status: u16, value: impl Serialize) -> Self {
        let body = serde_json::to_vec(&value)
            .unwrap_or_else(|_| br#"{"error":"response serialization failed"}"#.to_vec());
        Self {
            status,
            content_type: "application/json".to_string(),
            body,
            headers: Vec::new(),
        }
    }

    pub fn text(status: u16, value: impl Into<String>) -> Self {
        Self {
            status,
            content_type: "text/plain; charset=utf-8".to_string(),
            body: value.into().into_bytes(),
            headers: Vec::new(),
        }
    }

    pub fn bytes(status: u16, value: impl Into<Vec<u8>>) -> Self {
        Self {
            status,
            content_type: "application/octet-stream".to_string(),
            body: value.into(),
            headers: Vec::new(),
        }
    }
}

/// 入站路由与调用策略的宿主端口。
#[async_trait]
pub trait Ingress: Send + Sync + 'static {
    /// 无需 Bearer 鉴权的请求（例如健康检查）。
    fn is_public(&self, _request: &HttpRequest) -> bool {
        false
    }

    /// 处理普通 HTTP 请求，或声明实时通道 / WebSocket 升级。
    async fn handle(
        &self,
        request: HttpRequest,
        cancel: tokio_util::sync::CancellationToken,
    ) -> IngressReply;

    /// 执行一次插件调用。HTTP 与 WebSocket 共用同一入口。
    async fn invoke(
        &self,
        message: PluginMessageWire,
        cancel: tokio_util::sync::CancellationToken,
    ) -> Result<PluginPayload, String>;

    /// 实时通道退出后通知 ingress，供宿主在热重载前 drain 所有浏览器连接。
    async fn realtime_closed(&self, _cancel: tokio_util::sync::CancellationToken) {}
}

/// 运行时调用分发桥：让入站 transport 不必持有具体宿主即可走同一条 worker / 控制面路由。
#[async_trait]
pub trait InvokeDispatcher: Send + Sync {
    /// 只接管归 worker 的会话调用；返回 `None` 让 ingress 保留自己的控制面边界。
    async fn invoke_session(
        &self,
        _message: PluginMessageWire,
    ) -> Option<Result<PluginPayload, String>> {
        None
    }
}

static INVOKE_DISPATCHER: RwLock<Option<Arc<dyn InvokeDispatcher>>> = RwLock::new(None);

pub fn set_invoke_dispatcher(dispatcher: Arc<dyn InvokeDispatcher>) {
    *INVOKE_DISPATCHER
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(dispatcher);
}

pub fn clear_invoke_dispatcher() {
    *INVOKE_DISPATCHER
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}

/// `None` = 该分发器不接管这条会话调用；调用方保留自己的控制面边界。
pub async fn dispatch_session_invoke(
    message: PluginMessageWire,
) -> Option<Result<PluginPayload, String>> {
    let dispatcher = INVOKE_DISPATCHER
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()?;
    dispatcher.invoke_session(message).await
}

/// 普通 HTTP 请求的处理结果。
#[derive(Debug)]
pub enum IngressReply {
    Response(HttpResponse),
    Events(PluginChannel),
    Upgrade,
}
