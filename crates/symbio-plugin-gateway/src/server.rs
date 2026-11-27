//! Gateway 入站策略：只读白名单、Model 调用上下文、幂等与事件总线订阅。
//!
//! HTTP/1.1、WebSocket、SSE、鉴权和大小限制由 `symbio-http` 提供；本模块只实现
//! 该传输的 [`Ingress`] 端口，不复制线路机制。

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use symbio_http::{
    ingress_policy::{is_readonly_allowed, reserve_idempotency, Idempotency},
    payload_response, start as start_http, stop as stop_http, CancellationToken, HttpRequest,
    HttpResponse, Ingress, IngressReply, ServerConfig,
};
use symbio_plugin_api::routes::ROUTE_EVENT_BUS_SUBSCRIBE;
use symbio_plugin_api::{
    Plugin, PluginChannel, PluginInvokeRequest, PluginInvokeRequestExt, PluginMessageWire,
    PluginPayload, PluginSimpleRequest, SymbioKey, VdfsCaller, PATH, TRACE_ID, VDFS_CALLER,
};
use tracing::{debug, error, warn};

use super::config::GatewayConfig;

pub use symbio_http::ServerHandle;

/// 按 gateway 配置启动共享传输；未启用或非 HTTP 协议时不监听。
pub async fn start(
    config: &GatewayConfig,
    parent: Arc<dyn Plugin>,
) -> Result<Option<ServerHandle>, String> {
    if !config.inbound_enabled || config.inbound_protocol != "http" {
        return Ok(None);
    }
    let ingress = Arc::new(GatewayIngress::new(parent, config.inbound_readonly));
    start_http(
        ServerConfig {
            bind: config.inbound_bind.clone(),
            port: config.inbound_port,
            token: config.inbound_token.clone(),
            max_connections: config.inbound_max_connections,
            cors: true,
            allowed_origin: None,
        },
        ingress,
    )
    .await
    .map(Some)
}

pub async fn stop(handle: ServerHandle) {
    stop_http(handle).await;
}

struct GatewayIngress {
    parent: Arc<dyn Plugin>,
    readonly: bool,
    idempotency: Arc<Idempotency>,
}

impl GatewayIngress {
    fn new(parent: Arc<dyn Plugin>, readonly: bool) -> Self {
        Self {
            parent,
            readonly,
            idempotency: Idempotency::new(),
        }
    }
}

#[async_trait]
impl Ingress for GatewayIngress {
    fn is_public(&self, request: &HttpRequest) -> bool {
        if request.method == "GET" && request.path.starts_with("/api/v1/health") {
            return true;
        }
        // 未匹配路径历来直接回 404；保持该行为，不先用 token 把它改成 401。
        !(request.method == "POST" && request.path.starts_with("/api/v1/invoke")
            || request.method == "GET" && request.path.starts_with("/api/v1/events"))
    }

    async fn handle(&self, request: HttpRequest, _cancel: CancellationToken) -> IngressReply {
        if request.method == "GET" && request.path.starts_with("/api/v1/health") {
            return IngressReply::Response(HttpResponse::json(
                200,
                serde_json::json!({ "ok": true }),
            ));
        }

        if request.method == "GET" && request.path.starts_with("/api/v1/ws") {
            let upgraded = request
                .headers
                .get("upgrade")
                .is_some_and(|value| value.eq_ignore_ascii_case("websocket"));
            if !upgraded {
                return IngressReply::Response(HttpResponse::json(
                    400,
                    serde_json::json!({ "error": "WebSocket upgrade required" }),
                ));
            }
            return IngressReply::Upgrade;
        }

        if request.method == "GET" && request.path.starts_with("/api/v1/events") {
            if self.readonly {
                return IngressReply::Response(HttpResponse::json(
                    403,
                    serde_json::json!({ "error": "只读模式下不开放实时订阅" }),
                ));
            }
            return match self.subscribe().await {
                Ok(channel) => IngressReply::Events(channel),
                Err(error) => IngressReply::Response(HttpResponse::json(
                    500,
                    serde_json::json!({ "error": error }),
                )),
            };
        }

        if request.method == "POST" && request.path.starts_with("/api/v1/invoke") {
            return IngressReply::Response(self.handle_invoke(request, _cancel).await);
        }

        IngressReply::Response(HttpResponse::json(
            404,
            serde_json::json!({ "error": "not found" }),
        ))
    }

