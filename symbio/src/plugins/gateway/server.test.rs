//! `symbio/src/plugins/gateway/server.rs` 的单元测试 —— 拆自源码末尾的测试模块。
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

/// 只取指定键；无 `=` 的裸参数不参与匹配，也不得误取成整串。
#[test]
fn query_param_reads_only_the_named_key() {
    assert_eq!(
        query_param("/api/v1/ws?token=abc&x=1", "token").as_deref(),
        Some("abc")
    );
    assert_eq!(query_param("/api/v1/ws?token=abc", "missing"), None);
    assert_eq!(query_param("/api/v1/ws", "token"), None);
    assert_eq!(query_param("/p?flag&token=xy", "flag"), None);
    assert_eq!(
        query_param("/p?flag&token=xy", "token").as_deref(),
        Some("xy")
    );
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
