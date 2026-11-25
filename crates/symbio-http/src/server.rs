//! 手写 HTTP/1.1 / WebSocket / SSE 传输实现。

use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine;
use serde_json::Value;
use symbio_plugin_api::{PluginChannel, PluginFrame, PluginMessageWire, PluginPayload};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::{HttpRequest, HttpResponse, Ingress, IngressReply, PluginPayloadWire, ServerConfig};

const MAX_HEADER_BYTES: usize = 64 * 1024;
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
const MAX_WS_FRAME_BYTES: usize = 8 * 1024 * 1024;
const SSE_HEARTBEAT: Duration = Duration::from_secs(15);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const WRITE_TIMEOUT: Duration = Duration::from_secs(15);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_CONNECTION_CAP: usize = 1024;
const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

type ConnectionRegistry = Arc<Mutex<HashMap<u64, tokio::task::JoinHandle<()>>>>;

pub struct ServerHandle {
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<()>,
    connections: ConnectionRegistry,
    addr: std::net::SocketAddr,
}

impl ServerHandle {
    /// 实际监听地址；端口配置为 0 时返回内核分配的端口。
    pub fn local_addr(&self) -> std::net::SocketAddr {
        self.addr
    }
}

/// 停止接受新连接，取消并等待所有在途连接退出。
pub async fn stop(handle: ServerHandle) {
    handle.cancel.cancel();
    handle.task.abort();
    match handle.task.await {
        Ok(()) => {}
        Err(error) if !error.is_cancelled() => {
            eprintln!("HTTP server task shutdown failed: {error}");
        }
        Err(_) => {}
    }
    let connections =
        std::mem::take(&mut *handle.connections.lock().unwrap_or_else(|e| e.into_inner()));
    let mut tasks = JoinSet::new();
    for connection in connections.into_values() {
        tasks.spawn(connection);
    }
    if tokio::time::timeout(SHUTDOWN_TIMEOUT, async {
        while tasks.join_next().await.is_some() {}
    })
    .await
    .is_err()
    {
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
    }
}

pub async fn start(
    config: ServerConfig,
    ingress: Arc<dyn Ingress>,
) -> Result<ServerHandle, String> {
    require_token_for_non_loopback(&config.bind, &config.token)?;
    if config.allowed_origin.is_some() && config.token.is_empty() {
        return Err("显式允许跨源 Origin 时必须设置访问令牌".to_string());
    }
    if config
        .allowed_origin
        .as_deref()
        .is_some_and(|origin| !valid_origin(origin))
    {
        return Err("allowed_origin 必须是 http(s)://host[:port] 形式的精确 Origin".to_string());
    }
    let addr = (config.bind.as_str(), config.port);
    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| format!("监听失败 {}:{}: {e}", config.bind, config.port))?;
    let local = listener
        .local_addr()
        .map_err(|e| format!("读取监听地址失败 {}:{}: {e}", config.bind, config.port))?;

    let cancel = CancellationToken::new();
    let config = Arc::new(config);
    let permits = Arc::new(Semaphore::new(connection_cap(config.max_connections)));
    let connections: ConnectionRegistry = Arc::new(Mutex::new(HashMap::new()));
    let next_connection = Arc::new(AtomicU64::new(1));
    let task = tokio::spawn({
        let cancel = cancel.clone();
        let config = Arc::clone(&config);
        let ingress = Arc::clone(&ingress);
        let connections = Arc::clone(&connections);
        let next_connection = Arc::clone(&next_connection);
        async move {
            loop {
                let permit = tokio::select! {
                    _ = cancel.cancelled() => break,
                    permit = permits.clone().acquire_owned() => match permit {
                        Ok(permit) => permit,
                        Err(_) => break,
                    },
                };
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    accepted = listener.accept() => {
                        match accepted {
                            Ok((stream, _)) => {
                                let id = next_connection.fetch_add(1, Ordering::Relaxed);
                                let connections_for_task = Arc::clone(&connections);
                                let mut registry = connections.lock().unwrap_or_else(|e| e.into_inner());
                                let ingress_for_connection = Arc::clone(&ingress);
                                let config_for_connection = Arc::clone(&config);
                                let cancel_for_connection = cancel.clone();
                                let connection = tokio::spawn(async move {
                                    handle_conn(
                                        stream,
                                        ingress_for_connection,
                                        config_for_connection,
                                        cancel_for_connection,
                                        permit,
                                    )
                                    .await;
                                    connections_for_task
                                        .lock()
                                        .unwrap_or_else(|e| e.into_inner())
                                        .remove(&id);
                                });
                                registry.insert(id, connection);
                            }
                            Err(error) => warn!(%error, "[http] accept 失败"),
                        }
                    }
                }
            }
            info!("[http] 入站服务已停止");
        }
    });

    Ok(ServerHandle {
        cancel,
        task,
        connections,
        addr: local,
    })
}

