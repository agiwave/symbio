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
//! | `event_bus/subscribe`    | `GET /api/v1/events`（SSE，只读） | `PluginFrame::Data` 的信封     |
//! | —                        | `GET /api/v1/health`           | `{"ok":true}`                        |
//!
//! 因此**没有引入任何新协议**：第三方或另一个 Symbio 实例拿到的能力，在 worker
//! 子树内与前端一致——但**边界与前端不同**：gateway 挂在 worker composite 之下、
//! `Composite::route` 不向上转发，home 自身终结的四条 `home/*` 臂经入站不可达
//! （t69 ⑦ 钉住；只读白名单已与该可达面对齐，见 `config.rs::is_readonly_allowed`）。
//!
//! ## WebSocket 语义
//!
//! 一条 WS 就是一个会话：连接建立后，客户端**第一帧**发送 `PluginMessageWire`
//! （与 `route_v2` 的请求体完全相同），服务端据此分派；若后端返回会话通道，
//! 此后该连接双向转发 `PluginFrame`，结束（或断开）即 EOF。
//!
//! 之所以主通道用 WS 而非 SSE：前端 `connectPlugin`（会话/流式）需要双向通道，WS
//! 天然映射 Tauri 的 invoke + event 模型，且首帧即 `PluginMessageWire`，与 `route_v2`
//! 完全对称。SSE 只做**单向只读**的实时面（`GET /api/v1/events`），价值在 `curl -N`
//! 即可调试；它不新增信源也不改帧形状，只是把同一个总线信封换一种传输。
//!
//! ## 边界加固
//!
//! - **令牌不走 URL**：HTTP 用 `Authorization: Bearer`；WS 除该头外，浏览器还可经
//!   `Sec-WebSocket-Protocol: symbio.v1, bearer.<token>` 携带（浏览器 WS 构造器不能
//!   自定义头）。续订只回选 `symbio.v1`，**不回显 `bearer.*`**——回显等于把令牌写进
//!   响应头，代理与日志都会记下。语义与理由见 AR-008 §5.2。
//! - **连接上限**：`inbound_max_connections` 个配额，到顶后不再 `accept`（背压交给
//!   内核队列），而不是先收下再排队。HTTP 一次调用、WS 与 SSE 的每条长连接各占一个。
//! - **重放防护**：`POST /api/v1/invoke` 的 `Idempotency-Key` 在 TTL 内重复即 409，
//!   不缓存首次结果（拒绝重放，见 AR-008 §5.1.1）。

use super::config::{is_readonly_allowed, GatewayConfig};
use crate::symbio_core::{
    EventBusSubscribeRequest, Plugin, PluginChannel, PluginFrame, PluginInvokeRequest,
    PluginInvokeRequestExt, PluginMessageWire, PluginPayload, PluginPayloadWire,
    PluginSimpleRequest, SymbioKey, PATH, ROUTE_EVENT_BUS_SUBSCRIBE, TRACE_ID,
};
use base64::Engine;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};

/// HTTP 请求头上限（64 KiB）——头是逐字节读的，不设上限时一个只发头不发全的
/// 连接就能长期占住内存与任务。
const MAX_HEADER_BYTES: usize = 64 * 1024;

/// HTTP 请求体上限（8 MiB）。
///
/// 不设上限时，`Content-Length: 999999999999` 会让服务端一直往内存里读：
/// 单个请求即可耗尽进程内存。当前默认只绑回环地址，改绑 `0.0.0.0` 后
/// 就是一个远端 DoS 入口。
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

/// WebSocket 单帧上限（8 MiB）。
///
/// 帧长由客户端给出，`vec![0u8; len]` 会照着这个数**立即分配**——
/// len = 2^60 足以当场 OOM。
const MAX_WS_FRAME_BYTES: usize = 8 * 1024 * 1024;

/// 幂等键保留时长：同一键在这段时间内重复到达即视为重放。
const IDEMPOTENCY_TTL: Duration = Duration::from_secs(600);

/// 幂等表容量上限：TTL 内键无界增长本身就是一条内存 DoS。
const IDEMPOTENCY_CAPACITY: usize = 4096;

/// 幂等键长度上限（超过它的键没有正当用途；1–200 是常见代理的保守值）。
const MAX_IDEMPOTENCY_KEY_BYTES: usize = 200;

/// SSE 心跳间隔：长时间零字节的响应会被代理 / 内核判死（SSE 没有 WS 的 ping 帧），
/// 空闲期用注释行保活——注释行不触发 `EventSource` 的任何回调。
const SSE_HEARTBEAT: Duration = Duration::from_secs(15);

/// 连接上限的合法范围（与设置页 number 控件一致）。
///
/// 手改配置文件写个越界数字时钳到边界，而不是让 `Semaphore::new` 在启动期 panic。
const MAX_CONNECTION_CAP: usize = 1024;

