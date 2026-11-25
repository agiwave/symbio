use super::*;
use crate::worker::{BuildId, Epoch, WorkerId};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::mpsc;
use url::Url;

fn token() -> &'static str {
    "shim-token"
}

fn auth() -> Vec<(String, String)> {
    vec![("authorization".to_string(), format!("Bearer {}", token()))]
}

fn worker(epoch: u64) -> WorkerId {
    WorkerId::new(
        "session-test",
        Epoch::new(epoch).expect("epoch"),
        BuildId::parse("a".repeat(64)).expect("build"),
    )
    .expect("worker")
}

fn target() -> BrokerTarget {
    BrokerTarget {
        id: "p".to_string(),
        kind: super::super::container::BrokerKind::Model,
        scope: vec![],
        base: Url::parse("http://127.0.0.1:9/v1").expect("url"),
        headers: vec![("authorization".to_string(), "host-secret".to_string())],
        timeout_secs: 1,
    }
}

#[tokio::test]
async fn stale_epoch_and_unknown_target_are_rejected() {
    let registry = BrokerRegistry::new();
    let expected = worker(2);
    let targets = HashMap::from([("model:p".to_string(), target())]);
    let (tx, _rx) = mpsc::channel(4);
    let stale = WorkerLine::BrokerOpen {
        worker: worker(1),
        stream: 1,
        target: "model:p".to_string(),
        method: "POST".to_string(),
        path: "/model/p/chat/completions".to_string(),
        headers: auth(),
        body: String::new(),
    };
    assert!(registry
        .handle(stale, &expected, &targets, tx.clone(), 4, token())
        .await
        .is_err());
    let unknown = WorkerLine::BrokerOpen {
        worker: expected.clone(),
        stream: 1,
        target: "model:other".to_string(),
        method: "POST".to_string(),
        path: "/model/other/chat/completions".to_string(),
        headers: auth(),
        body: String::new(),
    };
    assert!(registry
        .handle(unknown, &expected, &targets, tx, 4, token())
        .await
        .is_err());
}

#[tokio::test]
async fn cancellation_aborts_registered_stream() {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listen");
    let address = listener.local_addr().expect("address");
    let (sent_tx, sent_rx) = tokio::sync::oneshot::channel();
    let upstream = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.expect("accept");
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
        assert_eq!(body, b"request");
        let mut socket = reader.into_inner();
        socket
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 10\r\n\r\nx")
            .await
            .expect("write");
        sent_tx.send(()).expect("sent");
        let mut buffer = [0u8; 1];
        let length = socket.read(&mut buffer).await.expect("read");
        assert_eq!(length, 0, "cancelled upstream must close the connection");
    });
    let registry = BrokerRegistry::new();
    let expected = worker(1);
    let mut target = target();
    target.base = Url::parse(&format!("http://{address}/v1")).expect("url");
    target.timeout_secs = 10;
    let targets = HashMap::from([("model:p".to_string(), target)]);
    let (tx, mut rx) = mpsc::channel(8);
    let open = WorkerLine::BrokerOpen {
        worker: expected.clone(),
        stream: 7,
        target: "model:p".to_string(),
        method: "POST".to_string(),
        path: "/model/p/chat/completions".to_string(),
        headers: auth(),
        body: BASE64.encode(b"request"),
    };
    registry
        .handle(open, &expected, &targets, tx.clone(), 4, token())
        .await
        .expect("open");
    tokio::time::timeout(Duration::from_secs(3), async {
        sent_rx.await.expect("upstream response");
        assert!(matches!(
            rx.recv().await,
            Some(WorkerLine::BrokerHead { status: 200, .. })
        ));
        assert!(matches!(
            rx.recv().await,
            Some(WorkerLine::BrokerData { .. })
        ));
        registry
            .handle(
                WorkerLine::BrokerCancel {
                    worker: expected,
                    stream: 7,
                },
                &worker(1),
                &targets,
                tx,
                4,
                token(),
            )
            .await
            .expect("cancel");
        upstream.await.expect("upstream closed");
    })
    .await
    .expect("cancellation timed out");
}