fn require_token_for_non_loopback(bind: &str, token: &str) -> Result<(), String> {
    let is_loopback = bind.eq_ignore_ascii_case("localhost")
        || bind
            .parse::<IpAddr>()
            .map(|address| address.is_loopback())
            .unwrap_or(false);
    if !is_loopback && token.is_empty() {
        return Err("绑定非回环地址时必须设置访问令牌".to_string());
    }
    Ok(())
}

fn connection_cap(configured: usize) -> usize {
    configured.clamp(1, MAX_CONNECTION_CAP)
}

fn check_auth(token: &str, request: &HttpRequest) -> bool {
    token.is_empty()
        || request
            .headers
            .get("authorization")
            .map(|value| value.trim() == format!("Bearer {token}"))
            .unwrap_or(false)
}

fn cors_origin(config: &ServerConfig) -> Option<&str> {
    config
        .allowed_origin
        .as_deref()
        .or_else(|| config.cors.then_some("*"))
}

fn valid_origin(origin: &str) -> bool {
    let Some((scheme, authority)) = origin.split_once("://") else {
        return false;
    };
    (scheme == "http" || scheme == "https")
        && !authority.is_empty()
        && !authority.contains(['/', '?', '#', '@'])
}

fn host_without_port(host: &str) -> &str {
    host.rsplit_once(':')
        .filter(|(_, port)| port.parse::<u16>().is_ok())
        .map_or(host, |(name, _)| name)
}

fn same_origin(host: &str, origin: &str) -> bool {
    let Some((scheme, authority)) = origin.split_once("://") else {
        return false;
    };
    scheme == "http" && authority.eq_ignore_ascii_case(host)
}

fn validate_request(config: &ServerConfig, request: &HttpRequest) -> Result<(), &'static str> {
    let host = request
        .headers
        .get("host")
        .map(String::as_str)
        .unwrap_or("");
    if !config.cors && config.allowed_origin.is_none() {
        let is_loopback = config.bind.eq_ignore_ascii_case("localhost")
            || config
                .bind
                .parse::<IpAddr>()
                .map(|address| address.is_loopback())
                .unwrap_or(false);
        if is_loopback {
            let allowed = host_without_port(host).eq_ignore_ascii_case(&config.bind)
                || host_without_port(host).eq_ignore_ascii_case("localhost")
                || matches!(host_without_port(host), "127.0.0.1" | "::1" | "[::1]");
            if !allowed {
                return Err("invalid Host");
            }
        }
    }

    let Some(origin) = request.headers.get("origin") else {
        return Ok(());
    };
    if config.cors && config.allowed_origin.is_none() {
        return Ok(());
    }
    if let Some(allowed) = config.allowed_origin.as_deref() {
        return if origin == allowed {
            Ok(())
        } else {
            Err("origin not allowed")
        };
    }
    if same_origin(host, origin) {
        Ok(())
    } else {
        Err("cross-origin request not allowed")
    }
}