/// 请求解析失败的原因
enum RequestError {
    /// 连接关闭 / 报文不完整：静默断开（客户端自己走了，不是错误）
    Closed,
    /// 超过大小上限：回 413，让客户端知道原因而不是干等
    TooLarge(String),
}

/// 入站服务句柄
pub struct ServerHandle {
    cancel: CancellationToken,
    task: tokio::task::JoinHandle<()>,
    /// 实际绑定地址（`inbound_port: 0` 时由操作系统分配）。
    ///
    /// 它不只是一个测试便利：配置写的是「想绑哪儿」，这里记的是「真的绑上了哪儿」
    /// ——两者在端口冲突 / 端口 0 时不同，把后者当成前者的抄本就会在状态面上说谎。
    addr: std::net::SocketAddr,
}

impl ServerHandle {
    /// 实际监听地址。
    pub fn local_addr(&self) -> std::net::SocketAddr {
        self.addr
    }
}

/// 停止入站服务。
///
/// ## 停用语义（[D-075](../../../../docs/003-design/002-架构总览.md#d-075-卸载能力先于目录分离session-切分立三条前置会话记忆与存储留守 "DE-002-架构总览") 补注）
///
/// **停止接受新连接 + 关闭已建立的实时通道；已经进入分派的请求让它跑完**。
/// 三个动作分别是：
///
/// 1. `cancel` → accept 循环退出（不再接新连接），已接受的连接任务各自在自己的
///    读帧 / 读请求边界看到取消位后关闭；
/// 2. `task.abort()` → accept 任务本身立即退出（它不持有任何业务事实）；
/// 3. **不中断一次正在进行的 `route`**：网关只是转发层，一次工具轮的事实由会话侧
///    收束；在转发中途掐断会把「停用一个通道」变成「一次未收束的轮」。
///
/// 连接任务由 `tokio::spawn` 独立持有，`abort` accept 任务不会波及它们——取消位
/// 因此必须显式传进每个连接，不能靠句柄的 drop。
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
    require_token_for_non_loopback(&cfg.inbound_bind, &cfg.inbound_token)?;

    let addr = format!("{}:{}", cfg.inbound_bind, cfg.inbound_port);
    let listener = TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("监听失败 {addr}: {e}"))?;
    let local = listener
        .local_addr()
        .map_err(|e| format!("读取监听地址失败 {addr}: {e}"))?;

    let cancel = CancellationToken::new();
    let token = cfg.inbound_token.clone();
    let readonly = cfg.inbound_readonly;
    let permits = Arc::new(Semaphore::new(connection_cap(cfg.inbound_max_connections)));
    let idempotency = Idempotency::new();

    let task = tokio::spawn({
        let cancel = cancel.clone();
        async move {
            loop {
                // 先拿配额再 `accept`：到顶时新连接留在内核 accept 队列里等（背压），
                // 而不是先接受再在应用里排队——后者只是把等待搬进本进程的内存。
                let permit = tokio::select! {
                    _ = cancel.cancelled() => break,
                    permit = permits.clone().acquire_owned() => match permit {
                        Ok(p) => p,
                        Err(_) => break,
                    },
                };
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    accepted = listener.accept() => {
                        match accepted {
                            Ok((stream, _)) => {
                                tokio::spawn(handle_conn(
                                    stream,
                                    router.clone(),
                                    token.clone(),
                                    readonly,
                                    idempotency.clone(),
                                    cancel.clone(),
                                    permit,
                                ));
                            }
                            // 配额随本轮 arm 结束归还；连接没建立，不占额度。
                            Err(e) => warn!(error = %e, "[gateway] accept 失败"),
                        }
                    }
                }
            }
            info!("[gateway] 入站服务已停止");
        }
    });

    Ok(Some(ServerHandle {
        cancel,
        task,
        addr: local,
    }))
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
            "Content-Type, Authorization, Idempotency-Key",
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

