//! `crates/symbio-plugin-gateway/src/server.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

use super::*;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn req_with(headers: &[(&str, &str)]) -> HttpRequest {
    HttpRequest {
        method: "GET".into(),
        path: "/".into(),
        headers: headers
            .iter()
            .map(|(k, v)| (k.to_lowercase(), v.to_string()))
            .collect(),
        body: Vec::new(),
    }
}

// ---- SHA-1 与握手 ----

/// SHA-1 已知答案。算法一旦被改坏，握手会**静默**失败（表现为"客户端连不上"，
/// 极难定位），故钉死在这里。
#[test]
fn sha1_matches_known_vectors() {
    assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    assert_eq!(
        hex(&sha1(b"The quick brown fox jumps over the lazy dog")),
        "2fd4e1c67a2d28fced849ee1bb76e7391b93eb12"
    );
}

/// RFC 6455 §1.3 官方示例：Accept 必须由 `key + GUID` 派生。
#[test]
fn websocket_accept_matches_rfc6455_example() {
    let key = "dGhlIHNhbXBsZSBub25jZQ==";
    let accept = base64::engine::general_purpose::STANDARD
        .encode(sha1(format!("{key}{WS_GUID}").as_bytes()));
    assert_eq!(accept, "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
}

// ---- 鉴权 / 查询参数 ----

/// 未配置 token = 不启用鉴权（放行）；配置了就必须带对，且不得凭前缀放行。
#[test]
fn check_auth_requires_the_configured_bearer_token() {
    let none = req_with(&[]);
    assert!(check_auth("", &none), "未配置 token 时不得拦截");

    let good = req_with(&[("authorization", "Bearer s3cret")]);
    assert!(check_auth("s3cret", &good));
    // 头值两侧空白应被容忍
    let padded = req_with(&[("authorization", "  Bearer s3cret  ")]);
    assert!(check_auth("s3cret", &padded));

    assert!(!check_auth("s3cret", &none), "缺失 Authorization 必须拒绝");
    assert!(
        !check_auth("s3cret", &req_with(&[("authorization", "Bearer nope")])),
        "错误 token 必须拒绝"
    );
    assert!(
        !check_auth(
            "s3cret",
            &req_with(&[("authorization", "Bearer s3cret-and-more")])
        ),
        "不得凭前缀放行"
    );
}

// ---- 非回环绑定的安全护栏 ----

/// 谓词四象限：**回环可空令牌、非回环必填**。
///
/// 两个方向都得钉——只钉拒绝方向的话，把条件写反（一律拒绝）也能通过，
/// 而那会让网关再也起不来，且没有哪条测试会说话。
#[test]
fn require_token_for_non_loopback_is_the_documented_boundary() {
    // 回环 + 空令牌：有意的本机开发形态，必须放行（config.rs 默认就是这个组合）
    for bind in ["127.0.0.1", "localhost", "::1"] {
        assert!(
            require_token_for_non_loopback(bind, "").is_ok(),
            "{bind} + 空令牌应放行（本机开发形态）"
        );
    }
    // 非回环 + 空令牌：拒绝
    for bind in ["0.0.0.0", "192.168.1.10", "::"] {
        let err = require_token_for_non_loopback(bind, "").expect_err("非回环 + 空令牌必须被拒绝");
        assert!(err.contains("必须设置访问令牌"), "{err}");
    }
    // 两个「带令牌」的象限：放行（对外暴露的正常形态 / 回环不因设了令牌反而被拒）
    assert!(require_token_for_non_loopback("0.0.0.0", "s3cret").is_ok());
    assert!(require_token_for_non_loopback("127.0.0.1", "s3cret").is_ok());
}

/// **接线断言**：谓词写得再对，`start` 不调它也等于没有。
///
/// 走真实入口 [`start`]：护栏必须在 `TcpListener::bind` **之前**返回。若它被整个删掉，
/// `start` 会绑上 `0.0.0.0:0` 并返回 `Ok`，下面的 `panic!` 即触发；若被挪到 bind 之后，
/// 端口会真的开出去（仍红）。`inbound_port: 0` 是为了万一守卫失效也不与他人抢端口。
///
/// 用 `match` 而非 `expect_err`：`ServerHandle` 没有 `impl Debug`，而 `expect_err`
/// 要求成功侧可打印。
#[tokio::test]
async fn start_refuses_non_loopback_bind_without_token() {
    let cfg = GatewayConfig {
        inbound_enabled: true,
        inbound_protocol: "http".into(),
        inbound_bind: "0.0.0.0".into(),
        inbound_port: 0,
        inbound_token: String::new(),
        ..Default::default()
    };
    match start(&cfg, Arc::new(NopRouter)).await {
        Err(err) => assert!(err.contains("必须设置访问令牌"), "{err}"),
        Ok(_) => panic!("非回环 + 空令牌必须拒绝启动，护栏被删掉或挪到 bind 之后了"),
    }
}

/// 什么都答 `NotFound` 的占位路由——护栏分支根本用不到它，但 [`start`] 的签名要
/// `Arc<dyn Plugin>`。刻意**不复用别处的 NopPlugin**：测试夹具跨模块引用会让
/// 两边的改动互相牵连。
struct NopRouter;

#[async_trait::async_trait]
impl Plugin for NopRouter {
    fn meta(&self) -> symbio_plugin_api::PluginMeta {
        symbio_plugin_api::PluginMeta::new("nop-router", "nop")
    }

    async fn route(
        self: Arc<Self>,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> symbio_plugin_api::PluginInvokeResponse<symbio_plugin_api::PluginPayload> {
        Err(symbio_plugin_api::PluginError::NotFound("nop".to_string()))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> symbio_plugin_api::PluginInvokeResponse<symbio_plugin_api::PluginPayload> {
        Ok(symbio_plugin_api::PluginPayload::new(&Vec::<
            serde_json::Value,
        >::new()))
    }
}

// ---- WS 令牌与子协议 ----

/// 浏览器只能经子协议携带令牌；非浏览器用 `Authorization`。两者都在时以头为准。
#[test]
fn ws_token_reads_authorization_then_subprotocol() {
    let auth = req_with(&[("authorization", "Bearer s3cret")]);
    assert_eq!(ws_token(&auth).as_deref(), Some("s3cret"));

    let sub = req_with(&[("sec-websocket-protocol", "symbio.v1, bearer.s3cret")]);
    assert_eq!(ws_token(&sub).as_deref(), Some("s3cret"));

    // 头优先：子协议里的坏令牌不应盖掉一个正确的头
    let both = req_with(&[
        ("authorization", "Bearer good"),
        ("sec-websocket-protocol", "bearer.bad"),
    ]);
    assert_eq!(ws_token(&both).as_deref(), Some("good"));

    assert_eq!(ws_token(&req_with(&[])), None);
    assert_eq!(
        ws_token(&req_with(&[("sec-websocket-protocol", "symbio.v1")])),
        None,
        "只报协议名时不应当成令牌"
    );
}

/// 握手只能回选 `symbio.v1`：回显 `bearer.*` 等于把令牌写进响应头。
#[test]
fn ws_select_subprotocol_never_echoes_bearer() {
    let offered = req_with(&[("sec-websocket-protocol", "symbio.v1, bearer.s3cret")]);
    assert_eq!(ws_select_subprotocol(&offered), Some("symbio.v1"));

    let bearer_only = req_with(&[("sec-websocket-protocol", "bearer.s3cret")]);
    assert_eq!(ws_select_subprotocol(&bearer_only), None);

    assert_eq!(ws_select_subprotocol(&req_with(&[])), None);
}

// ---- 幂等表 ----

/// 同键未释放 ⇒ 第二次拒绝；释放后（分派失败）⇒ 可重试。
#[test]
fn idempotency_rejects_duplicates_until_released() {
    let table = Idempotency::new();
    let ticket = table.reserve("k-1").expect("首次登记应成功");
    assert!(table.reserve("k-1").is_none(), "同键未释放时应拒绝");
    assert!(table.reserve("k-2").is_some(), "不同键互不影响");

    ticket.release();
    assert!(
        table.reserve("k-1").is_some(),
        "释放后同键应可重试（失败没有副作用）"
    );
}

/// 过期项在登记时被清掉：TTL 之后同一个键是新的请求，不是重放。
#[test]
fn idempotency_forgets_entries_past_ttl() {
    // 夹具写成「未来的 `now`」而不是「过去的记录」：`Instant` 的零点是开机，
    // `Instant::now() - IDEMPOTENCY_TTL` 在开机不满 600s 的机器上直接 panic，
    // 于是这条判据在新 VM / CI runner 上变成假失败（见 `Idempotency::reserve_at`）。
    let table = Idempotency::new();
    let stored = Instant::now();
    {
        let mut seen = table.seen.lock().unwrap_or_else(|e| e.into_inner());
        seen.insert("stale".to_string(), stored);
    }
    assert!(
        table
            .reserve_at("stale", stored + IDEMPOTENCY_TTL + Duration::from_secs(1))
            .is_some(),
        "过期键不应再算重放"
    );
}

/// 满表时淘汰最旧的一条而不是拒绝新键——否则任何人用一次性键就能永久占满。
#[test]
fn idempotency_evicts_oldest_when_full() {
    // 同上：所有时间戳都从 `base` 往后挪，`now` 落在夹具之后 500s（仍在 TTL 内，
    // 所以「淘汰」是被**容量**触发的，不是被过期清理顺带做的）。
    let table = Idempotency::new();
    let base = Instant::now();
    {
        let mut seen = table.seen.lock().unwrap_or_else(|e| e.into_inner());
        for i in 0..IDEMPOTENCY_CAPACITY - 1 {
            seen.insert(format!("old-{i}"), base + Duration::from_secs(10));
        }
        seen.insert("oldest".to_string(), base);
    }
    assert!(
        table
            .reserve_at("fresh", base + Duration::from_secs(500))
            .is_some(),
        "满表也必须接纳新键"
    );
    let seen = table.seen.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(seen.len(), IDEMPOTENCY_CAPACITY, "淘汰后容量仍应守住上限");
    assert!(!seen.contains_key("oldest"), "淘汰的应是最旧的一条");
}

/// 键的取值面向代理与日志：空白、控制字符、超长与空串一律不认。
#[test]
fn idempotency_key_shape_is_bounded() {
    assert!(valid_idempotency_key(
        "550e8400-e29b-41d4-a716-446655440000"
    ));
    assert!(!valid_idempotency_key(""));
    assert!(!valid_idempotency_key("has space"));
    assert!(!valid_idempotency_key("line\nbreak"));
    assert!(!valid_idempotency_key(
        &"a".repeat(MAX_IDEMPOTENCY_KEY_BYTES + 1)
    ));
}

// ---- 连接上限 ----

/// `0` 不能把服务变成「不可用」，越界也不能让 `Semaphore::new` panic。
#[test]
fn connection_cap_treats_zero_as_one_and_clamps() {
    assert_eq!(connection_cap(0), 1);
    assert_eq!(connection_cap(1), 1);
    assert_eq!(connection_cap(64), 64);
    assert_eq!(connection_cap(usize::MAX), MAX_CONNECTION_CAP);
}

// ---- HTTP 响应 ----

/// 响应必须声明并**与实际体一致**的 Content-Length，并带 CORS。
#[test]
fn http_response_declares_length_and_cors() {
    let resp = http_response(
        200,
        "OK",
        "application/json",
        b"{\"ok\":true}",
        &cors_headers(),
    );
    let text = String::from_utf8(resp).expect("响应应为合法 UTF-8");
    assert!(text.starts_with("HTTP/1.1 200 OK\r\n"));
    assert!(text.contains("Content-Length: 11\r\n"));
    assert!(text.contains("Access-Control-Allow-Origin: *\r\n"));
    assert!(text.ends_with("{\"ok\":true}"));
}

// ---- 请求解析与上限闸门 ----

#[tokio::test]
async fn read_request_parses_request_line_headers_and_body() {
    let raw = "POST /api/v1/invoke?x=1 HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: 7\r\n\r\n{\"a\":1}";
    let mut reader = raw.as_bytes();
    let req = read_request(&mut reader).await.ok().expect("应解析成功");
    assert_eq!(req.method, "POST");
    assert_eq!(req.path, "/api/v1/invoke?x=1");
    assert_eq!(
        req.headers.get("content-type").map(String::as_str),
        Some("application/json"),
        "头名应统一小写以便按名取值"
    );
    assert_eq!(req.body, b"{\"a\":1}");
}

/// 头是逐字节读的：不设上限时，只发头不发全的连接能长期占住内存与任务。
#[tokio::test]
async fn read_request_rejects_oversized_headers() {
    let mut raw = b"GET / HTTP/1.1\r\nX-Big: ".to_vec();
    raw.extend(std::iter::repeat_n(b'a', MAX_HEADER_BYTES + 16));
    let mut reader = raw.as_slice();
    assert!(
        matches!(
            read_request(&mut reader).await,
            Err(RequestError::TooLarge(_))
        ),
        "超长请求头应被拒"
    );
}

/// `Content-Length` 声明超限时，必须在**读体之前**就拒——否则单个请求
/// （`Content-Length: 999999999999`）即可耗尽进程内存。此处只发头、不发体：
/// 若能返回 TooLarge 而非 Closed，即证明它没有去读那个巨大的体。
#[tokio::test]
async fn read_request_rejects_oversized_body_declaration() {
    let raw = format!(
        "POST /x HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
        MAX_BODY_BYTES + 1
    );
    let mut reader = raw.as_bytes();
    assert!(matches!(
        read_request(&mut reader).await,
        Err(RequestError::TooLarge(_))
    ));
}

/// 对端中途走掉（报文不完整）是静默关闭，不是错误。
#[tokio::test]
async fn read_request_reports_closed_on_incomplete_input() {
    let mut reader = b"GET / HTTP/1.1\r\nHost: x\r\n".as_slice(); // 少了结尾空行
    assert!(matches!(
        read_request(&mut reader).await,
        Err(RequestError::Closed)
    ));
}

// ---- WebSocket 帧编解码 ----

/// 客户端帧带掩码：解出的必须是原文。
#[tokio::test]
async fn ws_read_frame_decodes_masked_text() {
    let payload = b"hello";
    let mask = [0x12u8, 0x34, 0x56, 0x78];
    let mut frame = vec![0x81, 0x80 | payload.len() as u8];
    frame.extend_from_slice(&mask);
    frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));

    let mut reader = frame.as_slice();
    let (op, data) = ws_read_frame(&mut reader).await.expect("应解出一帧");
    assert_eq!(op, 0x1);
    assert_eq!(data, payload);
}

/// 帧长由客户端给出，`vec![0u8; len]` 会照着这个数**立即分配**。
/// 这里只给 10 字节头、声明 2^40，若上限判定被挪到分配之后，本测试进程会
/// 因 1 TiB 分配失败而 abort；能干净地返回 `None` 即证明**未发生分配**。
#[tokio::test]
async fn ws_read_frame_rejects_oversized_length_before_allocating() {
    let mut frame = vec![0x81, 0x7F]; // 未掩码 + 8 字节扩展长度
    frame.extend_from_slice(&(1u64 << 40).to_be_bytes());
    let mut reader = frame.as_slice();
    assert!(ws_read_frame(&mut reader).await.is_none());
}

/// 服务端出帧不带掩码；三种长度编码（<126 / <65536 / ≥65536）都要能原样读回。
#[tokio::test]
async fn ws_frames_roundtrip_through_all_length_forms() {
    for len in [0usize, 125, 126, 65535, 65536] {
        let payload = vec![b'z'; len];
        let mut out = Vec::new();
        ws_send_frame(&mut out, 0x1, &payload)
            .await
            .expect("写帧失败");

        let mut reader = out.as_slice();
        let (op, data) = ws_read_frame(&mut reader).await.expect("应读回一帧");
        assert_eq!(op, 0x1);
        assert_eq!(data.len(), len, "长度为 {len} 的帧未能原样往返");
        assert_eq!(data, payload);
    }
}

/// 停用 ⇒ 监听真的关了。
///
/// 判据用**独立探针**（另一条 `TcpStream`）而不是问插件「你停了吗」：先证明服务
/// 活着（健康检查回 200），再 `stop`，最后证明新连接**连不上**。`inbound_port: 0`
/// 让内核分配端口，实际地址从 `local_addr()` 读——测试因此不抢端口、不依赖 9231。
#[tokio::test]
async fn stop_closes_the_listener_and_refuses_new_connections() {
    let cfg = GatewayConfig {
        inbound_enabled: true,
        inbound_protocol: "http".into(),
        inbound_bind: "127.0.0.1".into(),
        inbound_port: 0,
        ..Default::default()
    };
    let handle = start(&cfg, Arc::new(NopRouter))
        .await
        .expect("回环 + 端口 0 应能启动")
        .expect("enabled + http 必须返回句柄");
    let addr = handle.local_addr();

    // 探针 1：服务在听且能答（先证明这不是「一开始就没起来」）
    let mut live = TcpStream::connect(addr).await.expect("启动后应可连接");
    live.write_all(b"GET /api/v1/health HTTP/1.1\r\nHost: probe\r\n\r\n")
        .await
        .expect("写入健康检查请求");
    let mut buf = [0u8; 256];
    let n = live.read(&mut buf).await.expect("读取健康检查响应");
    assert!(
        String::from_utf8_lossy(&buf[..n]).starts_with("HTTP/1.1 200"),
        "启动后健康检查应回 200"
    );
    drop(live);

    stop(handle);

    // 探针 2：停用后新连接必须连不上（accept 循环已退出、监听已释放）
    let refused = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if TcpStream::connect(addr).await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(
        refused.is_ok(),
        "停用后监听仍然接受连接——accept 循环没有被收口"
    );
}

// ---- SSE 实时面（`GET /api/v1/events`） ----

/// 只对 `event_bus/subscribe` 应答的订阅路由：发一条信封后关闭通道。
///
/// 用**真实通道**而不是 mock 流：SSE 处理器的职责就是「路由拿到 Session 后逐帧转发」，
/// 换成假流就等于把这段接线测掉了。`mine` 在返回时 drop，帧已入缓冲，
/// 对端因此读到一条帧、随后 EOF——正好是一条最短的订阅生命周期。
struct OneFrameRouter;

#[async_trait::async_trait]
impl Plugin for OneFrameRouter {
    fn meta(&self) -> symbio_plugin_api::PluginMeta {
        symbio_plugin_api::PluginMeta::new("one-frame-router", "one")
    }

    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> symbio_plugin_api::PluginInvokeResponse<symbio_plugin_api::PluginPayload> {
        let path = ctx.get(symbio_plugin_api::PATH).unwrap_or_default();
        if path != symbio_plugin_api::routes::ROUTE_EVENT_BUS_SUBSCRIBE {
            return Err(symbio_plugin_api::PluginError::NotFound(path));
        }
        let (peer, mine) = symbio_plugin_api::PluginChannel::pair(4);
        mine.tx
            .send(symbio_plugin_api::PluginFrame::data(serde_json::json!({
                "type": "bus_event",
                "data": {"kind": "system", "session_id": null, "data": {"event": "connected"}},
            })))
            .await
            .map_err(|e| symbio_plugin_api::PluginError::InternalError(e.to_string()))?;
        Ok(symbio_plugin_api::PluginPayload::Session(peer))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> symbio_plugin_api::PluginInvokeResponse<symbio_plugin_api::PluginPayload> {
        Ok(symbio_plugin_api::PluginPayload::new(&Vec::<
            serde_json::Value,
        >::new()))
    }
}

async fn start_sse_gateway(readonly: bool, token: &str) -> (ServerHandle, std::net::SocketAddr) {
    let cfg = GatewayConfig {
        inbound_enabled: true,
        inbound_protocol: "http".into(),
        inbound_bind: "127.0.0.1".into(),
        inbound_port: 0,
        inbound_token: token.to_string(),
        inbound_readonly: readonly,
        ..Default::default()
    };
    let handle = start(&cfg, Arc::new(OneFrameRouter))
        .await
        .expect("回环 + 端口 0 应能启动")
        .expect("enabled + http 必须返回句柄");
    let addr = handle.local_addr();
    (handle, addr)
}

async fn get_once(addr: std::net::SocketAddr, request: &str) -> String {
    let mut stream = TcpStream::connect(addr).await.expect("应能连接网关");
    stream
        .write_all(request.as_bytes())
        .await
        .expect("写入请求");
    let mut buf = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), stream.read_to_end(&mut buf))
        .await
        .expect("响应应在超时前结束（关闭通道 / Content-Length）")
        .expect("读取响应");
    String::from_utf8_lossy(&buf).into_owned()
}