fn ws_token(request: &HttpRequest) -> Option<String> {
    if let Some(authorization) = request.headers.get("authorization") {
        let authorization = authorization.trim();
        if let Some(token) = authorization
            .strip_prefix("Bearer ")
            .or_else(|| authorization.strip_prefix("bearer "))
        {
            return Some(token.trim().to_string());
        }
    }
    request
        .headers
        .get("sec-websocket-protocol")
        .and_then(|value| {
            value
                .split(',')
                .map(str::trim)
                .find_map(|protocol| protocol.strip_prefix("bearer."))
        })
        .map(str::trim)
        .map(str::to_string)
}

fn ws_select_subprotocol(request: &HttpRequest) -> Option<&'static str> {
    const SUBPROTOCOL: &str = "symbio.v1";
    request
        .headers
        .get("sec-websocket-protocol")?
        .split(',')
        .map(str::trim)
        .any(|protocol| protocol == SUBPROTOCOL)
        .then_some(SUBPROTOCOL)
}

fn cors_headers(origin: &str) -> Vec<(&str, &str)> {
    vec![
        ("Access-Control-Allow-Origin", origin),
        ("Access-Control-Allow-Methods", "GET, POST, OPTIONS"),
        (
            "Access-Control-Allow-Headers",
            "Content-Type, Authorization, Idempotency-Key, Sec-WebSocket-Protocol",
        ),
        ("Vary", "Origin"),
    ]
}

fn status_text(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        409 => "Conflict",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        500 => "Internal Server Error",
        503 => "Service Unavailable",
        _ => "",
    }
}

fn http_response(response: &HttpResponse, cors: Option<&str>) -> Vec<u8> {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\n",
        response.status,
        status_text(response.status),
        response.content_type,
        response.body.len()
    );
    if let Some(origin) = cors {
        for (name, value) in cors_headers(origin) {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
    }
    for (name, value) in &response.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("Connection: close\r\n\r\n");
    let mut out = head.into_bytes();
    out.extend_from_slice(&response.body);
    out
}

async fn write_response<W: AsyncWrite + Unpin>(
    writer: &mut W,
    response: &HttpResponse,
    cors: Option<&str>,
) -> std::io::Result<()> {
    tokio::time::timeout(
        WRITE_TIMEOUT,
        writer.write_all(&http_response(response, cors)),
    )
    .await
    .map_err(|_| std::io::Error::new(ErrorKind::TimedOut, "HTTP response write timed out"))?
}

async fn write_all_timed<W: AsyncWrite + Unpin>(
    writer: &mut W,
    bytes: &[u8],
) -> std::io::Result<()> {
    tokio::time::timeout(WRITE_TIMEOUT, async {
        writer.write_all(bytes).await?;
        writer.flush().await
    })
    .await
    .map_err(|_| std::io::Error::new(ErrorKind::TimedOut, "HTTP response write timed out"))?
}

fn empty_response(status: u16, content_type: &str) -> HttpResponse {
    HttpResponse {
        status,
        content_type: content_type.to_string(),
        body: Vec::new(),
        headers: Vec::new(),
    }
}