/// **安全护栏**：绑定非回环地址时必须设置访问令牌。
///
/// 它与 [`check_auth`] 是同一枚硬币的两面——[`check_auth`] 在令牌为空时**直接放行**
/// （那是回环本机开发的有意形态），所以「谁能连进来」只能由本函数在**启动时**把住：
/// 令牌为空 ⇒ 只允许绑在回环上。少了这道判断，把网关绑到 `0.0.0.0` 且忘记设令牌，
/// 就是把整个事实源对网络上任意主机敞开，而请求侧不会拦下任何一条。
///
/// 提成函数不是为了复用（它只有一个调用点），是为了**可判据**：内联在 [`start`] 里时
/// 它没有测试，等于一道写了文档、随时可能被无声删掉的安全闸。判据见 `server.test.rs`：
/// 谓词四象限各一条（不占端口），外加一条走 [`start`] 的接线断言——谓词写得再对，
/// `start` 不调它也等于没有。
fn require_token_for_non_loopback(bind: &str, token: &str) -> Result<(), String> {
    let is_loopback = bind == "127.0.0.1" || bind == "localhost" || bind == "::1";
    if !is_loopback && token.is_empty() {
        return Err("绑定非回环地址时必须设置访问令牌".to_string());
    }
    Ok(())
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

/// 连接上限：`0` 按 `1` 处理（等于关掉服务，不该由默认值的意外产生），上限钳到
/// [`MAX_CONNECTION_CAP`]（避免手改配置把 `Semaphore::new` 推进 panic）。
fn connection_cap(configured: usize) -> usize {
    configured.clamp(1, MAX_CONNECTION_CAP)
}

// ==================== 重放防护（`Idempotency-Key`） ====================

/// 幂等表：**拒绝重复，不缓存旧结果**。
///
/// 同一个键在 [`IDEMPOTENCY_TTL`] 内第二次到达即 409；客户端应把「第一次是否成功」
/// 当作未知结果去读状态，而不是重放一次写操作。首次在**分派前**登记，分派失败则
/// 释放（失败没有产生副作用，可以带同一个键重试）。
///
/// 它是**进程内**的边界：重启即清空。跨进程去重需要持久化，而网关的定位是
/// 「本机的另一个入口」，不是集群网关（AR-008 §5.1.1）。
struct Idempotency {
    seen: Mutex<HashMap<String, Instant>>,
}

struct IdempotencyTicket {
    table: Arc<Idempotency>,
    key: String,
}

impl Idempotency {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            seen: Mutex::new(HashMap::new()),
        })
    }

    /// 登记一个键；未过 TTL 的重复键返回 `None`。
    fn reserve(self: &Arc<Self>, key: &str) -> Option<IdempotencyTicket> {
        // 锁中毒只可能来自别处的 panic：表上没有跨键不变量，取回内部值继续即可。
        let mut seen = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();
        seen.retain(|_, at| now.duration_since(*at) < IDEMPOTENCY_TTL);
        if seen.contains_key(key) {
            return None;
        }
        if seen.len() >= IDEMPOTENCY_CAPACITY {
            // 键由客户端自选、没有归属约束：若在满表时拒绝新键，任何人用 4096 个
            // 一次性键就能把合法客户端永久挡在 409 上。淘汰最旧的一条，让表始终
            // 只表达「最近 TTL 内的重放」。
            if let Some(oldest) = seen
                .iter()
                .min_by_key(|(_, at)| **at)
                .map(|(k, _)| k.clone())
            {
                seen.remove(&oldest);
            }
        }
        seen.insert(key.to_string(), now);
        Some(IdempotencyTicket {
            table: self.clone(),
            key: key.to_string(),
        })
    }
}

impl IdempotencyTicket {
    /// 分派失败：撤销登记，让同一个键可以重试。
    fn release(&self) {
        let mut seen = self.table.seen.lock().unwrap_or_else(|e| e.into_inner());
        seen.remove(&self.key);
    }
}

/// `Idempotency-Key` 的取值是否可用：1–200 个可见 ASCII。
///
/// 空白与控制字符一律不认——它们只会让「同一个键」在不同客户端/代理下
/// 被规范化成不同字符串，从而让去重失效。
fn valid_idempotency_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_IDEMPOTENCY_KEY_BYTES
        && key.bytes().all(|b| (0x21..=0x7E).contains(&b))
}

/// 从 WS 升级请求里取令牌。
///
/// 浏览器 `WebSocket` 构造器不能自定义头，只能经子协议携带；非浏览器客户端
/// （CLI / 其他实例 / wscat）用 `Authorization`。两者都提供时以 `Authorization`
/// 为准——它是显式凭据，子协议里的只是浏览器退路。
fn ws_token(req: &HttpRequest) -> Option<String> {
    if let Some(auth) = req.headers.get("authorization") {
        let auth = auth.trim();
        if let Some(t) = auth
            .strip_prefix("Bearer ")
            .or_else(|| auth.strip_prefix("bearer "))
        {
            return Some(t.trim().to_string());
        }
    }
    req.headers
        .get("sec-websocket-protocol")
        .and_then(|v| {
            v.split(',')
                .map(str::trim)
                .find_map(|p| p.strip_prefix("bearer."))
        })
        .map(|t| t.trim().to_string())
}

/// 握手回选的子协议：只回 `symbio.v1`，**永不回显 `bearer.*`**。
fn ws_select_subprotocol(req: &HttpRequest) -> Option<&'static str> {
    const SUBPROTOCOL: &str = "symbio.v1";
    req.headers
        .get("sec-websocket-protocol")?
        .split(',')
        .map(str::trim)
        .any(|p| p == SUBPROTOCOL)
        .then_some(SUBPROTOCOL)
}

