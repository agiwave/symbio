//! 入站服务的传输实现（手写 HTTP/1.1 + WebSocket，零新依赖、纯 Rust）
//!
//! ## 设计原则：只搬运行李，不改行李内容
//!
//! 本模块是**现有 Tauri 命令的一对一映射**，请求/响应沿用完全相同的线上类型：
//!
//! | 现有 Tauri 命令          | HTTP 端点                       | 载荷类型                            |
//! |--------------------------|--------------------------------|-------------------------------------|
//! | `route_v2`               | `POST /api/v1/invoke`           | `PluginMessageWire` → `PluginPayloadWire` |
//! | `listen("route/{id}")`   | `WS /api/v1/ws`（首帧=`PluginMessageWire`） | `PluginFrame`（双向）         |
//! | —                        | `GET /api/v1/health`           | `{"ok":true}`                        |
//!
//! 因此**没有引入任何新协议**：第三方或另一个 Symbio 实例拿到的能力与前端完全一致。
//!
//! ## WebSocket 语义
//!
//! 一条 WS 就是一个会话：连接建立后，客户端**第一帧**发送 `PluginMessageWire`
//! （与 `route_v2` 的请求体完全相同），服务端据此分派；若后端返回会话通道，
//! 此后该连接双向转发 `PluginFrame`，结束（或断开）即 EOF。
//!
//! 之所以用 WS 而非 SSE：前端 `connectPlugin`（会话/流式）需要双向通道，WS 天然映射
//! Tauri 的 invoke + event 模型，且首帧即 `PluginMessageWire`，与 `route_v2` 完全对称。

use super::config::{is_readonly_allowed, GatewayConfig};
use crate::symbio_core::{
    InvokeRequest, Plugin, PluginFrame, PluginMessageWire, PluginPayload, PluginPayloadWire,
    SimpleRequest,
};
use base64::Engine;
use serde_json::Value;
use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// 入站服务句柄
pub struct ServerHandle {
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<()>,
}

/// 停止入站服务
pub fn stop(handle: ServerHandle) {
    handle.cancel.cancel();
    handle.task.abort();
}

// ==================== 启动 ====================

/// 按配置启动入站服务
///
/// - `native` 或不启用：不监听端口，返回 `Ok(None)`
/// - `http`：建立监听，返回句柄
/// - 绑定非回环地址但未设令牌：返回错误（安全护栏）
pub async fn start(
    cfg: &GatewayConfig,
    router: Arc<dyn Plugin>,
) -> Result<Option<ServerHandle>, String> {
    if !cfg.inbound_enabled || cfg.inbound_protocol != "http" {
        return Ok(None);
    }
    let is_loopback = cfg.inbound_bind == "127.0.0.1"
        || cfg.inbound_bind == "localhost"
        || cfg.inbound_bind == "::1";
    if !is_loopback && cfg.inbound_token.is_empty() {
        return Err("绑定非回环地址时必须设置访问令牌".to_string());
    }

    let addr = format!("{}:{}", cfg.inbound_bind, cfg.inbound_port);
    let listener = TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("监听失败 {addr}: {e}"))?;

    let cancel = CancellationToken::new();
    let token = cfg.inbound_token.clone();
    let readonly = cfg.inbound_readonly;

    let task = tokio::spawn({
        let cancel = cancel.clone();
        async move {
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    accepted = listener.accept() => {
                        match accepted {
                            Ok((stream, _)) => {
                                let router = router.clone();
                                let token = token.clone();
                                tokio::spawn(handle_conn(stream, router, token, readonly));
                            }
                            Err(e) => warn!(error = %e, "[gateway] accept 失败"),
                        }
                    }
                }
            }
            info!("[gateway] 入站服务已停止");
        }
    });

    Ok(Some(ServerHandle { cancel, task }))
}

// ==================== HTTP 连接分发 ====================

