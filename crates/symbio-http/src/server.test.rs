//! 共享传输的边界与线上行为测试。

use super::*;

use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use async_trait::async_trait;
use serde_json::json;
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn request_with(headers: &[(&str, &str)]) -> HttpRequest {
    HttpRequest {
        method: "GET".to_string(),
        path: "/".to_string(),
        headers: headers
            .iter()
            .map(|(name, value)| (name.to_lowercase(), value.to_string()))
            .collect(),
        body: Vec::new(),
    }
}

fn config(token: &str, cors: bool) -> ServerConfig {
    ServerConfig {
        bind: "127.0.0.1".to_string(),
        port: 0,
        token: token.to_string(),
        max_connections: 64,
        cors,
        allowed_origin: None,
    }
}

struct TestIngress;

#[async_trait]
impl Ingress for TestIngress {
    fn is_public(&self, request: &HttpRequest) -> bool {
        request.method == "GET" && request.path.starts_with("/api/v1/health")
    }

    async fn handle(&self, request: HttpRequest, cancel: CancellationToken) -> IngressReply {
        if request.method == "GET" && request.path.starts_with("/api/v1/ws") {
            return IngressReply::Upgrade;
        }
        if request.method == "GET" && request.path.starts_with("/api/v1/health") {
            return IngressReply::Response(HttpResponse::json(200, json!({ "ok": true })));
        }
        if request.method == "GET" && request.path.starts_with("/api/v1/events") {
            return match self.subscribe().await {
                Ok(channel) => IngressReply::Events(channel),
                Err(error) => {
                    IngressReply::Response(HttpResponse::json(500, json!({ "error": error })))
                }
            };
        }
        if request.method == "POST" && request.path.starts_with("/api/v1/invoke") {
            return match serde_json::from_slice::<PluginMessageWire>(&request.body) {
                Ok(message) => match self.invoke(message, cancel).await {
                    Ok(payload) => match payload_response(payload).await {
                        Ok(response) => IngressReply::Response(response),
                        Err(error) => IngressReply::Response(HttpResponse::json(
                            400,
                            json!({ "error": error }),
                        )),
                    },
                    Err(error) => {
                        IngressReply::Response(HttpResponse::json(400, json!({ "error": error })))
                    }
                },
                Err(error) => IngressReply::Response(HttpResponse::json(
                    400,
                    json!({ "error": format!("invalid request: {error}") }),
                )),
            };
        }
        IngressReply::Response(HttpResponse::json(404, json!({ "error": "not found" })))
    }