async fn handle_conn(
    mut stream: TcpStream,
    ingress: Arc<dyn Ingress>,
    config: Arc<ServerConfig>,
    cancel: CancellationToken,
    _permit: OwnedSemaphorePermit,
) {
    let cors = cors_origin(&config);
    let request = match tokio::time::timeout(REQUEST_TIMEOUT, read_request(&mut stream)).await {
        Ok(Ok(request)) => request,
        Ok(Err(RequestError::Closed)) => return,
        Ok(Err(RequestError::BadRequest(what))) => {
            if let Err(error) = write_response(
                &mut stream,
                &HttpResponse::json(400, serde_json::json!({ "error": what })),
                cors,
            )
            .await
            {
                debug!(error = %error, "client disconnected while rejecting malformed request");
            }
            return;
        }
        Ok(Err(RequestError::TooLarge(what))) => {
            if let Err(error) = write_response(
                &mut stream,
                &HttpResponse::json(413, serde_json::json!({ "error": what })),
                cors,
            )
            .await
            {
                debug!(error = %error, "client disconnected while rejecting oversized request");
            }
            return;
        }
        Err(_) => {
            if let Err(error) = write_response(
                &mut stream,
                &HttpResponse::json(408, serde_json::json!({ "error": "request timed out" })),
                cors,
            )
            .await
            {
                debug!(error = %error, "client disconnected during request timeout");
            }
            return;
        }
    };

    if cancel.is_cancelled() {
        if write_response(
            &mut stream,
            &HttpResponse::json(503, serde_json::json!({ "error": "server stopping" })),
            cors,
        )
        .await
        .is_err()
        {
            return;
        }
        return;
    }

    if let Err(error) = validate_request(&config, &request) {
        if write_response(
            &mut stream,
            &HttpResponse::json(403, serde_json::json!({ "error": error })),
            cors,
        )
        .await
        .is_err()
        {
            return;
        }
        return;
    }

    if request.method == "OPTIONS" {
        if write_response(&mut stream, &empty_response(204, ""), cors)
            .await
            .is_err()
        {
            return;
        }
        return;
    }

    let request_cancel = cancel.child_token();
    let is_websocket = request
        .headers
        .get("upgrade")
        .map(|value| value.eq_ignore_ascii_case("websocket"))
        .unwrap_or(false);
    if is_websocket {
        if !config.token.is_empty() && ws_token(&request).as_deref() != Some(config.token.as_str())
        {
            let mut response = HttpResponse::bytes(401, b"unauthorized".to_vec());
            response.content_type.clear();
            if write_response(&mut stream, &response, cors).await.is_err() {
                return;
            }
            return;
        }
    } else if !config.token.is_empty()
        && !ingress.is_public(&request)
        && !check_auth(&config.token, &request)
    {
        let mut response = HttpResponse::bytes(401, b"unauthorized".to_vec());
        response.content_type.clear();
        if write_response(&mut stream, &response, cors).await.is_err() {
            return;
        }
        return;
    }

    let upgrade_headers = request.headers.clone();
    match ingress.handle(request, request_cancel.clone()).await {
        IngressReply::Response(response) => {
            request_cancel.cancel();
            if let Err(error) = write_response(&mut stream, &response, cors).await {
                debug!(error = %error, "client disconnected while writing HTTP response");
            }
        }
        IngressReply::Events(channel) => {
            handle_sse(stream, channel, cors, request_cancel.clone()).await;
            request_cancel.cancel();
            ingress.realtime_closed(request_cancel.clone()).await;
        }
        IngressReply::Upgrade => {
            let request = HttpRequest {
                method: String::new(),
                path: String::new(),
                headers: upgrade_headers,
                body: Vec::new(),
            };
            if ws_handshake(&mut stream, &request).await.is_err() {
                warn!("[http] WebSocket 握手失败");
                request_cancel.cancel();
                ingress.realtime_closed(request_cancel.clone()).await;
                return;
            }
            handle_ws(stream, ingress, request_cancel).await;
        }
    }
}

#[derive(Debug)]
enum RequestError {
    Closed,
    BadRequest(String),
    TooLarge(String),
}