/// 一条信封 = 一条 SSE 事件：`data:` 行原样承载 JSON，空行结束事件。
#[test]
fn sse_data_line_keeps_one_event_per_envelope() {
    let line = sse_data_line(&serde_json::json!({
        "type": "bus_event",
        "data": {"kind": "vdfs", "session_id": null, "data": {"path": "/x"}},
    }));
    let text = String::from_utf8(line).expect("SSE 输出应是 UTF-8");
    assert!(
        text.starts_with("data: "),
        "事件必须以 data: 行承载：{text}"
    );
    assert!(text.ends_with("\n\n"), "事件必须以空行结束：{text:?}");
    let json: Value =
        serde_json::from_str(text.trim_start_matches("data: ").trim_end()).expect("应能解回 JSON");
    assert_eq!(
        json["data"]["kind"], "vdfs",
        "信封必须原样过，不重新解释 kind"
    );
}

/// 端点真的把总线帧送到 HTTP 边界：状态行 + SSE 内容类型 + 信封正文。
#[tokio::test]
async fn sse_endpoint_streams_the_bus_envelope_verbatim() {
    let (handle, addr) = start_sse_gateway(false, "").await;
    let text = get_once(addr, "GET /api/v1/events HTTP/1.1\r\nHost: probe\r\n\r\n").await;
    assert!(text.starts_with("HTTP/1.1 200 OK\r\n"), "{text}");
    assert!(
        text.contains("Content-Type: text/event-stream"),
        "SSE 必须用 text/event-stream：{text}"
    );
    assert!(text.contains("data: "), "应下发总线信封：{text}");
    assert!(
        text.contains(r#""event":"connected""#),
        "应原样下发 event_bus 的 connected 信封：{text}"
    );
    stop(handle);
}

/// 只读模式不开放实时面：长连接广播会把可读范围扩到全部在途内容。
#[tokio::test]
async fn sse_endpoint_is_forbidden_in_readonly_mode() {
    let (handle, addr) = start_sse_gateway(true, "").await;
    let text = get_once(addr, "GET /api/v1/events HTTP/1.1\r\nHost: probe\r\n\r\n").await;
    assert!(text.starts_with("HTTP/1.1 403 Forbidden"), "{text}");
    assert!(text.contains("只读模式下不开放实时订阅"), "{text}");
    stop(handle);
}

/// 令牌规则与 invoke 同一收起：配了令牌就必须带对，否则 401。
#[tokio::test]
async fn sse_endpoint_requires_the_configured_bearer_token() {
    let (handle, addr) = start_sse_gateway(false, "s3cret").await;
    let denied = get_once(addr, "GET /api/v1/events HTTP/1.1\r\nHost: probe\r\n\r\n").await;
    assert!(denied.starts_with("HTTP/1.1 401 Unauthorized"), "{denied}");
    let allowed = get_once(
        addr,
        "GET /api/v1/events HTTP/1.1\r\nHost: probe\r\nAuthorization: Bearer s3cret\r\n\r\n",
    )
    .await;
    assert!(allowed.starts_with("HTTP/1.1 200 OK"), "{allowed}");
    stop(handle);
}
