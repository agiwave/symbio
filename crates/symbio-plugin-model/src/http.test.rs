//! `http.rs` 的单元测试。
//!
//! 与实现同级分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）。
//! 这里只测传输层的前置闸；真实 HTTP + SSE 闭环由 e2e 的 `mock-llm` 承担。

use super::*;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

/// 0 = 不限制：连续派发不产生任何等待。
#[tokio::test]
async fn rate_gate_zero_never_waits() {
    let gate = RateGate::new(Duration::ZERO);
    let abort = ExecAbortSignal::new();
    for _ in 0..5 {
        assert!(gate.wait_turn(&abort).await);
    }
}

/// 两次派发之间必须等满最小间隔，而不是「超限即报错」。
#[tokio::test]
async fn rate_gate_waits_between_turns() {
    let gate = RateGate::new(Duration::from_millis(80));
    let abort = ExecAbortSignal::new();
    assert!(gate.wait_turn(&abort).await, "首次应立即可发");
    let started = Instant::now();
    assert!(gate.wait_turn(&abort).await);
    assert!(
        started.elapsed() >= Duration::from_millis(60),
        "第二次应等到最小间隔后才放行"
    );
}

/// 等待被中止应立即收口，不能把会话挂到间隔结束。
#[tokio::test]
async fn rate_gate_aborts_while_waiting() {
    let gate = RateGate::new(Duration::from_secs(1));
    let abort = ExecAbortSignal::new();
    assert!(gate.wait_turn(&abort).await); // 占住第一次

    let a2 = abort.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(20)).await;
        a2.abort();
    });
    let started = Instant::now();
    assert!(!gate.wait_turn(&abort).await, "中止应返回 false");
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "abort 必须立即打断等待"
    );
}

/// 并发派发不能一起通过：判定与占位在同一把锁内，抢不到的继续等。
#[tokio::test]
async fn rate_gate_concurrent_turns_do_not_burst() {
    let gate = Arc::new(RateGate::new(Duration::from_millis(80)));
    let abort = ExecAbortSignal::new();
    assert!(gate.wait_turn(&abort).await); // 第一次占住

    let g2 = gate.clone();
    let a2 = abort.clone();
    let task = tokio::spawn(async move { g2.wait_turn(&a2).await });
    let started = Instant::now();
    assert!(task.await.unwrap());
    assert!(
        started.elapsed() >= Duration::from_millis(60),
        "并发的第二次派发不得与第一次一起放行"
    );
}

/// 极简本地服务：第一次 429（`Retry-After: 0`），第二次 200 空流。
/// 用真实 TCP + reqwest 锁住「429 后等待并继续调用，而不是返回限流错误」。
async fn spawn_429_then_ok() -> (String, tokio::task::JoinHandle<usize>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("本地回环端口应可绑定");
    let addr = listener.local_addr().expect("应有本地地址");
    let handle = tokio::spawn(async move {
        let mut hits = 0usize;
        for _ in 0..2 {
            let (mut socket, _) = listener.accept().await.expect("应有对应的连接");
            hits += 1;
            let mut buf = [0u8; 2048];
            let _ = socket.read(&mut buf).await; // 读完请求头即可
            let response = if hits == 1 {
                "HTTP/1.1 429 Too Many Requests\r\nRetry-After: 0\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            } else {
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            };
            socket
                .write_all(response.as_bytes())
                .await
                .expect("响应应写成功");
        }
        hits
    });
    (format!("http://{addr}"), handle)
}

#[tokio::test]
async fn http_429_with_retry_after_waits_and_retries() {
    let (url, server) = spawn_429_then_ok().await;
    let abort = ExecAbortSignal::new();
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        execute_post_with_abort(&url, reqwest::header::HeaderMap::new(), b"{}", &abort),
    )
    .await
    .expect("不应挂死");
    assert!(
        matches!(result, PostResult::Ok(_)),
        "429 后应继续重试直到成功，而不是返回限流错误"
    );
    assert_eq!(
        server.await.expect("服务任务不应 panic"),
        2,
        "应恰好发出两次请求"
    );
}

/// 单次 429，**不带** `Retry-After`：服务端没说何时可用 ⇒ 不能猜等待时间。
async fn spawn_single_429_without_retry_after() -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("本地回环端口应可绑定");
    let addr = listener.local_addr().expect("应有本地地址");
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("应有一次连接");
        let mut buf = [0u8; 2048];
        let _ = socket.read(&mut buf).await;
        let response =
            "HTTP/1.1 429 Too Many Requests\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        socket
            .write_all(response.as_bytes())
            .await
            .expect("响应应写成功");
    });
    format!("http://{addr}")
}

#[tokio::test]
async fn http_429_without_retry_after_reports_error() {
    let url = spawn_single_429_without_retry_after().await;
    let abort = ExecAbortSignal::new();
    let result = tokio::time::timeout(
        Duration::from_secs(5),
        execute_post_with_abort(&url, reqwest::header::HeaderMap::new(), b"{}", &abort),
    )
    .await
    .expect("不应挂死");
    assert!(
        matches!(result, PostResult::RateLimited(_)),
        "无法判断可用时刻时必须报错，不能盲目等待"
    );
}

/// `Retry-After` 的解析边界：只有可解析的值才算「明确时刻」。
#[test]
fn parse_retry_after_knows_what_it_cannot_know() {
    use time::format_description::well_known::Rfc2822;

    let mut headers = reqwest::header::HeaderMap::new();
    assert!(
        parse_retry_after(&headers).is_none(),
        "无 Retry-After = 无法判断"
    );

    headers.insert(reqwest::header::RETRY_AFTER, "5".parse().unwrap());
    assert_eq!(parse_retry_after(&headers), Some(Duration::from_secs(5)));

    headers.insert(reqwest::header::RETRY_AFTER, "garbage".parse().unwrap());
    assert!(parse_retry_after(&headers).is_none(), "解析不了 = 无法判断");

    // HTTP 日期形式（`GMT` 与 `+0000` 都要认）。
    let future = time::OffsetDateTime::now_utc() + time::Duration::seconds(60);
    let date = future
        .format(&Rfc2822)
        .expect("日期应可格式化")
        .replace(" +0000", " GMT");
    headers.insert(reqwest::header::RETRY_AFTER, date.parse().unwrap());
    let wait = parse_retry_after(&headers).expect("HTTP 日期应可解析为可用时刻");
    assert!(
        wait.as_secs() >= 55 && wait.as_secs() <= 61,
        "应换成约 60s 的等待时长，实际 {wait:?}"
    );
}
