use super::*;
use crate::worker::broker::BrokerRegistry;
use crate::worker::container::{BrokerKind, BrokerTarget};
use crate::worker::{BuildId, Epoch};
use url::Url;

fn worker() -> WorkerId {
    WorkerId::new(
        "session-test",
        Epoch::new(1).expect("epoch"),
        BuildId::parse("a".repeat(64)).expect("build"),
    )
    .expect("worker")
}

#[test]
fn target_parser_rejects_unknown_kind_and_escape() {
    assert_eq!(
        broker_target("/model/p/chat/completions?x=1").expect("target"),
        (
            "model:p".to_string(),
            "/model/p/chat/completions?x=1".to_string()
        )
    );
    assert_eq!(
        broker_target("/mcp/remote").expect("target"),
        ("mcp:remote".to_string(), "/mcp/remote".to_string())
    );
    assert!(broker_target("/other/p").is_err());
    assert!(broker_target("/model/").is_err());
    assert_eq!(
        broker_target("/subagent/reviewer/model/p/chat/completions").expect("target"),
        (
            "subagent:reviewer/model:p".to_string(),
            "/subagent/reviewer/model/p/chat/completions".to_string()
        )
    );
    assert!(broker_target("/model/%2e%2e").is_err());
}

#[tokio::test]
async fn client_fences_and_validates_chunk_order() {
    let worker = worker();
    let (outgoing, mut frames) = mpsc::channel(8);
    let client = BrokerClient::new(worker.clone(), Arc::new(outgoing), "shim-token");
    let mut stream = client
        .open(
            "model:p".to_string(),
            "POST".to_string(),
            "/model/p".to_string(),
            vec![],
            vec![],
        )
        .await
        .expect("open");
    assert!(matches!(
        frames.recv().await,
        Some(WorkerLine::BrokerOpen { .. })
    ));
    client
        .receive(WorkerLine::BrokerHead {
            worker: worker.clone(),
            stream: 1,
            status: 200,
            headers: vec![],
        })
        .await
        .expect("head");
    assert_eq!(stream.head().await.expect("head").0, 200);
    client
        .receive(WorkerLine::BrokerData {
            worker: worker.clone(),
            stream: 1,
            offset: 0,
            data: BASE64.encode(b"ok"),
        })
        .await
        .expect("data");
    assert_eq!(stream.data().await.expect("data").expect("chunk"), b"ok");
    client
        .receive(WorkerLine::BrokerData {
            worker: worker.clone(),
            stream: 1,
            offset: 0,
            data: BASE64.encode(b"bad"),
        })
        .await
        .expect("stream-local bad chunk");
    assert!(stream.data().await.is_err());
    let stale = WorkerId::new(
        "session-test",
        Epoch::new(2).expect("epoch"),
        BuildId::parse("a".repeat(64)).expect("build"),
    )
    .expect("worker");
    assert!(client
        .receive(WorkerLine::BrokerEnd {
            worker: stale,
            stream: 1
        })
        .await
        .is_err());
    client
        .receive(WorkerLine::BrokerEnd { worker, stream: 1 })
        .await
        .expect("end");
    assert!(matches!(stream.data().await, Ok(None)));
}

#[tokio::test]
async fn data_backpressure_is_stream_local() {
    let worker = worker();
    let (outgoing, mut frames) = mpsc::channel(8);
    let client = BrokerClient::new(worker.clone(), Arc::new(outgoing), "shim-token");
    let mut stream = client
        .open(
            "model:p".to_string(),
            "GET".to_string(),
            "/model/p".to_string(),
            vec![],
            vec![],
        )
        .await
        .expect("open");
    assert!(matches!(
        frames.recv().await,
        Some(WorkerLine::BrokerOpen { .. })
    ));
    for offset in 0..16 {
        client
            .receive(WorkerLine::BrokerData {
                worker: worker.clone(),
                stream: 1,
                offset,
                data: BASE64.encode(b"x"),
            })
            .await
            .expect("queued data");
    }
    let consumer = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        for _ in 0..17 {
            assert_eq!(stream.data().await.expect("data"), Some(b"x".to_vec()));
        }
    });
    client
        .receive(WorkerLine::BrokerData {
            worker,
            stream: 1,
            offset: 16,
            data: BASE64.encode(b"x"),
        })
        .await
        .expect("backpressure does not fail the worker");
    consumer.await.expect("consumer");
}