#[tokio::test]
async fn broker_injects_only_trusted_credentials_and_releases_completed_streams() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("listen");
    let address = listener.local_addr().expect("address");
    let upstream = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.expect("accept");
        let mut request = Vec::new();
        let mut buffer = [0u8; 1024];
        loop {
            let length = socket.read(&mut buffer).await.expect("read");
            if length == 0 {
                break;
            }
            request.extend_from_slice(&buffer[..length]);
            if request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                break;
            }
        }
        let text = String::from_utf8(request).expect("request");
        let lower = text.to_ascii_lowercase();
        assert!(lower.contains("authorization: host-secret\r\n"), "{text}");
        assert!(!lower.contains("worker-secret"));
        assert!(lower.contains("post /v1/chat/completions?stream=true http/1.1"));
        let body = vec![b'x'; CHUNK + 17];
        socket.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nset-cookie: secret\r\nlocation: https://upstream.example/signed?credential=host-secret\r\ncontent-length: ").await.expect("write");
        socket
            .write_all(body.len().to_string().as_bytes())
            .await
            .expect("write");
        socket.write_all(b"\r\n\r\n").await.expect("write");
        socket.write_all(&body).await.expect("write");
    });
    let registry = BrokerRegistry::new();
    let expected = worker(1);
    let mut target = target();
    target.base = Url::parse(&format!("http://{address}/v1")).expect("url");
    let targets = HashMap::from([("model:p".to_string(), target)]);
    let (tx, mut rx) = mpsc::channel(8);
    let open = WorkerLine::BrokerOpen {
        worker: expected.clone(),
        stream: 1,
        target: "model:p".to_string(),
        method: "POST".to_string(),
        path: "/model/p/chat/completions?stream=true".to_string(),
        headers: vec![
            ("authorization".to_string(), format!("Bearer {}", token())),
            ("x-api-key".to_string(), "worker-secret".to_string()),
        ],
        body: String::new(),
    };
    registry
        .handle(open, &expected, &targets, tx.clone(), 1, token())
        .await
        .expect("open");
    let mut data_length = 0;
    let mut headers = Vec::new();
    loop {
        match rx.recv().await.expect("frame") {
            WorkerLine::BrokerHead {
                headers: values, ..
            } => headers = values,
            WorkerLine::BrokerData { data, .. } => {
                data_length += BASE64.decode(data).expect("data").len()
            }
            WorkerLine::BrokerEnd { .. } => break,
            WorkerLine::BrokerError { message, .. } => panic!("{message}"),
            _ => panic!("unexpected frame"),
        }
    }
    upstream.await.expect("upstream");
    assert_eq!(data_length, CHUNK + 17);
    assert!(!headers.iter().any(|(name, _)| name == "set-cookie"));
    assert!(!headers.iter().any(|(name, _)| name == "location"));
    let next = WorkerLine::BrokerOpen {
        worker: expected.clone(),
        stream: 2,
        target: "model:p".to_string(),
        method: "GET".to_string(),
        path: "/model/p".to_string(),
        headers: auth(),
        body: String::new(),
    };
    registry
        .handle(next, &expected, &targets, tx, 1, token())
        .await
        .expect("completed stream released");
}

#[tokio::test]
async fn broker_rejects_missing_shim_token() {
    let registry = BrokerRegistry::new();
    let expected = worker(1);
    let targets = HashMap::from([("model:p".to_string(), target())]);
    let (tx, _rx) = mpsc::channel(1);
    let open = WorkerLine::BrokerOpen {
        worker: expected.clone(),
        stream: 1,
        target: "model:p".to_string(),
        method: "GET".to_string(),
        path: "/model/p".to_string(),
        headers: vec![],
        body: String::new(),
    };
    assert!(registry
        .handle(open, &expected, &targets, tx, 1, token())
        .await
        .is_err());
}

#[tokio::test]
async fn broker_rejects_malformed_body_and_empty_token() {
    let registry = BrokerRegistry::new();
    let expected = worker(1);
    let targets = HashMap::from([("model:p".to_string(), target())]);
    let (tx, _rx) = mpsc::channel(1);
    let open = WorkerLine::BrokerOpen {
        worker: expected.clone(),
        stream: 1,
        target: "model:p".to_string(),
        method: "POST".to_string(),
        path: "/model/p".to_string(),
        headers: auth(),
        body: "not-base64!".to_string(),
    };
    assert!(registry
        .handle(open, &expected, &targets, tx.clone(), 1, token())
        .await
        .is_err());
    let open = WorkerLine::BrokerOpen {
        worker: expected.clone(),
        stream: 2,
        target: "model:p".to_string(),
        method: "GET".to_string(),
        path: "/model/p".to_string(),
        headers: vec![("authorization".to_string(), "Bearer ".to_string())],
        body: String::new(),
    };
    assert!(registry
        .handle(open, &expected, &targets, tx, 1, "")
        .await
        .is_err());
}

#[tokio::test]
async fn broker_errors_never_disclose_upstream_url() {
    let registry = BrokerRegistry::new();
    let expected = worker(1);
    let mut target = target();
    target.base = Url::parse("http://127.0.0.1:9/v1?credential=host-secret").expect("url");
    let targets = HashMap::from([("model:p".to_string(), target)]);
    let (tx, mut rx) = mpsc::channel(1);
    let open = WorkerLine::BrokerOpen {
        worker: expected.clone(),
        stream: 1,
        target: "model:p".to_string(),
        method: "GET".to_string(),
        path: "/model/p".to_string(),
        headers: auth(),
        body: String::new(),
    };
    registry
        .handle(open, &expected, &targets, tx, 1, token())
        .await
        .expect("open");
    let message = tokio::time::timeout(Duration::from_secs(5), async {
        match rx.recv().await.expect("frame") {
            WorkerLine::BrokerError { message, .. } => message,
            _ => panic!("unexpected frame"),
        }
    })
    .await
    .expect("broker error timed out");
    assert_eq!(message, "broker 上游连接失败");
    assert!(!message.contains("host-secret"));
    assert!(!message.contains("127.0.0.1"));
}

#[test]
fn credential_and_hop_by_hop_headers_are_never_forwarded_from_worker() {
    for name in [
        "authorization",
        "cookie",
        "x-api-key",
        "x-goog-api-key",
        "host",
        "connection",
    ] {
        assert!(is_credential_header(name) || HOP_HEADERS.contains(&name));
    }
}