async fn read_request<R: AsyncRead + Unpin>(stream: &mut R) -> Result<HttpRequest, RequestError> {
    let mut buffer = Vec::new();
    let mut byte = [0u8; 1];
    let header_end = loop {
        let count = stream
            .read(&mut byte)
            .await
            .map_err(|_| RequestError::Closed)?;
        if count == 0 {
            return Err(RequestError::Closed);
        }
        buffer.push(byte[0]);
        if buffer.len() >= 4 && &buffer[buffer.len() - 4..] == b"\r\n\r\n" {
            break buffer.len();
        }
        if buffer.len() > MAX_HEADER_BYTES {
            return Err(RequestError::TooLarge(format!(
                "request headers too large (limit {MAX_HEADER_BYTES} bytes)"
            )));
        }
    };

    let header_text = String::from_utf8(buffer[..header_end].to_vec())
        .map_err(|_| RequestError::BadRequest("request headers must be valid UTF-8".to_string()))?;
    let mut lines = header_text.split("\r\n");
    let request_line = lines.next().ok_or(RequestError::Closed)?;
    let mut parts = request_line.split_whitespace();
    let method = parts
        .next()
        .ok_or_else(|| RequestError::BadRequest("missing request method".to_string()))?
        .to_string();
    let path = parts
        .next()
        .ok_or_else(|| RequestError::BadRequest("missing request target".to_string()))?
        .to_string();
    if parts.next() != Some("HTTP/1.1") || parts.next().is_some() {
        return Err(RequestError::BadRequest(
            "request line must end with HTTP/1.1".to_string(),
        ));
    }
    let mut headers = HashMap::new();
    for line in lines {
        if line.is_empty() {
            break;
        }
        let Some(index) = line.find(':') else {
            return Err(RequestError::BadRequest(
                "malformed request header".to_string(),
            ));
        };
        let name = line[..index].trim().to_ascii_lowercase();
        if name.is_empty()
            || !name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
        {
            return Err(RequestError::BadRequest(
                "invalid request header name".to_string(),
            ));
        }
        if headers.contains_key(&name) {
            return Err(RequestError::BadRequest(format!(
                "duplicate request header: {name}"
            )));
        }
        headers.insert(name, line[index + 1..].trim().to_string());
    }
    if headers.contains_key("transfer-encoding") {
        return Err(RequestError::BadRequest(
            "Transfer-Encoding is not supported".to_string(),
        ));
    }

    let mut body = Vec::new();
    if let Some(value) = headers.get("content-length") {
        if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(RequestError::BadRequest(
                "invalid Content-Length".to_string(),
            ));
        }
        let length = value
            .parse::<usize>()
            .map_err(|_| RequestError::BadRequest("invalid Content-Length".to_string()))?;
        if length > MAX_BODY_BYTES {
            return Err(RequestError::TooLarge(format!(
                "request body too large: {length} bytes (limit {MAX_BODY_BYTES} bytes)"
            )));
        }
        while body.len() < length {
            let remaining = length - body.len();
            let mut chunk = [0u8; 4096];
            let wanted = remaining.min(chunk.len());
            let count = stream
                .read(&mut chunk[..wanted])
                .await
                .map_err(|_| RequestError::Closed)?;
            if count == 0 {
                return Err(RequestError::Closed);
            }
            body.extend_from_slice(&chunk[..count]);
        }
    }

    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

enum PayloadDelivery {
    Empty,
    Once(Value),
    Stream(PluginChannel),
}

fn classify_payload(payload: PluginPayload) -> Result<PayloadDelivery, String> {
    match payload {
        PluginPayload::Data(data) => Ok(PayloadDelivery::Once(
            data.serialize().map_err(|error| error.to_string())?,
        )),
        PluginPayload::Empty => Ok(PayloadDelivery::Empty),
        PluginPayload::Session(channel) => Ok(PayloadDelivery::Stream(channel)),
    }
}