#[tokio::test]
async fn http_shim_broker_and_upstream_stream_end_to_end() {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    let upstream_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("upstream listener");
    let upstream_address = upstream_listener.local_addr().expect("upstream address");
    let (continue_tx, continue_rx) = oneshot::channel::<()>();
    let upstream = tokio::spawn(async move {
        let (socket, _) = upstream_listener.accept().await.expect("accept");
        let mut reader = BufReader::new(socket);
        let mut request_head = String::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).await.expect("read head");
            if line == "\r\n" {
                break;
            }
            request_head.push_str(&line);
        }
        let length = request_head
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then(|| value.trim().parse::<usize>().expect("length"))
            })
            .expect("content length");
        let mut body = vec![0u8; length];
        reader.read_exact(&mut body).await.expect("read body");
        assert_eq!(body, b"prompt");
        let lower = request_head.to_ascii_lowercase();
        assert!(
            lower.contains("authorization: host-secret\r\n"),
            "{request_head}"
        );
        assert!(lower.contains("post /v1/chat/completions?stream=true"));
        assert!(!lower.contains("worker-secret"));

        let mut socket = reader.into_inner();
        socket
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nset-cookie: host-secret\r\ntransfer-encoding: chunked\r\n\r\n")
            .await
            .expect("response head");
        let first = b"data: one\n\n";
        socket
            .write_all(format!("{:x}\r\n", first.len()).as_bytes())
            .await
            .expect("chunk size");
        socket.write_all(first).await.expect("first chunk");
        socket.write_all(b"\r\n").await.expect("chunk end");
        socket.flush().await.expect("flush");
        continue_rx.await.expect("continue");
        let last = b"data: two\n\n";
        socket
            .write_all(format!("{:x}\r\n", last.len()).as_bytes())
            .await
            .expect("chunk size");
        socket.write_all(last).await.expect("last chunk");
        socket.write_all(b"\r\n0\r\n\r\n").await.expect("end");
    });

    let identity = worker();
    let (worker_tx, mut worker_rx) = mpsc::channel::<WorkerLine>(16);
    let (host_tx, mut host_rx) = mpsc::channel::<WorkerLine>(16);
    let (stop_tx, mut stop_rx) = mpsc::channel::<()>(1);
    let client = BrokerClient::new(identity.clone(), Arc::new(worker_tx), "shim-token");
    let bridge_identity = identity.clone();
    let bridge_client = client.clone();
    let registry = BrokerRegistry::new();
    let targets = std::collections::HashMap::from([(
        "model:p".to_string(),
        BrokerTarget {
            id: "p".to_string(),
            kind: BrokerKind::Model,
            scope: vec![],
            base: Url::parse(&format!("http://{upstream_address}/v1")).expect("url"),
            headers: vec![("authorization".to_string(), "host-secret".to_string())],
            timeout_secs: 10,
        },
    )]);
    let bridge = tokio::spawn(async move {
        loop {
            tokio::select! {
                frame = worker_rx.recv() => {
                    let Some(frame) = frame else { break };
                    registry.handle(
                        frame,
                        &bridge_identity,
                        &targets,
                        host_tx.clone(),
                        2,
                        "shim-token",
                    )
                    .await
                    .expect("broker request");
                }
                frame = host_rx.recv() => {
                    let Some(frame) = frame else { break };
                    bridge_client.receive(frame).await.expect("broker reply");
                }
                _ = stop_rx.recv() => break,
            }
        }
    });

    let shim_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("shim listener");
    let shim_address = shim_listener.local_addr().expect("shim address");
    let shim = tokio::spawn(async move {
        let (socket, _) = shim_listener.accept().await.expect("accept");
        handle_http(socket, client).await.expect("shim response");
    });
    let mut http = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("http client")
        .post(format!(
            "http://{shim_address}/model/p/chat/completions?stream=true"
        ))
        .header("x-api-key", "worker-secret")
        .body("prompt")
        .send()
        .await
        .expect("http response");
    assert_eq!(http.status(), 200);
    assert!(http.headers().get("set-cookie").is_none());
    let first = http.chunk().await.expect("first response chunk");
    assert_eq!(first.as_deref(), Some(b"data: one\n\n".as_slice()));
    continue_tx.send(()).expect("continue");
    let mut body = first.expect("first chunk").as_ref().to_vec();
    while let Some(chunk) = http.chunk().await.expect("response chunk") {
        body.extend_from_slice(&chunk);
    }
    assert_eq!(body, b"data: one\n\ndata: two\n\n");
    shim.await.expect("shim");
    stop_tx.send(()).await.expect("stop bridge");
    bridge.await.expect("bridge");
    upstream.await.expect("upstream");
}