async fn handle_conn(
    mut stream: TcpStream,
    router: Arc<dyn Plugin>,
    token: String,
    readonly: bool,
    idempotency: Arc<Idempotency>,
    cancel: CancellationToken,
    // 连接存活即占一个配额；持有到本函数返回（`_` 前缀只表示不读它）。
    _permit: OwnedSemaphorePermit,
) {
    let req = match read_request(&mut stream).await {
        Ok(r) => r,
        Err(RequestError::Closed) => return,
        Err(RequestError::TooLarge(what)) => {
            let b = format!("{{\"error\":\"{what}\"}}").into_bytes();
            let _ = stream
                .write_all(&http_response(
                    413,
                    "Payload Too Large",
                    "application/json",
                    &b,
                    &cors_headers(),
                ))
                .await;
            return;
        }
    };

    // 停用中的竞态窗口：accept 已关闭，但取消前刚被接下的连接可能在这里。
    // 回一条可归因的 503，而不是静默断开；已经进入分派的请求不受此判影响。
    if cancel.is_cancelled() {
        let _ = stream
            .write_all(&http_response(
                503,
                "Service Unavailable",
                "application/json",
                br#"{"error":"gateway stopping"}"#,
                &cors_headers(),
            ))
            .await;
        return;
    }

    // CORS 预检
    if req.method == "OPTIONS" {
        let _ = stream
            .write_all(&http_response(204, "No Content", "", b"", &cors_headers()))
            .await;
        return;
    }

    // WebSocket 升级（令牌走 Authorization 或子协议 `bearer.<token>`，不走 URL）
    if req
        .headers
        .get("upgrade")
        .map(|s| s.to_lowercase())
        .as_deref()
        == Some("websocket")
    {
        if !token.is_empty() && ws_token(&req).as_deref() != Some(token.as_str()) {
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
        handle_ws(stream, router, readonly, cancel).await;
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

    // 实时面（只读）：GET /api/v1/events
    if req.method == "GET" && req.path.starts_with("/api/v1/events") {
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
        // 只读模式不开放实时面：它是**长连接广播**，一次订阅覆盖该作用域内全部
        // 会话的在途内容（含尚未落库的增量），与「逐个地址去读」的只读口径不同。
        // 同一条取舍见 `config.rs::is_readonly_allowed` 的模块注释。
        if readonly {
            let b = "{\"error\":\"只读模式下不开放实时订阅\"}";
            let _ = stream
                .write_all(&http_response(
                    403,
                    "Forbidden",
                    "application/json",
                    b.as_bytes(),
                    &cors_headers(),
                ))
                .await;
            return;
        }
        handle_sse(stream, router, cancel).await;
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
        // 幂等键在**分派前**登记：先登记再执行，两个并发同键请求里至多一个会跑。
        let ticket = match req.headers.get("idempotency-key").map(|k| k.trim()) {
            Some(key) if !valid_idempotency_key(key) => {
                let _ = stream
                    .write_all(&http_response(
                        400,
                        "Bad Request",
                        "application/json",
                        br#"{"error":"invalid Idempotency-Key"}"#,
                        &cors_headers(),
                    ))
                    .await;
                return;
            }
            Some(key) => match idempotency.reserve(key) {
                Some(t) => Some(t),
                None => {
                    let _ = stream
                        .write_all(&http_response(
                            409,
                            "Conflict",
                            "application/json",
                            br#"{"error":"duplicate Idempotency-Key"}"#,
                            &cors_headers(),
                        ))
                        .await;
                    return;
                }
            },
            None => None,
        };
        let msg: PluginMessageWire = match serde_json::from_slice(&req.body) {
            Ok(m) => m,
            Err(e) => {
                if let Some(t) = &ticket {
                    t.release();
                }
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
                // 失败没有产生副作用：撤销登记，客户端可带同一个键重试。
                if let Some(t) = &ticket {
                    t.release();
                }
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

// 泛化到 `AsyncRead`（而非 `TcpStream`）：解析逻辑与传输无关，泛型化后
// 上限判定（头 64K / 体 8M）可以用内存缓冲区直接测——这几条正是防 DoS 的闸门，
// 不能只靠"读代码看起来对"。
async fn read_request<R: AsyncRead + Unpin>(stream: &mut R) -> Result<HttpRequest, RequestError> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    let header_end = loop {
        let n = stream
            .read(&mut byte)
            .await
            .map_err(|_| RequestError::Closed)?;
        if n == 0 {
            return Err(RequestError::Closed);
        }
        buf.push(byte[0]);
        if buf.len() >= 4 && &buf[buf.len() - 4..] == b"\r\n\r\n" {
            break buf.len();
        }
        if buf.len() > MAX_HEADER_BYTES {
            return Err(RequestError::TooLarge(format!(
                "request headers too large (limit {MAX_HEADER_BYTES} bytes)"
            )));
        }
    };
    let header_bytes = &buf[..header_end];
    let header_str = String::from_utf8_lossy(header_bytes);
    let mut lines = header_str.split("\r\n");
    let request_line = lines.next().ok_or(RequestError::Closed)?;
    let mut parts = request_line.split_whitespace();
    let method = parts.next().ok_or(RequestError::Closed)?.to_string();
    let path = parts.next().ok_or(RequestError::Closed)?.to_string();
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
        if len > MAX_BODY_BYTES {
            return Err(RequestError::TooLarge(format!(
                "request body too large: {len} bytes (limit {MAX_BODY_BYTES})"
            )));
        }
        if len > 0 {
            while body.len() < len {
                let mut chunk = [0u8; 4096];
                let n = stream
                    .read(&mut chunk)
                    .await
                    .map_err(|_| RequestError::Closed)?;
                if n == 0 {
                    break;
                }
                body.extend_from_slice(&chunk[..n]);
            }
            body.truncate(len);
        }
    }
    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

// ==================== 分派（与传输无关） ====================

/// 进程内载荷的**交付分类**
///
/// `PluginPayload` 的 3 个变体按「怎么交付给对端」分成两类，分界就是本类型。
enum PayloadDelivery {
    /// 空载荷：无内容可交付
    ///
    /// 单列一个变体（而不是折叠进 `Once(Value::Null)`）是因为**两个入口对它的
    /// 处置确实不同**：HTTP 把它写成 `null` 响应体，WS 直接关连接、不发数据帧。
    /// 折叠会让 WS 从「静默关闭」变成「先发一个 null 帧再关」——那是行为变化，
    /// 不是收口。收口的目标是**分类只有一份**，不是**把差异抹平**。
    Empty,
    /// 一次性响应：已经序列化好的载荷数据
    Once(Value),
    /// 长连接会话：**消费方式由调用方决定**
    ///
    /// HTTP 入口把它折叠为「最后一帧」（一次性语义），WS 入口做双向转发——
    /// 这两种消费方式是**真差异**（HTTP 没有双向能力），故不强行统一；
    /// 统一的只是**分类**：哪些载荷是长连接、哪些是一次性。
    Stream(PluginChannel),
}

/// 把路由返回的载荷分类（`PluginPayload` 的 3 态在此穷尽）
///
/// 这是「进程内载荷 → 线上交付」的**唯一**分类实现。从前它写在两个传输入口里
/// （HTTP 与 WS 各一份），于是新增 / 删除一个载荷变体要改三处（枚举 + 两个入口），
/// 漏一处就出现「HTTP 能调、WS 不能调」这类**不对称**——而那种 bug 不会让任何
/// 测试变红，只会在用户手里显形。
///
/// ⚠️ 本函数**无拒绝分支**：3 个变体全有去处。这正是删掉死变体 `Native` 之后的
/// 收益——从前这里必须为它写一条「不支持跨传输」的 `Err`，让「载荷有几种形态」
/// 这个基础问题没有唯一答案。
fn classify_payload(payload: PluginPayload) -> Result<PayloadDelivery, String> {
    match payload {
        PluginPayload::Data(d) => Ok(PayloadDelivery::Once(
            d.serialize().map_err(|e| e.to_string())?,
        )),
        PluginPayload::Empty => Ok(PayloadDelivery::Empty),
        PluginPayload::Session(chan) => Ok(PayloadDelivery::Stream(chan)),
    }
}

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
    // `trace_id`：002-ERROR_CODES.md 约定「每个请求的 metadata 应包含 trace_id，后端会记录在
    // 日志中」。壳层 `route_v2` 一直记着，网关这侧（CLI / e2e / 第三方客户端的唯一入口）
    // 此前连读都不读——**链路恰好断在它身上**。缺席统一写 `-`：字段恒在，日志行的形状才
    // 稳定（与壳层 `origin` 的缺省同一取向）；键名取自键面（`TRACE_ID.name()`），
    // 协议 `metadata` 与上下文键是同一张表，两边各写一遍字面量就是两份真相。
    let trace_id = msg
        .metadata
        .get(TRACE_ID.name())
        .and_then(|v| v.as_str())
        .unwrap_or("-");

    if readonly && !is_readonly_allowed(path, &msg.payload) {
        warn!(trace_id = %trace_id, path = %path, "只读模式下拒绝调用");
        return Err(format!("只读模式下禁止调用: {path}"));
    }

    let ctx = build_ctx(msg);
    // 逐请求一行走 `debug`：这是一条**访问日志**，默认过滤级别（`symbio=debug`）下照记，
    // 但把控制台留给 `info` 级的事件。失败必须升到 `error`——那才是需要顺着 trace_id
    // 回查的那一刻。
    debug!(trace_id = %trace_id, path = %path, "网关路由");
    let payload = router.clone().route(ctx).await.map_err(|e| {
        error!(trace_id = %trace_id, path = %path, error = %e, "网关路由失败");
        e.to_string()
    })?;

    match classify_payload(payload)? {
        PayloadDelivery::Empty => Ok(PluginPayloadWire::Data(Value::Null)),
        PayloadDelivery::Once(value) => Ok(PluginPayloadWire::Data(value)),
        PayloadDelivery::Stream(mut chan) => {
            // 一次性语义：消费到 EOF，保留最后一帧；永不 EOF 的订阅由超时兜底。
            //
            // 保留 `Arc` 而不是每帧 `(*v).clone()`：后者是整棵 JSON 树的深拷贝，
            // 在一个「只关心最后一帧」的循环里付 N 次深拷贝毫无意义。
            let mut last: Option<Arc<Value>> = None;
            let mut failed = None;
            loop {
                match tokio::time::timeout(Duration::from_secs(30), chan.rx.recv()).await {
                    Ok(Some(PluginFrame::Data(v))) => last = Some(v),
                    Ok(Some(PluginFrame::Error(m, _))) => {
                        failed = Some(m);
                        break;
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }
            // 一次性调用把流「用掉」了，但**订阅本身**还挂在总线全局表里；
            // 不取消令牌，`event_bus` 的清理任务永远不会反注册。
            chan.cancel_token.cancel();
            if let Some(message) = failed {
                return Err(message);
            }
            let value = match last {
                // 独占时直接解包（零拷贝）；仍被别处持有才退化为深拷贝。
                Some(a) => Arc::try_unwrap(a).unwrap_or_else(|shared| (*shared).clone()),
                None => Value::Null,
            };
            Ok(PluginPayloadWire::Data(value))
        }
    }
}

/// 由 `PluginMessageWire` 构造与 `route_v2` 完全一致的 `PluginSimpleRequest`
///
/// 装配本身在 core（[`PluginSimpleRequest::from_parts`]）：桌面命令层走同一份，
/// 两个入口看到的 metadata 类型不能分叉。
fn build_ctx(msg: &PluginMessageWire) -> Arc<dyn PluginInvokeRequest> {
    Arc::new(PluginSimpleRequest::from_parts(
        msg.payload.clone(),
        msg.metadata.as_object(),
    ))
}

// ==================== SSE 实时面 ====================

/// 一条总线信封 → 一条 SSE 事件。
///
/// 信封**原样**过（`{kind, session_id, data}`）：SSE 是 `event_bus/subscribe` 的
/// 只读包装，不重新解释 `kind`——消费方按信封里的 `kind` 自行分派，与 WS 客户端
/// 看到的是同一份数据。不写 `event:` 行：带了具名事件后，`EventSource.onmessage`
/// 不再触发，默认用法反而看不全。
fn sse_data_line(value: &Value) -> Vec<u8> {
    let text = serde_json::to_string(value).unwrap_or_else(|_| "null".to_string());
    let mut out = Vec::with_capacity(text.len() + 8);
    // 紧凑序列化不含换行；分行写是防御性口径——将来若改 pretty 输出，
    // 裸换行会把一条事件提前截断，而 SSE 允许同一条事件带多行 `data:`。
    for line in text.lines() {
        out.extend_from_slice(b"data: ");
        out.extend_from_slice(line.as_bytes());
        out.push(b'\n');
    }
    out.push(b'\n');
    out
}

/// `GET /api/v1/events`：订阅总线并把帧逐条写成 SSE。
///
/// ## 它只是传输，不是第二个信源
///
/// 订阅拿到的与 WS 完全相同的帧；**要收到会话流仍需另走 `vdfs/watch`**
/// （总线只是广播口，见 DE-013 §5.3）。本端点不登记 watch，也不按 `kind` 过滤。
///
/// ## 断线必须主动取消订阅
///
/// `event_bus` 的清理任务在 `PluginChannel::cancel_token` 上等待反注册；只 drop
/// 通道不会触发它（令牌 drop 不等于 cancel）。不主动取消就会留下幽灵订阅者：
/// 广播继续往一条没人读的通道里写，最终只能靠 resync 空转。
///
/// ## 断线为什么读半边
///
/// SSE 是**只写**通道：客户端不发任何东西，因此「对端已断开」只有两种时刻可见——
/// 下一次写失败，或读到 EOF。只靠写（心跳）意味着断开后要空挂一整个心跳周期；
/// 这里同时 select 读半边，FIN 一到就收口（与 WS 的读循环同一判据）。
async fn handle_sse(stream: TcpStream, router: Arc<dyn Plugin>, cancel: CancellationToken) {
    let (mut read_half, mut write_half) = stream.into_split();
    let ctx = Arc::new(PluginSimpleRequest::new(None, None));
    ctx.set(PATH, ROUTE_EVENT_BUS_SUBSCRIBE.to_string());
    if let Err(e) = ctx.set_payload(EventBusSubscribeRequest {}) {
        let b = format!("{{\"error\":\"{e}\"}}").into_bytes();
        let _ = write_half
            .write_all(&http_response(
                500,
                "Internal Server Error",
                "application/json",
                &b,
                &cors_headers(),
            ))
            .await;
        return;
    }
    let mut chan = match Arc::clone(&router).route(ctx).await {
        Ok(PluginPayload::Session(c)) => c,
        Ok(_) => {
            let b = "{\"error\":\"event_bus/subscribe 返回了非会话载荷\"}";
            let _ = write_half
                .write_all(&http_response(
                    500,
                    "Internal Server Error",
                    "application/json",
                    b.as_bytes(),
                    &cors_headers(),
                ))
                .await;
            return;
        }
        Err(e) => {
            let b = format!("{{\"error\":\"{e}\"}}").into_bytes();
            let _ = write_half
                .write_all(&http_response(
                    500,
                    "Internal Server Error",
                    "application/json",
                    &b,
                    &cors_headers(),
                ))
                .await;
            return;
        }
    };

    // SSE 没有 `Content-Length`（响应体是无限长的事件流），因此不走
    // `http_response`；`Connection: keep-alive` 与 `X-Accel-Buffering: no`
    // 分别挡住「HTTP/1.1 默认长连接」与 nginx 一类反向代理的响应缓冲。
    let head = "HTTP/1.1 200 OK\r\n\
                Content-Type: text/event-stream\r\n\
                Cache-Control: no-cache\r\n\
                X-Accel-Buffering: no\r\n\
                Access-Control-Allow-Origin: *\r\n\
                Connection: keep-alive\r\n\r\n";
    if write_half.write_all(head.as_bytes()).await.is_err() {
        chan.cancel_token.cancel();
        return;
    }
    if write_half.flush().await.is_err() {
        chan.cancel_token.cancel();
        return;
    }

    // 客户端在 SSE 上发来的字节没有语义（请求体已读完），读到的内容一律丢弃；
    // 这里要的只是 EOF / 错误。
    let mut scratch = [0u8; 256];
    loop {
        let frame = tokio::select! {
            _ = cancel.cancelled() => break,
            r = read_half.read(&mut scratch) => {
                match r {
                    Ok(0) | Err(_) => break,
                    // 客户端发了点什么：与本流无关，继续等事件。
                    Ok(_) => continue,
                }
            }
            r = tokio::time::timeout(SSE_HEARTBEAT, chan.rx.recv()) => r,
        };
        let chunk = match frame {
            // 空闲：注释行保活。写失败 = 对端已走，下一轮 select 也会看到取消 / EOF，
            // 但这里直接收口更省一次往返。
            Err(_elapsed) => b": ping\n\n".to_vec(),
            Ok(Some(PluginFrame::Data(v))) => sse_data_line(&v),
            Ok(Some(PluginFrame::Error(message, _))) => {
                let payload = serde_json::json!({ "error": message });
                // 终局：错误帧发完就结束这次订阅，写不出去也没有后续动作可做。
                let _ = write_half.write_all(&sse_data_line(&payload)).await; // grep-audit-allow S-002-bonus: 终局写，失败即对端已走
                break;
            }
            // 发布方已关闭：没有更多事件，正常结束。
            Ok(None) => break,
        };
        if write_half.write_all(&chunk).await.is_err() || write_half.flush().await.is_err() {
            break;
        }
    }

    // 先摘订阅再关连接：反注册不依赖 TCP 的 FIN 时序。
    chan.cancel_token.cancel();
    let _ = write_half.shutdown().await; // grep-audit-allow S-002-bonus: 关闭写半边是收尾动作，失败即对端已走
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
    let subprotocol = ws_select_subprotocol(req)
        .map(|p| format!("Sec-WebSocket-Protocol: {p}\r\n"))
        .unwrap_or_default();
    let resp = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n{subprotocol}\r\n"
    );
    stream
        .write_all(resp.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

async fn handle_ws(
    stream: TcpStream,
    router: Arc<dyn Plugin>,
    readonly: bool,
    cancel: CancellationToken,
) {
    let (mut read_half, mut write_half) = stream.into_split();

    // 第一帧：请求体 PluginMessageWire（与 route_v2 的请求体一致）。
    // 停用竞态与 HTTP 侧同判：连接已建立但还没有请求内容 ⇒ 直接关，不进入分派。
    let text = tokio::select! {
        _ = cancel.cancelled() => {
            ws_close(&mut write_half).await;
            return;
        }
        t = ws_read_text(&mut read_half) => match t {
            Some(t) => t,
            None => {
                ws_close(&mut write_half).await;
                return;
            }
        },
    };
    let msg: PluginMessageWire = match serde_json::from_str(&text) {
        Ok(m) => m,
        Err(e) => {
            let _ = ws_send_text(
                &mut write_half,
                &format!("{{\"Error\":[\"invalid request: {e}\"]}}"),
            )
            .await;
            ws_close(&mut write_half).await;
            return;
        }
    };
    let path = msg
        .metadata
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if readonly && !is_readonly_allowed(path, &msg.payload) {
        let _ = ws_send_text(
            &mut write_half,
            &format!("{{\"Error\":[\"只读模式下禁止调用: {path}\"]}}"),
        )
        .await;
        ws_close(&mut write_half).await;
        return;
    }

    let ctx = build_ctx(&msg);
    // 路由错误与载荷序列化错误合流为同一条错误帧路径——对 WS 而言两者无差别。
    //
    // 顺带修掉一处**既有的不一致**：从前本入口对 `Data` 的序列化失败用
    // `unwrap_or(Value::Null)` **静默降级**，而 HTTP 入口（`dispatch_once`）是
    // `?` **报错**。同一个载荷、两种传输，行为不同——这正是「分类写了两份」的
    // 代价。收口后统一为报错（错误不该静默）。
    let delivery = router
        .route(ctx)
        .await
        .map_err(|e| e.to_string())
        .and_then(classify_payload);
    match delivery {
        Ok(PayloadDelivery::Stream(chan)) => {
            let channel_token = chan.cancel_token.clone();
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
                    _ = cancel.cancelled() => break,
                    r = ws_read_frame(&mut read_half) => {
                        match r {
                            Some((op, data)) => {
                                if op == 0x8 { break; } // close
                                else if op == 0x9 {
                                    // 回 pong 失败 = 对端已经走了；下一轮读帧会拿到 EOF
                                    // 而 break，故此处不必单独处理。
                                    let _ = ws_send_frame(&mut write_half, 0xA, &data).await; // ping → pong // grep-audit-allow S-002-bonus: 对端已走，下一轮读帧会 EOF 退出
                                } else if let Ok(f) = serde_json::from_slice::<PluginFrame>(&data) {
                                    if tx.send(f).await.is_err() { break; }
                                }
                            }
                            None => break,
                        }
                    }
                }
            }
            // 与 SSE 同一条理由：连接结束不等于订阅结束，令牌不取消就留下幽灵订阅者。
            channel_token.cancel();
            ws_close(&mut write_half).await;
        }
        Ok(PayloadDelivery::Once(value)) => {
            let frame = PluginFrame::data(value);
            let text = serde_json::to_string(&frame).unwrap_or_default();
            // 响应帧发不出去 = 对端已消失；关连接是唯一的后续动作，没有别的可做。
            let _ = ws_send_text(&mut write_half, &text).await; // grep-audit-allow S-002-bonus: 对端已消失，关连接即唯一后续
            ws_close(&mut write_half).await;
        }
        Ok(PayloadDelivery::Empty) => {
            ws_close(&mut write_half).await;
        }
        Err(e) => {
            // 错误帧同样：发得出去就发，发不出去（对端已消失）也只能关连接。
            let _ = ws_send_text(&mut write_half, &format!("{{\"Error\":[\"{e}\"]}}")).await; // grep-audit-allow S-002-bonus: 对端已消失，关连接即唯一后续
            ws_close(&mut write_half).await;
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
        // u64 → usize：32 位平台上超出 usize 的帧长在这里就被拒（不静默截断）
        len = usize::try_from(u64::from_be_bytes(b)).ok()?;
    }
    // 上限必须在**分配之前**判：`vec![0u8; len]` 会照着帧长立即分配，
    // len = 2^60 足以当场 OOM——而这个数是客户端完全可控的。
    if len > MAX_WS_FRAME_BYTES {
        warn!("[gateway] WebSocket 帧超限（{len} > {MAX_WS_FRAME_BYTES}），断开连接");
        return None;
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

/// 关闭 WS 写半边（best-effort，**失败无需处理**）。
///
/// 会失败的情形只有「对端已经消失」——对端都没了，`shutdown` 失败也就没有别的
/// 动作可做。本函数存在的意义是**把这条理由写一次**：`handle_ws` 有 8 个收尾点
/// 都要关连接，逐个写 `let _ = write_half.shutdown().await;` 会让「这里为什么
/// 可以吞错」散成 8 份没有解释的 `let _`，看起来像随手丢错误。
async fn ws_close<W: AsyncWrite + Unpin>(write_half: &mut W) {
    let _ = write_half.shutdown().await; // grep-audit-allow S-002-bonus: 对端已消失时关闭失败，无可为
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

#[cfg(test)]
#[path = "server.test.rs"]
mod tests;