pub async fn payload_response(payload: PluginPayload) -> Result<HttpResponse, String> {
    match classify_payload(payload)? {
        PayloadDelivery::Empty => Ok(HttpResponse::json(
            200,
            PluginPayloadWire::Data(Value::Null),
        )),
        PayloadDelivery::Once(value) => Ok(HttpResponse::json(200, PluginPayloadWire::Data(value))),
        PayloadDelivery::Stream(mut channel) => {
            let mut last = None;
            let mut failed = None;
            loop {
                match tokio::time::timeout(Duration::from_secs(30), channel.rx.recv()).await {
                    Ok(Some(PluginFrame::Data(value))) => last = Some(value),
                    Ok(Some(PluginFrame::Error(message, _))) => {
                        failed = Some(message);
                        break;
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
            channel.cancel_token.cancel();
            if let Some(message) = failed {
                return Err(message);
            }
            let value = match last {
                Some(value) => Arc::try_unwrap(value).unwrap_or_else(|shared| (*shared).clone()),
                None => Value::Null,
            };
            Ok(HttpResponse::json(200, PluginPayloadWire::Data(value)))
        }
    }
}

fn sse_data_line(value: &Value) -> Vec<u8> {
    let text = serde_json::to_string(value).unwrap_or_else(|_| "null".to_string());
    let mut out = Vec::with_capacity(text.len() + 8);
    for line in text.lines() {
        out.extend_from_slice(b"data: ");
        out.extend_from_slice(line.as_bytes());
        out.push(b'\n');
    }
    out.push(b'\n');
    out
}

async fn handle_sse(
    stream: TcpStream,
    mut channel: PluginChannel,
    cors: Option<&str>,
    cancel: CancellationToken,
) {
    let (mut read_half, mut write_half) = stream.into_split();
    let mut head = String::from(
        "HTTP/1.1 200 OK\r\n\
         Content-Type: text/event-stream\r\n\
         Cache-Control: no-cache\r\n\
         X-Accel-Buffering: no\r\n",
    );
    if let Some(origin) = cors {
        for (name, value) in cors_headers(origin) {
            head.push_str(&format!("{name}: {value}\r\n"));
        }
    }
    head.push_str("Connection: keep-alive\r\n\r\n");
    if write_all_timed(&mut write_half, head.as_bytes())
        .await
        .is_err()
    {
        channel.cancel_token.cancel();
        return;
    }

    let mut scratch = [0u8; 256];
    loop {
        let frame = tokio::select! {
            _ = cancel.cancelled() => break,
            result = read_half.read(&mut scratch) => {
                match result {
                    Ok(0) | Err(_) => break,
                    Ok(_) => continue,
                }
            }
            result = tokio::time::timeout(SSE_HEARTBEAT, channel.rx.recv()) => result,
        };
        let chunk = match frame {
            Err(_) => b": ping\n\n".to_vec(),
            Ok(Some(PluginFrame::Data(value))) => sse_data_line(&value),
            Ok(Some(PluginFrame::Error(message, _))) => {
                let payload = serde_json::json!({ "error": message });
                if write_all_timed(&mut write_half, &sse_data_line(&payload))
                    .await
                    .is_err()
                {
                    break;
                }
                break;
            }
            Ok(None) => break,
        };
        if write_all_timed(&mut write_half, &chunk).await.is_err() {
            break;
        }
    }

    channel.cancel_token.cancel();
    match tokio::time::timeout(WRITE_TIMEOUT, write_half.shutdown()).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => debug!(error = %error, "SSE peer disconnected during shutdown"),
        Err(_) => debug!("SSE shutdown timed out"),
    }
}

async fn ws_handshake(stream: &mut TcpStream, request: &HttpRequest) -> Result<(), String> {
    let key = request
        .headers
        .get("sec-websocket-key")
        .ok_or("缺少 Sec-WebSocket-Key")?;
    let accept = base64::engine::general_purpose::STANDARD
        .encode(sha1(format!("{key}{WS_GUID}").as_bytes()));
    let subprotocol = ws_select_subprotocol(request)
        .map(|protocol| format!("Sec-WebSocket-Protocol: {protocol}\r\n"))
        .unwrap_or_default();
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n{subprotocol}\r\n"
    );
    tokio::time::timeout(WRITE_TIMEOUT, stream.write_all(response.as_bytes()))
        .await
        .map_err(|_| "WebSocket handshake timed out".to_string())?
        .map_err(|error| error.to_string())
}

fn ws_error(message: impl Into<String>) -> String {
    serde_json::json!({ "Error": [message.into()] }).to_string()
}

async fn handle_ws(stream: TcpStream, ingress: Arc<dyn Ingress>, cancel: CancellationToken) {
    handle_ws_inner(stream, ingress.clone(), cancel.clone()).await;
    cancel.cancel();
    ingress.realtime_closed(cancel.clone()).await;
}

async fn send_ws_plugin_frame(
    sender: &tokio::sync::mpsc::Sender<PluginFrame>,
    frame: PluginFrame,
    cancel: &CancellationToken,
) -> bool {
    tokio::select! {
        biased;
        _ = cancel.cancelled() => false,
        result = sender.send(frame) => result.is_ok(),
    }
}

async fn handle_ws_inner(stream: TcpStream, ingress: Arc<dyn Ingress>, cancel: CancellationToken) {
    let (mut read_half, mut write_half) = stream.into_split();
    let text = tokio::select! {
        _ = cancel.cancelled() => {
            ws_close(&mut write_half).await;
            return;
        }
        result = tokio::time::timeout(REQUEST_TIMEOUT, ws_read_text(&mut read_half)) => match result {
            Ok(Some(text)) => text,
            _ => {
                ws_close(&mut write_half).await;
                return;
            }
        },
    };
    let message: PluginMessageWire = match serde_json::from_str(&text) {
        Ok(message) => message,
        Err(error) => {
            if ws_send_text(
                &mut write_half,
                &ws_error(format!("invalid request: {error}")),
            )
            .await
            .is_err()
            {
                return;
            }
            ws_close(&mut write_half).await;
            return;
        }
    };

    let delivery = ingress
        .invoke(message, cancel.clone())
        .await
        .map_err(|error| error.to_string())
        .and_then(classify_payload);
    match delivery {
        Ok(PayloadDelivery::Stream(channel)) => {
            let channel_token = channel.cancel_token.clone();
            let (tx, mut rx) = (channel.tx, channel.rx);
            loop {
                tokio::select! {
                    frame = rx.recv() => {
                        match frame {
                            Some(frame) => {
                                let text = match serde_json::to_string(&frame) {
                                    Ok(text) => text,
                                    Err(_) => ws_error("frame serialization failed"),
                                };
                                if ws_send_text(&mut write_half, &text).await.is_err() {
                                    break;
                                }
                            }
                            None => break,
                        }
                    }
                    _ = cancel.cancelled() => break,
                    result = ws_read_frame(&mut read_half) => {
                        match result {
                            Some((opcode, data)) => {
                                if opcode == 0x8 {
                                    break;
                                } else if opcode == 0x9 {
                                    if ws_send_frame(&mut write_half, 0xA, &data).await.is_err() {
                                        break;
                                    }
                                } else if let Ok(frame) = serde_json::from_slice::<PluginFrame>(&data) {
                                    if !send_ws_plugin_frame(&tx, frame, &cancel).await {
                                        break;
                                    }
                                }
                            }
                            None => break,
                        }
                    }
                }
            }
            channel_token.cancel();
            ws_close(&mut write_half).await;
        }
        Ok(PayloadDelivery::Once(value)) => {
            let text = serde_json::to_string(&PluginFrame::data(value))
                .unwrap_or_else(|_| ws_error("frame serialization failed"));
            if ws_send_text(&mut write_half, &text).await.is_err() {
                return;
            }
            ws_close(&mut write_half).await;
        }
        Ok(PayloadDelivery::Empty) => {
            ws_close(&mut write_half).await;
        }
        Err(error) => {
            if ws_send_text(&mut write_half, &ws_error(error))
                .await
                .is_err()
            {
                return;
            }
            ws_close(&mut write_half).await;
        }
    }
}

async fn ws_read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Option<(u8, Vec<u8>)> {
    let mut header = [0u8; 2];
    reader.read_exact(&mut header).await.ok()?;
    let opcode = header[0] & 0x0F;
    let masked = (header[1] & 0x80) != 0;
    let mut length = (header[1] & 0x7F) as usize;
    if length == 126 {
        let mut extended = [0u8; 2];
        reader.read_exact(&mut extended).await.ok()?;
        length = u16::from_be_bytes(extended) as usize;
    } else if length == 127 {
        let mut extended = [0u8; 8];
        reader.read_exact(&mut extended).await.ok()?;
        length = usize::try_from(u64::from_be_bytes(extended)).ok()?;
    }
    if length > MAX_WS_FRAME_BYTES {
        warn!(
            length,
            limit = MAX_WS_FRAME_BYTES,
            "[http] WebSocket 帧超限"
        );
        return None;
    }
    let mut mask = [0u8; 4];
    if masked {
        reader.read_exact(&mut mask).await.ok()?;
    }
    let mut payload = vec![0u8; length];
    reader.read_exact(&mut payload).await.ok()?;
    if masked {
        for (index, byte) in payload.iter_mut().enumerate() {
            *byte ^= mask[index % 4];
        }
    }
    Some((opcode, payload))
}

async fn ws_send_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    opcode: u8,
    data: &[u8],
) -> std::io::Result<()> {
    let length = data.len();
    let first = 0x80 | opcode;
    let mut frame = Vec::with_capacity(data.len() + 14);
    frame.push(first);
    if length < 126 {
        frame.push(length as u8);
    } else if length < 65536 {
        frame.push(126);
        frame.extend_from_slice(&(length as u16).to_be_bytes());
    } else {
        frame.push(127);
        frame.extend_from_slice(&(length as u64).to_be_bytes());
    }
    frame.extend_from_slice(data);
    write_all_timed(writer, &frame).await
}

