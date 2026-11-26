//! Gateway 入站策略测试；共享线路机制住在 `symbio-http` 的测试中。

use super::*;

use serde_json::json;
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;
use symbio_plugin_api::{PluginError, PluginFrame, PluginInvokeResponse, PluginMeta};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[test]
fn build_ctx_marks_untrusted_gateway_calls_as_model_and_overwrites_forged_metadata() {
    let mut message = PluginMessageWire {
        metadata: json!({}),
        payload: json!({}),
    };
    for caller in ["model", "manager"] {
        message.metadata[VDFS_CALLER.name()] = Value::String(caller.into());
        let context = build_ctx(&message);
        assert_eq!(context.get(VDFS_CALLER), Some(VdfsCaller::Model));
    }
}

struct NopRouter;

#[async_trait]
impl Plugin for NopRouter {
    fn meta(&self) -> PluginMeta {
        PluginMeta::new("nop-router", "nop")
    }

    async fn route(
        self: Arc<Self>,
        _context: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Err(PluginError::NotFound("nop".to_string()))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _context: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Ok(PluginPayload::new(&Vec::<Value>::new()))
    }
}

#[tokio::test]
async fn gateway_start_refuses_non_loopback_without_token() {
    let config = GatewayConfig {
        inbound_enabled: true,
        inbound_protocol: "http".into(),
        inbound_bind: "0.0.0.0".into(),
        inbound_port: 0,
        inbound_token: String::new(),
        ..Default::default()
    };
    match start(&config, Arc::new(NopRouter)).await {
        Err(error) => assert!(error.contains("必须设置访问令牌")),
        Ok(_) => panic!("non-loopback bind without token must fail"),
    }
}

struct RecordingRouter {
    calls: Mutex<Vec<(String, Option<VdfsCaller>)>>,
}

impl RecordingRouter {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            calls: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl Plugin for RecordingRouter {
    fn meta(&self) -> PluginMeta {
        PluginMeta::new("recording-router", "recording")
    }

    async fn route(
        self: Arc<Self>,
        context: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = context.get(PATH).unwrap_or_default();
        let caller = context.get(VDFS_CALLER);
        self.calls
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push((path, caller));
        Ok(PluginPayload::new(&json!({ "ok": true })))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _context: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Ok(PluginPayload::new(&Vec::<Value>::new()))
    }
}

#[tokio::test]
async fn invoke_enforces_readonly_and_marks_model_caller() {
    let router = RecordingRouter::new();
    let ingress = GatewayIngress::new(router.clone(), true);
    let message = |path: &str| PluginMessageWire {
        metadata: json!({
            "path": path,
            VDFS_CALLER.name(): "manager",
        }),
        payload: json!({}),
    };

    assert!(ingress
        .invoke(message("vdfs/write"), CancellationToken::new())
        .await
        .is_err());
    assert!(router
        .calls
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .is_empty());
    assert!(ingress
        .invoke(message("vdfs/list"), CancellationToken::new())
        .await
        .is_ok());
    let calls = router
        .calls
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "vdfs/list");
    assert_eq!(calls[0].1, Some(VdfsCaller::Model));
}

#[tokio::test]
async fn http_invoke_reserves_idempotency_before_dispatch() {
    let router = RecordingRouter::new();
    let ingress = GatewayIngress::new(router.clone(), false);
    let request = |key: &str| HttpRequest {
        method: "POST".into(),
        path: "/api/v1/invoke".into(),
        headers: HashMap::from([
            ("content-type".into(), "application/json".into()),
            ("idempotency-key".into(), key.into()),
        ]),
        body: serde_json::to_vec(&PluginMessageWire {
            metadata: json!({ "path": "vdfs/list" }),
            payload: json!({}),
        })
        .unwrap(),
    };

    assert_eq!(
        ingress
            .handle_invoke(request("same"), CancellationToken::new())
            .await
            .status,
        200
    );
    assert_eq!(
        ingress
            .handle_invoke(request("same"), CancellationToken::new())
            .await
            .status,
        409
    );
    assert_eq!(
        router
            .calls
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .len(),
        1
    );
    assert_eq!(
        ingress
            .handle_invoke(request("bad key"), CancellationToken::new())
            .await
            .status,
        400
    );
    let mut non_json = request("not-reserved");
    non_json.headers.remove("idempotency-key");
    non_json
        .headers
        .insert("content-type".into(), "text/plain".into());
    assert_eq!(
        ingress
            .handle_invoke(non_json, CancellationToken::new())
            .await
            .status,
        415
    );
}