#[tokio::test]
async fn truncated_upstream_is_not_reported_as_clean_http_eof() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let upstream_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("upstream listener");
    let upstream_address = upstream_listener.local_addr().expect("upstream address");
    let upstream = tokio::spawn(async move {
        let (socket, _) = upstream_listener.accept().await.expect("accept");
        let mut reader = BufReader::new(socket);
        let mut line = String::new();
        while reader.read_line(&mut line).await.expect("read request") != 0 {
            if line == "\r\n" {
                break;
            }
            line.clear();
        }
        let mut socket = reader.into_inner();
        socket
            .write_all(b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n1\r\nx\r\n")
            .await
            .expect("truncated response");
    });
    let identity = worker();
    let (worker_tx, mut worker_rx) = mpsc::channel::<WorkerLine>(16);
    let (host_tx, mut host_rx) = mpsc::channel::<WorkerLine>(16);
    let (stop_tx, mut stop_rx) = mpsc::channel::<()>(1);
    let client = BrokerClient::new(identity.clone(), Arc::new(worker_tx), "shim-token");
    let bridge_identity = identity.clone();
    let bridge_client = client.clone();
    let registry = BrokerRegistry::new();
    let targets = std::collections::HashMap::from([(
        "model:p".to_string(),
        BrokerTarget {
            id: "p".to_string(),
            kind: BrokerKind::Model,
            scope: vec![],
            base: Url::parse(&format!("http://{upstream_address}/v1")).expect("url"),
            headers: vec![],
            timeout_secs: 5,
        },
    )]);
    let bridge = tokio::spawn(async move {
        loop {
            tokio::select! {
                frame = worker_rx.recv() => {
                    let Some(frame) = frame else { break };
                    registry.handle(frame, &bridge_identity, &targets, host_tx.clone(), 2, "shim-token")
                        .await.expect("broker request");
                }
                frame = host_rx.recv() => {
                    let Some(frame) = frame else { break };
                    bridge_client.receive(frame).await.expect("broker reply");
                }
                _ = stop_rx.recv() => break,
            }
        }
    });
    let shim_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("shim listener");
    let shim_address = shim_listener.local_addr().expect("shim address");
    let shim = tokio::spawn(async move {
        let (socket, _) = shim_listener.accept().await.expect("accept");
        handle_http(socket, client).await
    });
    let mut http = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("http client")
        .get(format!("http://{shim_address}/model/p"))
        .send()
        .await
        .expect("http response");
    assert_eq!(http.status(), 200);
    assert_eq!(
        http.chunk().await.expect("first chunk").as_deref(),
        Some(b"x".as_slice())
    );
    let error = http.chunk().await.expect_err("truncated body must fail");
    assert!(error.is_decode(), "{error:?}");
    assert!(shim.await.expect("shim task").is_err());
    stop_tx.send(()).await.expect("stop bridge");
    bridge.await.expect("bridge");
    upstream.await.expect("upstream");
}

#[tokio::test]
async fn http_disconnect_cancels_stalled_upstream() {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    let upstream_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("upstream listener");
    let upstream_address = upstream_listener.local_addr().expect("upstream address");
    let upstream = tokio::spawn(async move {
        let (socket, _) = upstream_listener.accept().await.expect("accept");
        let mut reader = BufReader::new(socket);
        let mut line = String::new();
        while reader.read_line(&mut line).await.expect("read request") != 0 {
            if line == "\r\n" {
                break;
            }
            line.clear();
        }
        let mut socket = reader.into_inner();
        socket
            .write_all(b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n1\r\nx\r\n")
            .await
            .expect("response");
        tokio::time::timeout(Duration::from_secs(5), async {
            let mut byte = [0u8; 1];
            assert_eq!(socket.read(&mut byte).await.expect("read"), 0);
        })
        .await
        .expect("upstream was not cancelled");
    });
    let identity = worker();
    let (worker_tx, mut worker_rx) = mpsc::channel::<WorkerLine>(16);
    let (host_tx, mut host_rx) = mpsc::channel::<WorkerLine>(16);
    let (stop_tx, mut stop_rx) = mpsc::channel::<()>(1);
    let client = BrokerClient::new(identity.clone(), Arc::new(worker_tx), "shim-token");
    let bridge_identity = identity.clone();
    let bridge_client = client.clone();
    let registry = BrokerRegistry::new();
    let targets = std::collections::HashMap::from([(
        "model:p".to_string(),
        BrokerTarget {
            id: "p".to_string(),
            kind: BrokerKind::Model,
            scope: vec![],
            base: Url::parse(&format!("http://{upstream_address}/v1")).expect("url"),
            headers: vec![],
            timeout_secs: 30,
        },
    )]);
    let bridge = tokio::spawn(async move {
        loop {
            tokio::select! {
                frame = worker_rx.recv() => {
                    let Some(frame) = frame else { break };
                    registry.handle(frame, &bridge_identity, &targets, host_tx.clone(), 2, "shim-token")
                        .await.expect("broker request");
                }
                frame = host_rx.recv() => {
                    let Some(frame) = frame else { break };
                    bridge_client.receive(frame).await.expect("broker reply");
                }
                _ = stop_rx.recv() => break,
            }
        }
    });
    let shim_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("shim listener");
    let shim_address = shim_listener.local_addr().expect("shim address");
    let shim = tokio::spawn(async move {
        let (socket, _) = shim_listener.accept().await.expect("accept");
        handle_http(socket, client).await
    });
    let mut http = reqwest::Client::builder()
        .no_proxy()
        .build()
        .expect("http client")
        .get(format!("http://{shim_address}/model/p"))
        .send()
        .await
        .expect("http response");
    assert_eq!(
        http.chunk().await.expect("first chunk").as_deref(),
        Some(b"x".as_slice())
    );
    drop(http);
    upstream.await.expect("upstream cancelled");
    assert!(shim.await.expect("shim task").is_err());
    stop_tx.send(()).await.expect("stop bridge");
    bridge.await.expect("bridge");
}