async fn ws_send_text<W: AsyncWrite + Unpin>(writer: &mut W, text: &str) -> std::io::Result<()> {
    ws_send_frame(writer, 0x1, text.as_bytes()).await
}

async fn ws_close<W: AsyncWrite + Unpin>(writer: &mut W) {
    match tokio::time::timeout(WRITE_TIMEOUT, writer.shutdown()).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => debug!(error = %error, "WebSocket peer disconnected during shutdown"),
        Err(_) => debug!("WebSocket shutdown timed out"),
    }
}

async fn ws_read_text<R: AsyncRead + Unpin>(reader: &mut R) -> Option<String> {
    match ws_read_frame(reader).await {
        Some((opcode, data)) if opcode == 0x1 || opcode == 0x2 => String::from_utf8(data).ok(),
        _ => None,
    }
}

#[allow(clippy::many_single_char_names)]
fn sha1(message: &[u8]) -> [u8; 20] {
    let mut hash: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut padded = message.to_vec();
    let bit_length = (padded.len() as u64).wrapping_mul(8);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_length.to_be_bytes());
    for chunk in padded.chunks(64) {
        let mut words = [0u32; 80];
        for (index, word) in words.iter_mut().enumerate().take(16) {
            *word = u32::from_be_bytes([
                chunk[4 * index],
                chunk[4 * index + 1],
                chunk[4 * index + 2],
                chunk[4 * index + 3],
            ]);
        }
        for index in 16..80 {
            words[index] =
                (words[index - 3] ^ words[index - 8] ^ words[index - 14] ^ words[index - 16])
                    .rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (hash[0], hash[1], hash[2], hash[3], hash[4]);
        for (index, &word) in words.iter().enumerate() {
            let round_constant = [0x5A827999u32, 0x6ED9EBA1, 0x8F1BBCDC, 0xCA62C1D6][index / 20];
            let function = match index / 20 {
                0 => (b & c) | ((!b) & d),
                1 | 3 => b ^ c ^ d,
                _ => (b & c) | (b & d) | (c & d),
            };
            let temporary = a
                .rotate_left(5)
                .wrapping_add(function)
                .wrapping_add(e)
                .wrapping_add(round_constant)
                .wrapping_add(word);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temporary;
        }
        hash[0] = hash[0].wrapping_add(a);
        hash[1] = hash[1].wrapping_add(b);
        hash[2] = hash[2].wrapping_add(c);
        hash[3] = hash[3].wrapping_add(d);
        hash[4] = hash[4].wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for (index, word) in hash.iter().enumerate() {
        out[index * 4..index * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

#[cfg(test)]
#[path = "server.test.rs"]
mod tests;