struct OneFrameRouter;

#[async_trait]
impl Plugin for OneFrameRouter {
    fn meta(&self) -> PluginMeta {
        PluginMeta::new("one-frame-router", "one")
    }

    async fn route(
        self: Arc<Self>,
        context: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let path = context.get(PATH).unwrap_or_default();
        if path != ROUTE_EVENT_BUS_SUBSCRIBE {
            return Err(PluginError::NotFound(path));
        }
        if context.get(VDFS_CALLER) != Some(VdfsCaller::Model) {
            return Err(PluginError::InternalError(
                "event_bus caller must be Model".into(),
            ));
        }
        let (peer, mine) = PluginChannel::pair(4);
        mine.tx
            .send(PluginFrame::data(json!({
                "type": "bus_event",
                "data": {"kind": "system", "session_id": null, "data": {"event": "connected"}},
            })))
            .await
            .map_err(|error| PluginError::InternalError(error.to_string()))?;
        Ok(PluginPayload::Session(peer))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _context: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Ok(PluginPayload::new(&Vec::<Value>::new()))
    }
}

async fn start_gateway(readonly: bool, token: &str) -> (ServerHandle, std::net::SocketAddr) {
    let config = GatewayConfig {
        inbound_enabled: true,
        inbound_protocol: "http".into(),
        inbound_bind: "127.0.0.1".into(),
        inbound_port: 0,
        inbound_token: token.to_string(),
        inbound_readonly: readonly,
        ..Default::default()
    };
    let handle = start(&config, Arc::new(OneFrameRouter))
        .await
        .expect("start gateway")
        .expect("HTTP listener");
    let address = handle.local_addr();
    (handle, address)
}

async fn get_once(address: std::net::SocketAddr, request: &str) -> String {
    let mut stream = TcpStream::connect(address).await.expect("connect gateway");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("write request");
    let mut buffer = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut buffer))
        .await
        .expect("response should finish")
        .expect("read response");
    String::from_utf8_lossy(&buffer).into_owned()
}

#[tokio::test]
async fn health_is_public_even_when_gateway_token_is_configured() {
    let (handle, address) = start_gateway(false, "s3cret").await;
    let response = get_once(
        address,
        "GET /api/v1/health HTTP/1.1\r\nHost: probe\r\n\r\n",
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 200 OK"));
    assert!(response.ends_with("{\"ok\":true}"));
    stop(handle).await;
}

#[tokio::test]
async fn readonly_mode_forbids_sse_and_normal_mode_uses_model_subscription() {
    let (readonly_handle, readonly_address) = start_gateway(true, "").await;
    let denied = get_once(
        readonly_address,
        "GET /api/v1/events HTTP/1.1\r\nHost: probe\r\n\r\n",
    )
    .await;
    assert!(denied.starts_with("HTTP/1.1 403 Forbidden"));
    assert!(denied.contains("只读模式下不开放实时订阅"));
    stop(readonly_handle).await;

    let (handle, address) = start_gateway(false, "").await;
    let allowed = get_once(
        address,
        "GET /api/v1/events HTTP/1.1\r\nHost: probe\r\n\r\n",
    )
    .await;
    assert!(allowed.starts_with("HTTP/1.1 200 OK"));
    assert!(allowed.contains(r#""event":"connected""#));
    stop(handle).await;
}