    async fn invoke(
        &self,
        message: PluginMessageWire,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<PluginPayload, String> {
        let path = message
            .metadata
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("");
        match path {
            "once" => Ok(PluginPayload::new(&json!({ "ok": true }))),
            "empty" => Ok(PluginPayload::Empty),
            "stream" => {
                let (peer, mine) = PluginChannel::pair(4);
                mine.tx
                    .send(PluginFrame::data(json!({ "step": 1 })))
                    .await
                    .map_err(|error| error.to_string())?;
                mine.tx
                    .send(PluginFrame::data(json!({ "step": 2 })))
                    .await
                    .map_err(|error| error.to_string())?;
                Ok(PluginPayload::Session(peer))
            }
            "fail" => Err("动作 \"abort\" 不存在".to_string()),
            _ => Err("unknown test route".to_string()),
        }
    }
}

impl TestIngress {
    async fn subscribe(&self) -> Result<PluginChannel, String> {
        let (peer, mine) = PluginChannel::pair(4);
        mine.tx
            .send(PluginFrame::data(json!({
                "type": "bus_event",
                "data": {"kind": "system", "session_id": null, "data": {"event": "connected"}},
            })))
            .await
            .map_err(|error| error.to_string())?;
        Ok(peer)
    }
}

async fn start_server(token: &str, cors: bool) -> (ServerHandle, std::net::SocketAddr) {
    let handle = start(config(token, cors), Arc::new(TestIngress))
        .await
        .expect("loopback server should start");
    let address = handle.local_addr();
    (handle, address)
}

async fn get_once(address: std::net::SocketAddr, request: &str) -> String {
    let mut stream = TcpStream::connect(address)
        .await
        .expect("server should accept connection");
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

fn invoke_request(path: &str) -> String {
    format!(
        "POST /api/v1/invoke HTTP/1.1\r\nHost: probe\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
        json!({ "metadata": { "path": path }, "payload": {} }).to_string().len(),
        json!({ "metadata": { "path": path }, "payload": {} })
    )
}

#[test]
fn sha1_matches_known_vectors() {
    assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    assert_eq!(
        hex(&sha1(b"The quick brown fox jumps over the lazy dog")),
        "2fd4e1c67a2d28fced849ee1bb76e7391b93eb12"
    );
}

#[test]
fn websocket_accept_matches_rfc6455_example() {
    let key = "dGhlIHNhbXBsZSBub25jZQ==";
    let accept = base64::engine::general_purpose::STANDARD
        .encode(sha1(format!("{key}{WS_GUID}").as_bytes()));
    assert_eq!(accept, "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
}

#[test]
fn bearer_auth_requires_the_exact_configured_token() {
    assert!(check_auth("", &request_with(&[])));
    assert!(check_auth(
        "s3cret",
        &request_with(&[("authorization", "  Bearer s3cret  ")])
    ));
    assert!(!check_auth("s3cret", &request_with(&[])));
    assert!(!check_auth(
        "s3cret",
        &request_with(&[("authorization", "Bearer nope")])
    ));
    assert!(!check_auth(
        "s3cret",
        &request_with(&[("authorization", "Bearer s3cret-and-more")])
    ));
}

#[test]
fn non_loopback_without_token_is_rejected() {
    for bind in ["127.0.0.1", "localhost", "::1", "127.0.0.2"] {
        assert!(require_token_for_non_loopback(bind, "").is_ok());
    }
    for bind in ["0.0.0.0", "192.168.1.10", "::"] {
        assert!(require_token_for_non_loopback(bind, "").is_err());
    }
    assert!(require_token_for_non_loopback("0.0.0.0", "s3cret").is_ok());
}

#[tokio::test]
async fn start_checks_the_token_before_binding() {
    let mut config = config("", true);
    config.bind = "0.0.0.0".to_string();
    match start(config, Arc::new(TestIngress)).await {
        Err(error) => assert!(error.contains("必须设置访问令牌")),
        Ok(_) => panic!("non-loopback bind without token must fail"),
    }
}

#[test]
fn websocket_token_uses_authorization_then_subprotocol() {
    assert_eq!(
        ws_token(&request_with(&[("authorization", "Bearer s3cret")])).as_deref(),
        Some("s3cret")
    );
    assert_eq!(
        ws_token(&request_with(&[(
            "sec-websocket-protocol",
            "symbio.v1, bearer.s3cret"
        )]))
        .as_deref(),
        Some("s3cret")
    );
    assert_eq!(
        ws_token(&request_with(&[
            ("authorization", "Bearer good"),
            ("sec-websocket-protocol", "bearer.bad")
        ]))
        .as_deref(),
        Some("good")
    );
    assert_eq!(ws_token(&request_with(&[])), None);
}

#[test]
fn websocket_never_echoes_the_bearer_subprotocol() {
    assert_eq!(
        ws_select_subprotocol(&request_with(&[(
            "sec-websocket-protocol",
            "symbio.v1, bearer.s3cret"
        )])),
        Some("symbio.v1")
    );
    assert_eq!(
        ws_select_subprotocol(&request_with(&[(
            "sec-websocket-protocol",
            "bearer.s3cret"
        )])),
        None
    );
}

#[test]
fn connection_cap_clamps_to_the_supported_range() {
    assert_eq!(connection_cap(0), 1);
    assert_eq!(connection_cap(1), 1);
    assert_eq!(connection_cap(64), 64);
    assert_eq!(connection_cap(usize::MAX), MAX_CONNECTION_CAP);
}

#[test]
fn http_response_declares_length_and_cors() {
    let response = HttpResponse::json(200, json!({ "ok": true }));
    let wire = http_response(&response, Some("*"));
    let text = String::from_utf8(wire).expect("valid UTF-8 response");
    assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(text.contains("Content-Length: 11\r\n"));
    assert!(text.contains("Access-Control-Allow-Origin: *\r\n"));
    assert!(text.ends_with("{\"ok\":true}"));
}

#[test]
fn json_errors_escape_quotes() {
    let response = HttpResponse::json(400, json!({ "error": "动作 \"abort\" 不存在" }));
    let value: Value = serde_json::from_slice(&response.body).expect("valid JSON error");
    assert_eq!(value["error"], "动作 \"abort\" 不存在");
}

#[tokio::test]
async fn read_request_parses_line_headers_and_body() {
    let raw = "POST /api/v1/invoke?x=1 HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: 7\r\n\r\n{\"a\":1}";
    let request = read_request(&mut raw.as_bytes())
        .await
        .expect("valid request");
    assert_eq!(request.method, "POST");
    assert_eq!(request.path, "/api/v1/invoke?x=1");
    assert_eq!(
        request.headers.get("content-type").map(String::as_str),
        Some("application/json")
    );
    assert_eq!(request.body, b"{\"a\":1}");
}

#[tokio::test]
async fn read_request_rejects_oversized_headers() {
    let mut raw = b"GET / HTTP/1.1\r\nX-Big: ".to_vec();
    raw.extend(std::iter::repeat_n(b'a', MAX_HEADER_BYTES + 16));
    assert!(matches!(
        read_request(&mut raw.as_slice()).await,
        Err(RequestError::TooLarge(_))
    ));
}

#[tokio::test]
async fn read_request_rejects_oversized_body_before_reading_it() {
    let raw = format!(
        "POST /x HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
        MAX_BODY_BYTES + 1
    );
    assert!(matches!(
        read_request(&mut raw.as_bytes()).await,
        Err(RequestError::TooLarge(_))
    ));
}

#[tokio::test]
async fn incomplete_request_is_a_silent_close() {
    let mut reader = b"GET / HTTP/1.1\r\nHost: x\r\n".as_slice();
    assert!(matches!(
        read_request(&mut reader).await,
        Err(RequestError::Closed)
    ));
}

#[tokio::test]
async fn read_request_rejects_ambiguous_framing_and_duplicate_headers() {
    for raw in [
        "POST /x HTTP/1.1\r\nHost: a\r\nhost: b\r\nContent-Length: 0\r\n\r\n",
        "POST /x HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n",
        "POST /x HTTP/1.1\r\nContent-Length: nope\r\n\r\n",
        "POST /x HTTP/1.1\r\nContent-Length: +1\r\n\r\na",
        "POST /x HTTP/1.1\r\nContent-Length: 1, 1\r\n\r\na",
        "POST /x HTTP/1.1\r\nContent-Length: 999999999999999999999999\r\n\r\n",
        "POST /x HTTP/1.1\r\nBroken-Header\r\n\r\n",
        "POST /x HTTP/1.0\r\n\r\n",
    ] {
        assert!(
            matches!(
                read_request(&mut raw.as_bytes()).await,
                Err(RequestError::BadRequest(_))
            ),
            "must reject {raw:?}"
        );
    }
}

#[tokio::test]
async fn read_request_requires_the_declared_body_and_does_not_overread() {
    let incomplete = "POST /x HTTP/1.1\r\nContent-Length: 2\r\n\r\na";
    assert!(matches!(
        read_request(&mut incomplete.as_bytes()).await,
        Err(RequestError::Closed)
    ));

    let mut reader = "POST /x HTTP/1.1\r\nContent-Length: 1\r\n\r\naNEXT".as_bytes();
    let request = read_request(&mut reader).await.expect("complete body");
    assert_eq!(request.body, b"a");
    assert_eq!(reader, b"NEXT");
}

#[tokio::test]
async fn websocket_backpressure_send_observes_cancellation() {
    let (sender, mut receiver) = mpsc::channel::<PluginFrame>(1);
    sender
        .send(PluginFrame::data(json!({ "queued": true })))
        .await
        .expect("fill bounded channel");
    let cancel = CancellationToken::new();
    let sending = tokio::spawn({
        let cancel = cancel.clone();
        async move {
            send_ws_plugin_frame(
                &sender,
                PluginFrame::data(json!({ "blocked": true })),
                &cancel,
            )
            .await
        }
    });
    tokio::task::yield_now().await;
    cancel.cancel();
    assert!(!tokio::time::timeout(Duration::from_secs(1), sending)
        .await
        .expect("cancelled send must return")
        .expect("send task"));
    let received = receiver.recv().await.expect("original frame");
    assert_eq!(
        serde_json::to_value(received).expect("serialize frame"),
        serde_json::to_value(PluginFrame::data(json!({ "queued": true })))
            .expect("serialize expected frame")
    );
}

#[tokio::test]
async fn websocket_frame_decodes_masked_text() {
    let payload = b"hello";
    let mask = [0x12u8, 0x34, 0x56, 0x78];
    let mut frame = vec![0x81, 0x80 | payload.len() as u8];
    frame.extend_from_slice(&mask);
    frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    let (opcode, data) = ws_read_frame(&mut frame.as_slice())
        .await
        .expect("one frame");
    assert_eq!(opcode, 0x1);
    assert_eq!(data, payload);
}

#[tokio::test]
async fn websocket_frame_rejects_huge_length_before_allocation() {
    let mut frame = vec![0x81, 0x7F];
    frame.extend_from_slice(&(1u64 << 40).to_be_bytes());
    assert!(ws_read_frame(&mut frame.as_slice()).await.is_none());
}

#[tokio::test]
async fn websocket_frames_roundtrip_all_length_forms() {
    for length in [0usize, 125, 126, 65535, 65536] {
        let payload = vec![b'z'; length];
        let mut output = Vec::new();
        ws_send_frame(&mut output, 0x1, &payload)
            .await
            .expect("write frame");
        let (opcode, data) = ws_read_frame(&mut output.as_slice())
            .await
            .expect("read frame");
        assert_eq!(opcode, 0x1);
        assert_eq!(data, payload);
    }
}

#[tokio::test]
async fn stop_closes_the_listener() {
    let (handle, address) = start_server("", true).await;
    let response = get_once(
        address,
        "GET /api/v1/health HTTP/1.1\r\nHost: probe\r\n\r\n",
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 200 OK"));
    stop(handle).await;
    let refused = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if TcpStream::connect(address).await.is_err() {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await;
    assert!(refused.is_ok(), "listener still accepts after stop");
}

#[test]
fn sse_data_line_keeps_one_event_per_envelope() {
    let line = sse_data_line(&json!({
        "type": "bus_event",
        "data": {"kind": "vdfs", "session_id": null, "data": {"path": "/x"}},
    }));
    let text = String::from_utf8(line).expect("UTF-8 SSE event");
    assert!(text.starts_with("data: "));
    assert!(text.ends_with("\n\n"));
    let value: Value = serde_json::from_str(text.trim_start_matches("data: ").trim_end())
        .expect("valid event JSON");
    assert_eq!(value["data"]["kind"], "vdfs");
}

#[tokio::test]
async fn sse_endpoint_streams_the_envelope_verbatim() {
    let (handle, address) = start_server("", true).await;
    let response = get_once(
        address,
        "GET /api/v1/events HTTP/1.1\r\nHost: probe\r\n\r\n",
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(response.contains("Content-Type: text/event-stream"));
    assert!(response.contains("data: "));
    assert!(response.contains(r#""event":"connected""#));
    stop(handle).await;
}

#[tokio::test]
async fn sse_endpoint_requires_bearer_auth() {
    let (handle, address) = start_server("s3cret", true).await;
    let denied = get_once(
        address,
        "GET /api/v1/events HTTP/1.1\r\nHost: probe\r\n\r\n",
    )
    .await;
    assert!(denied.starts_with("HTTP/1.1 401 Unauthorized"));
    let allowed = get_once(
        address,
        "GET /api/v1/events HTTP/1.1\r\nHost: probe\r\nAuthorization: Bearer s3cret\r\n\r\n",
    )
    .await;
    assert!(allowed.starts_with("HTTP/1.1 200 OK"));
    stop(handle).await;
}

#[tokio::test]
async fn http_invoke_delivers_once_empty_and_last_stream_frame() {
    let (handle, address) = start_server("", true).await;
    for (path, expected) in [
        ("once", json!({ "type": "Data", "data": { "ok": true } })),
        ("empty", json!({ "type": "Data", "data": null })),
        ("stream", json!({ "type": "Data", "data": { "step": 2 } })),
    ] {
        let response = get_once(address, &invoke_request(path)).await;
        let (_, body) = response.split_once("\r\n\r\n").expect("HTTP body");
        let value: Value = serde_json::from_str(body).expect("valid invoke response");
        assert_eq!(value, expected, "route {path}");
    }
    stop(handle).await;
}

#[tokio::test]
async fn http_invoke_errors_are_valid_json() {
    let (handle, address) = start_server("", true).await;
    let response = get_once(address, &invoke_request("fail")).await;
    assert!(response.starts_with("HTTP/1.1 400 Bad Request"));
    let (_, body) = response.split_once("\r\n\r\n").expect("HTTP body");
    let value: Value = serde_json::from_str(body).expect("valid error JSON");
    assert_eq!(value["error"], "动作 \"abort\" 不存在");
    stop(handle).await;
}

async fn read_headers<R: AsyncRead + Unpin>(reader: &mut R) -> String {
    let mut bytes = Vec::new();
    loop {
        let mut byte = [0u8; 1];
        reader.read_exact(&mut byte).await.expect("read header");
        bytes.push(byte[0]);
        if bytes.ends_with(b"\r\n\r\n") {
            return String::from_utf8(bytes).expect("UTF-8 headers");
        }
    }
}

fn masked_frame(payload: &[u8]) -> Vec<u8> {
    let mask = [0x10u8, 0x20, 0x30, 0x40];
    let mut frame = vec![0x81, 0x80 | payload.len() as u8];
    frame.extend_from_slice(&mask);
    frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    frame
}

async fn read_server_frame<R: AsyncRead + Unpin>(reader: &mut R) -> String {
    let (_, payload) = ws_read_frame(reader).await.expect("server frame");
    String::from_utf8(payload).expect("UTF-8 frame")
}

#[tokio::test]
async fn websocket_invoke_errors_are_safely_serialized() {
    let (handle, address) = start_server("s3cret", true).await;
    let mut stream = TcpStream::connect(address).await.expect("connect");
    stream
        .write_all(
            b"GET /api/v1/ws HTTP/1.1\r\nHost: probe\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Protocol: symbio.v1, bearer.s3cret\r\n\r\n",
        )
        .await
        .expect("handshake request");
    let headers = read_headers(&mut stream).await;
    assert!(headers.starts_with("HTTP/1.1 101 Switching Protocols"));
    assert!(headers.contains("Sec-WebSocket-Protocol: symbio.v1\r\n"));
    assert!(!headers.contains("bearer.s3cret"));

    let message = json!({ "metadata": { "path": "fail" }, "payload": {} }).to_string();
    stream
        .write_all(&masked_frame(message.as_bytes()))
        .await
        .expect("first frame");
    let text = read_server_frame(&mut stream).await;
    let value: Value = serde_json::from_str(&text).expect("valid WS error JSON");
    assert_eq!(value["Error"][0], "动作 \"abort\" 不存在");
    stop(handle).await;
}

#[tokio::test]
async fn loopback_rejects_host_rebinding_and_cross_origin_requests() {
    let (handle, address) = start_server("", false).await;
    for request in [
        "GET /api/v1/health HTTP/1.1\r\nHost: attacker.example\r\n\r\n",
        "GET /api/v1/health HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: https://attacker.example\r\n\r\n",
    ] {
        let response = get_once(address, request).await;
        assert!(response.starts_with("HTTP/1.1 403 Forbidden"));
    }
    let allowed = get_once(
        address,
        "GET /api/v1/health HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: http://127.0.0.1\r\n\r\n",
    )
    .await;
    assert!(allowed.starts_with("HTTP/1.1 200 OK"));
    stop(handle).await;
}

#[tokio::test]
async fn explicit_origin_requires_a_token_and_exact_origin_match() {
    let mut untrusted = config("", false);
    untrusted.allowed_origin = Some("https://console.example".to_string());
    assert!(start(untrusted, Arc::new(TestIngress)).await.is_err());

    let mut config = config("s3cret", false);
    config.allowed_origin = Some("https://console.example".to_string());
    let handle = start(config, Arc::new(TestIngress))
        .await
        .expect("explicit origin server");
    let address = handle.local_addr();
    let denied = get_once(
        address,
        "GET /api/v1/events HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: https://console.example\r\n\r\n",
    )
    .await;
    assert!(denied.starts_with("HTTP/1.1 401 Unauthorized"));
    let wrong_origin = get_once(
        address,
        "GET /api/v1/health HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: https://other.example\r\nAuthorization: Bearer s3cret\r\n\r\n",
    )
    .await;
    assert!(wrong_origin.starts_with("HTTP/1.1 403 Forbidden"));
    let allowed = get_once(
        address,
        "GET /api/v1/events HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: https://console.example\r\nAuthorization: Bearer s3cret\r\n\r\n",
    )
    .await;
    assert!(allowed.starts_with("HTTP/1.1 200 OK"));
    assert!(allowed.contains("Access-Control-Allow-Origin: https://console.example\r\n"));
    stop(handle).await;
}

#[tokio::test]
async fn websocket_cross_origin_is_rejected_before_upgrade() {
    let (handle, address) = start_server("", false).await;
    let response = get_once(
        address,
        "GET /api/v1/ws HTTP/1.1\r\nHost: 127.0.0.1\r\nOrigin: https://attacker.example\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\r\n",
    )
    .await;
    assert!(response.starts_with("HTTP/1.1 403 Forbidden"));
    assert!(!response.starts_with("HTTP/1.1 101 Switching Protocols"));
    stop(handle).await;
}

#[tokio::test(start_paused = true)]
async fn incomplete_request_times_out_without_holding_a_connection_forever() {
    let (handle, address) = start_server("", true).await;
    let mut stream = TcpStream::connect(address).await.expect("connect");
    stream
        .write_all(b"GET /api/v1/health HTTP/1.1\r\nHost: probe\r\n")
        .await
        .expect("partial request");
    tokio::time::advance(REQUEST_TIMEOUT + Duration::from_secs(1)).await;
    let mut response = Vec::new();
    stream
        .read_to_end(&mut response)
        .await
        .expect("timeout response");
    assert!(String::from_utf8_lossy(&response).starts_with("HTTP/1.1 408 Request Timeout"));
    stop(handle).await;
}

struct StalledWriter;

impl AsyncWrite for StalledWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
        _buffer: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Poll::Pending
    }

    fn poll_flush(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[tokio::test(start_paused = true)]
async fn stalled_response_write_times_out() {
    let result = write_response(
        &mut StalledWriter,
        &HttpResponse::json(200, json!({ "ok": true })),
        None,
    )
    .await;
    assert_eq!(
        result.expect_err("stalled write").kind(),
        std::io::ErrorKind::TimedOut
    );
}

struct ObservedIngress {
    sender: Mutex<Option<mpsc::Sender<PluginFrame>>>,
    closed: AtomicUsize,
}

#[async_trait]
impl Ingress for ObservedIngress {
    async fn handle(&self, _request: HttpRequest, _cancel: CancellationToken) -> IngressReply {
        let (peer, mine) = PluginChannel::pair(1);
        *self.sender.lock().unwrap() = Some(mine.tx);
        IngressReply::Events(peer)
    }

    async fn invoke(
        &self,
        _message: PluginMessageWire,
        _cancel: CancellationToken,
    ) -> Result<PluginPayload, String> {
        Ok(PluginPayload::Empty)
    }

    async fn realtime_closed(&self, _cancel: CancellationToken) {
        self.closed.fetch_add(1, Ordering::AcqRel);
    }
}

#[tokio::test]
async fn stop_drains_realtime_connections_and_notifies_ingress() {
    let ingress = Arc::new(ObservedIngress {
        sender: Mutex::new(None),
        closed: AtomicUsize::new(0),
    });
    let handle = start(config("", true), ingress.clone())
        .await
        .expect("server");
    let mut stream = TcpStream::connect(handle.local_addr())
        .await
        .expect("connect");
    stream
        .write_all(b"GET /api/v1/events HTTP/1.1\r\nHost: probe\r\n\r\n")
        .await
        .expect("SSE request");
    let headers = read_headers(&mut stream).await;
    assert!(headers.starts_with("HTTP/1.1 200 OK"));
    stop(handle).await;
    assert_eq!(ingress.closed.load(Ordering::Acquire), 1);
}