    async fn invoke(
        &self,
        message: PluginMessageWire,
        _cancel: CancellationToken,
    ) -> Result<PluginPayload, String> {
        let path = message
            .metadata
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("");
        let trace_id = message
            .metadata
            .get(TRACE_ID.name())
            .and_then(Value::as_str)
            .unwrap_or("-");
        if self.readonly && !is_readonly_allowed(path, &message.payload) {
            warn!(trace_id = %trace_id, path = %path, "只读模式下拒绝调用");
            return Err(format!("只读模式下禁止调用: {path}"));
        }

        debug!(trace_id = %trace_id, path = %path, "网关路由");
        if let Some(result) =
            symbio_http::dispatch_session_invoke(message.clone(), VdfsCaller::Model).await
        {
            return result.map_err(|error| {
                error!(trace_id = %trace_id, path = %path, error = %error, "运行时分发失败");
                error
            });
        }
        // 没有 RuntimeHost 分发器时（独立 ingress 单测）仍走构造期注入的插件树。
        let context = build_ctx(&message);
        self.parent.clone().route(context).await.map_err(|error| {
            error!(trace_id = %trace_id, path = %path, error = %error, "网关路由失败");
            error.to_string()
        })
    }
}

impl GatewayIngress {
    async fn subscribe(&self) -> Result<PluginChannel, String> {
        let context = Arc::new(PluginSimpleRequest::new(None, None));
        context.set(VDFS_CALLER, VdfsCaller::Model);
        context.set(PATH, ROUTE_EVENT_BUS_SUBSCRIBE.to_string());
        context
            .set_payload(serde_json::json!({}))
            .map_err(|error| error.to_string())?;
        match self.parent.clone().route(context).await {
            Ok(PluginPayload::Session(channel)) => Ok(channel),
            Ok(_) => Err("event_bus/subscribe 返回了非会话载荷".to_string()),
            Err(error) => Err(error.to_string()),
        }
    }

    async fn handle_invoke(&self, request: HttpRequest, cancel: CancellationToken) -> HttpResponse {
        if !request.is_json() {
            return HttpResponse::json(
                415,
                serde_json::json!({ "error": "Content-Type must be application/json" }),
            );
        }
        let ticket = match reserve_idempotency(&request, &self.idempotency) {
            Ok(ticket) => ticket,
            Err(response) => return response,
        };

        let message: PluginMessageWire = match serde_json::from_slice(&request.body) {
            Ok(message) => message,
            Err(error) => {
                if let Some(ticket) = &ticket {
                    ticket.release();
                }
                return HttpResponse::json(
                    400,
                    serde_json::json!({ "error": format!("invalid request: {error}") }),
                );
            }
        };

        let payload = match self.invoke(message, cancel).await {
            Ok(payload) => payload,
            Err(error) => {
                if let Some(ticket) = &ticket {
                    ticket.release();
                }
                return HttpResponse::json(400, serde_json::json!({ "error": error }));
            }
        };
        match payload_response(payload).await {
            Ok(response) => response,
            Err(error) => {
                if let Some(ticket) = &ticket {
                    ticket.release();
                }
                HttpResponse::json(400, serde_json::json!({ "error": error }))
            }
        }
    }
}

/// 由线路消息构造与桌面调用一致的上下文，并覆盖客户端伪造的 caller。
fn build_ctx(message: &PluginMessageWire) -> Arc<dyn PluginInvokeRequest> {
    let context = Arc::new(PluginSimpleRequest::from_parts(
        message.payload.clone(),
        message.metadata.as_object(),
    ));
    context.set(VDFS_CALLER, VdfsCaller::Model);
    context
}

#[cfg(test)]
#[path = "server.test.rs"]
mod tests;