struct HttpRequest {
    method: String,
    path: String,
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

fn cors_headers() -> [(&'static str, &'static str); 3] {
    [
        ("Access-Control-Allow-Origin", "*"),
        ("Access-Control-Allow-Methods", "GET, POST, OPTIONS"),
        (
            "Access-Control-Allow-Headers",
            "Content-Type, Authorization",
        ),
    ]
}

fn http_response(
    status: u16,
    status_text: &str,
    content_type: &str,
    body: &[u8],
    extra: &[(&str, &str)],
) -> Vec<u8> {
    let mut head = format!(
        "HTTP/1.1 {status} {status_text}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\n",
        body.len()
    );
    for (k, v) in extra {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("Connection: close\r\n\r\n");
    let mut out = head.into_bytes();
    out.extend_from_slice(body);
    out
}

fn check_auth(token: &str, req: &HttpRequest) -> bool {
    if token.is_empty() {
        return true;
    }
    req.headers
        .get("authorization")
        .map(|a| a.trim())
        .map(|a| a == format!("Bearer {token}"))
        .unwrap_or(false)
}

fn query_param(path: &str, key: &str) -> Option<String> {
    let q = path.split_once('?').map(|(_, q)| q).unwrap_or("");
    q.split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v.to_string())
}

async fn handle_conn(
    mut stream: TcpStream,
    router: Arc<dyn Plugin>,
    token: String,
    readonly: bool,
) {
    let Some(req) = read_request(&mut stream).await else {
        return;
    };

    // CORS 预检
    if req.method == "OPTIONS" {
        let _ = stream
            .write_all(&http_response(204, "No Content", "", b"", &cors_headers()))
            .await;
        return;
    }

    // WebSocket 升级（令牌经查询参数传递，因 WS 握手无法自定义头）
    if req
        .headers
        .get("upgrade")
        .map(|s| s.to_lowercase())
        .as_deref()
        == Some("websocket")
    {
        if !token.is_empty() && query_param(&req.path, "token").as_deref() != Some(token.as_str()) {
            let _ = stream
                .write_all(&http_response(
                    401,
                    "Unauthorized",
                    "",
                    b"unauthorized",
                    &cors_headers(),
                ))
                .await;
            return;
        }
        if ws_handshake(&mut stream, &req).await.is_err() {
            warn!("[gateway] WebSocket 握手失败");
            return;
        }
        handle_ws(stream, router, readonly).await;
        return;
    }

    // 健康检查
    if req.method == "GET" && req.path.starts_with("/api/v1/health") {
        let _ = stream
            .write_all(&http_response(
                200,
                "OK",
                "application/json",
                br#"{"ok":true}"#,
                &cors_headers(),
            ))
            .await;
        return;
    }

    // 一次性调用：POST /api/v1/invoke
    if req.method == "POST" && req.path.starts_with("/api/v1/invoke") {
        if !check_auth(&token, &req) {
            let _ = stream
                .write_all(&http_response(
                    401,
                    "Unauthorized",
                    "",
                    b"unauthorized",
                    &cors_headers(),
                ))
                .await;
            return;
        }
        let msg: PluginMessageWire = match serde_json::from_slice(&req.body) {
            Ok(m) => m,
            Err(e) => {
                let b = format!("{{\"error\":\"invalid request: {e}\"}}").into_bytes();
                let _ = stream
                    .write_all(&http_response(
                        400,
                        "Bad Request",
                        "application/json",
                        &b,
                        &cors_headers(),
                    ))
                    .await;
                return;
            }
        };
        match dispatch_once(&router, &msg, readonly).await {
            Ok(wire) => {
                let body = serde_json::to_vec(&wire).unwrap_or_default();
                let _ = stream
                    .write_all(&http_response(
                        200,
                        "OK",
                        "application/json",
                        &body,
                        &cors_headers(),
                    ))
                    .await;
            }
            Err(e) => {
                let b = format!("{{\"error\":\"{e}\"}}").into_bytes();
                let _ = stream
                    .write_all(&http_response(
                        400,
                        "Bad Request",
                        "application/json",
                        &b,
                        &cors_headers(),
                    ))
                    .await;
            }
        }
        return;
    }

    // 未匹配
    let _ = stream
        .write_all(&http_response(
            404,
            "Not Found",
            "application/json",
            b"{\"error\":\"not found\"}",
            &cors_headers(),
        ))
        .await;
}

// ==================== 请求解析 ====================

async fn read_request(stream: &mut TcpStream) -> Option<HttpRequest> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    let header_end = loop {
        let n = stream.read(&mut byte).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.push(byte[0]);
        if buf.len() >= 4 && &buf[buf.len() - 4..] == b"\r\n\r\n" {
            break buf.len();
        }
        if buf.len() > 64 * 1024 {
            return None;
        }
    };
    let header_bytes = &buf[..header_end];
    let header_str = String::from_utf8_lossy(header_bytes);
    let mut lines = header_str.split("\r\n");
    let request_line = lines.next()?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next()?.to_string();
    let path = parts.next()?.to_string();
    let mut headers: HashMap<String, String> = HashMap::new();
    for line in lines {
        if let Some(idx) = line.find(':') {
            let k = line[..idx].trim().to_lowercase();
            let v = line[idx + 1..].trim().to_string();
            headers.insert(k, v);
        }
    }
    let mut body = Vec::new();
    if let Some(len) = headers
        .get("content-length")
        .and_then(|s| s.parse::<usize>().ok())
    {
        if len > 0 {
            while body.len() < len {
                let mut chunk = [0u8; 4096];
                let n = stream.read(&mut chunk).await.ok()?;
                if n == 0 {
                    break;
                }
                body.extend_from_slice(&chunk[..n]);
            }
            body.truncate(len);
        }
    }
    Some(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

// ==================== 分派（与传输无关） ====================

/// 把线路请求交给分形路由，返回线路响应
///
/// 会话型响应在此消费到 EOF（或超时）后折叠为最后一帧数据，
/// 与前端 `callPlugin` 的一次性语义一致。
async fn dispatch_once(
    router: &Arc<dyn Plugin>,
    msg: &PluginMessageWire,
    readonly: bool,
) -> Result<PluginPayloadWire, String> {
    let path = msg
        .metadata
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if readonly && !is_readonly_allowed(path) {
        return Err(format!("只读模式下禁止调用: {path}"));
    }

    let ctx = build_ctx(msg);
    let payload = router.clone().route(ctx).await.map_err(|e| e.to_string())?;

    match payload {
        PluginPayload::Data(d) => Ok(PluginPayloadWire::Data(
            d.serialize().map_err(|e| e.to_string())?,
        )),
        PluginPayload::Empty => Ok(PluginPayloadWire::Data(Value::Null)),
        PluginPayload::Native(_) => Err("该路径返回进程内原生对象，不支持跨传输调用".to_string()),
        PluginPayload::Session(mut chan) => {
            // 消费到 EOF，保留最后一帧；永不 EOF 的订阅由超时兜底
            let mut last = Value::Null;
            loop {
                match tokio::time::timeout(Duration::from_secs(30), chan.rx.recv()).await {
                    Ok(Some(PluginFrame::Data(v))) => last = v,
                    Ok(Some(PluginFrame::Error(m, _))) => return Err(m),
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
            Ok(PluginPayloadWire::Data(last))
        }
    }
}

/// 由 `PluginMessageWire` 构造与 `route_v2` 完全一致的 `SimpleRequest`
fn build_ctx(msg: &PluginMessageWire) -> Arc<dyn InvokeRequest> {
    let mut extensions: HashMap<String, Arc<dyn Any + Send + Sync>> = HashMap::new();
    extensions.insert(
        "payload".to_string(),
        Arc::new(msg.payload.clone()) as Arc<dyn Any + Send + Sync>,
    );
    if let Some(obj) = msg.metadata.as_object() {
        for (k, v) in obj {
            if let Some(s) = v.as_str() {
                extensions.insert(
                    k.clone(),
                    Arc::new(s.to_string()) as Arc<dyn Any + Send + Sync>,
                );
            } else {
                extensions.insert(k.clone(), Arc::new(v.clone()) as Arc<dyn Any + Send + Sync>);
            }
        }
    }
    Arc::new(SimpleRequest {
        envs: Arc::new(RwLock::new(HashMap::new())),
        extensions: Arc::new(RwLock::new(extensions)),
    })
}

// ==================== WebSocket ====================

const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

async fn ws_handshake(stream: &mut TcpStream, req: &HttpRequest) -> Result<(), String> {
    let key = req
        .headers
        .get("sec-websocket-key")
        .ok_or("缺少 Sec-WebSocket-Key")?;
    let concat = format!("{key}{WS_GUID}");
    let accept = base64::engine::general_purpose::STANDARD.encode(sha1(concat.as_bytes()));
    let resp = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n"
    );
    stream
        .write_all(resp.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

async fn handle_ws(stream: TcpStream, router: Arc<dyn Plugin>, readonly: bool) {
    let (mut read_half, mut write_half) = stream.into_split();

    // 第一帧：请求体 PluginMessageWire（与 route_v2 的请求体一致）
    let text = match ws_read_text(&mut read_half).await {
        Some(t) => t,
        None => {
            let _ = write_half.shutdown().await;
            return;
        }
    };
    let msg: PluginMessageWire = match serde_json::from_str(&text) {
        Ok(m) => m,
        Err(e) => {
            let _ = ws_send_text(
                &mut write_half,
                &format!("{{\"Error\":[\"invalid request: {e}\"]}}"),
            )
            .await;
            let _ = write_half.shutdown().await;
            return;
        }
    };
    let path = msg
        .metadata
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if readonly && !is_readonly_allowed(path) {
        let _ = ws_send_text(
            &mut write_half,
            &format!("{{\"Error\":[\"只读模式下禁止调用: {path}\"]}}"),
        )
        .await;
        let _ = write_half.shutdown().await;
        return;
    }

    let ctx = build_ctx(&msg);
    match router.route(ctx).await {
        Ok(PluginPayload::Session(chan)) => {
            let (tx, mut rx) = (chan.tx, chan.rx);
            // 单任务 select：后端帧 → WS；WS 帧 → 后端（含 ping/pong、close）
            loop {
                tokio::select! {
                    frame = rx.recv() => {
                        match frame {
                            Some(f) => {
                                let text = serde_json::to_string(&f).unwrap_or_default();
                                if ws_send_text(&mut write_half, &text).await.is_err() { break; }
                            }
                            None => break,
                        }
                    }
                    r = ws_read_frame(&mut read_half) => {
                        match r {
                            Some((op, data)) => {
                                if op == 0x8 { break; } // close
                                else if op == 0x9 {
                                    let _ = ws_send_frame(&mut write_half, 0xA, &data).await; // ping → pong
                                } else if let Ok(f) = serde_json::from_slice::<PluginFrame>(&data) {
                                    if tx.send(f).await.is_err() { break; }
                                }
                            }
                            None => break,
                        }
                    }
                }
            }
            let _ = write_half.shutdown().await;
        }
        Ok(PluginPayload::Data(d)) => {
            let value = d.serialize().unwrap_or(Value::Null);
            let frame = PluginFrame::Data(value);
            let text = serde_json::to_string(&frame).unwrap_or_default();
            let _ = ws_send_text(&mut write_half, &text).await;
            let _ = write_half.shutdown().await;
        }
        Ok(PluginPayload::Empty) => {
            let _ = write_half.shutdown().await;
        }
        Ok(PluginPayload::Native(_)) => {
            let _ = ws_send_text(
                &mut write_half,
                "{\"Error\":[\"该路径返回进程内原生对象，不支持跨传输调用\"]}",
            )
            .await;
            let _ = write_half.shutdown().await;
        }
        Err(e) => {
            let _ = ws_send_text(&mut write_half, &format!("{{\"Error\":[\"{e}\"]}}")).await;
            let _ = write_half.shutdown().await;
        }
    }
}

// ---- WebSocket 帧编解码（客户端帧带掩码，服务端帧不带掩码） ----

async fn ws_read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Option<(u8, Vec<u8>)> {
    let mut hdr = [0u8; 2];
    reader.read_exact(&mut hdr).await.ok()?;
    let opcode = hdr[0] & 0x0F;
    let masked = (hdr[1] & 0x80) != 0;
    let mut len = (hdr[1] & 0x7F) as usize;
    if len == 126 {
        let mut b = [0u8; 2];
        reader.read_exact(&mut b).await.ok()?;
        len = u16::from_be_bytes(b) as usize;
    } else if len == 127 {
        let mut b = [0u8; 8];
        reader.read_exact(&mut b).await.ok()?;
        len = u64::from_be_bytes(b) as usize;
    }
    let mut mask = [0u8; 4];
    if masked {
        reader.read_exact(&mut mask).await.ok()?;
    }
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).await.ok()?;
    if masked {
        for i in 0..len {
            payload[i] ^= mask[i % 4];
        }
    }
    Some((opcode, payload))
}

async fn ws_send_frame<W: AsyncWrite + Unpin>(
    writer: &mut W,
    opcode: u8,
    data: &[u8],
) -> Result<(), std::io::Error> {
    let len = data.len();
    let b0 = 0x80 | opcode; // FIN + opcode
    if len < 126 {
        writer.write_all(&[b0, len as u8]).await?;
    } else if len < 65536 {
        writer.write_all(&[b0, 126]).await?;
        writer.write_all(&(len as u16).to_be_bytes()).await?;
    } else {
        writer.write_all(&[b0, 127]).await?;
        writer.write_all(&(len as u64).to_be_bytes()).await?;
    }
    writer.write_all(data).await?;
    writer.flush().await?;
    Ok(())
}

async fn ws_send_text<W: AsyncWrite + Unpin>(
    writer: &mut W,
    text: &str,
) -> Result<(), std::io::Error> {
    ws_send_frame(writer, 0x1, text.as_bytes()).await
}

async fn ws_read_text<R: AsyncRead + Unpin>(reader: &mut R) -> Option<String> {
    match ws_read_frame(reader).await {
        Some((op, data)) if op == 0x1 || op == 0x2 => String::from_utf8(data).ok(),
        _ => None,
    }
}

// ---- 内联 SHA1（用于 WebSocket 握手的 Sec-WebSocket-Accept，零新依赖） ----

#[allow(clippy::many_single_char_names)] // SHA1 算法标准变量名 h/a/b/c/d/e，改名反而降低可读性
fn sha1(message: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut msg = message.to_vec();
    let ml = (msg.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&ml.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 80];
        for (i, word) in w.iter_mut().enumerate().take(16) {
            *word = u32::from_be_bytes([
                chunk[4 * i],
                chunk[4 * i + 1],
                chunk[4 * i + 2],
                chunk[4 * i + 3],
            ]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        // SHA-1 轮常数：每 20 轮一组（0x5A827999 / 0x6ED9EBA1 / 0x8F1BBCDC / 0xCA62C1D6）
        let round_k = |i: usize| [0x5A827999u32, 0x6ED9EBA1, 0x8F1BBCDC, 0xCA62C1D6][i / 20];
        for (i, &wi) in w.iter().enumerate() {
            let k = round_k(i);
            let f = match i / 20 {
                0 => (b & c) | ((!b) & d),
                1 | 3 => b ^ c ^ d,
                _ => (b & c) | (b & d) | (c & d),
            };
            let temp = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = temp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for i in 0..5 {
        out[i * 4..i * 4 + 4].copy_from_slice(&h[i].to_be_bytes());
    }
    out
}
