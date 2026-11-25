use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_util::sync::CancellationToken;

use super::container::{BROKER_PORT, MAX_BROKER_STREAMS};
use super::protocol::WorkerLine;
use super::WorkerId;

const MAX_BODY: usize = 8 * 1024 * 1024;
const MAX_HEAD: usize = 64 * 1024;
const HEADER_TIMEOUT: Duration = Duration::from_secs(5);
const BODY_TIMEOUT: Duration = Duration::from_secs(30);
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
const DATA_SEND_TIMEOUT: Duration = Duration::from_secs(10);
type HeadResult = Result<(u16, Vec<(String, String)>), String>;

struct PendingStream {
    head: Option<oneshot::Sender<HeadResult>>,
    data: mpsc::Sender<Vec<u8>>,
    terminal: Option<oneshot::Sender<Option<String>>>,
    expected_offset: u64,
}

#[derive(Clone)]
pub struct BrokerClient {
    worker: WorkerId,
    outgoing: Arc<mpsc::Sender<WorkerLine>>,
    token: String,
    pending: Arc<Mutex<HashMap<u64, PendingStream>>>,
    next: Arc<AtomicU64>,
}

impl BrokerClient {
    pub fn new(
        worker: WorkerId,
        outgoing: Arc<mpsc::Sender<WorkerLine>>,
        token: impl Into<String>,
    ) -> Self {
        Self {
            worker,
            outgoing,
            token: token.into(),
            pending: Arc::new(Mutex::new(HashMap::new())),
            next: Arc::new(AtomicU64::new(1)),
        }
    }

    pub async fn receive(&self, line: WorkerLine) -> Result<(), String> {
        match line {
            WorkerLine::BrokerHead {
                worker,
                stream,
                status,
                headers,
            } => {
                worker.validate(&self.worker).map_err(|e| e.to_string())?;
                let mut pending = self.pending.lock().await;
                let entry = pending.get_mut(&stream).ok_or("未知 broker stream")?;
                let _ = entry
                    .head
                    .take()
                    .ok_or("broker 响应头已接收")?
                    .send(Ok((status, headers)));
            }
            WorkerLine::BrokerData {
                worker,
                stream,
                offset,
                data,
            } => {
                worker.validate(&self.worker).map_err(|e| e.to_string())?;
                let Ok(bytes) = BASE64.decode(data) else {
                    self.fail_stream(stream, "broker 响应分片无效".to_string())
                        .await;
                    return Ok(());
                };
                let length = bytes.len() as u64;
                let data_tx = {
                    let mut pending = self.pending.lock().await;
                    let Some(entry) = pending.get_mut(&stream) else {
                        return Ok(());
                    };
                    if entry.expected_offset != offset {
                        drop(pending);
                        self.fail_stream(stream, "broker 响应分片序号不连续".to_string())
                            .await;
                        return Ok(());
                    }
                    entry.expected_offset += length;
                    entry.data.clone()
                };
                let delivered = tokio::time::timeout(DATA_SEND_TIMEOUT, data_tx.send(bytes)).await;
                if !matches!(delivered, Ok(Ok(()))) {
                    self.fail_stream(stream, "broker HTTP 接收端积压".to_string())
                        .await;
                }
            }
            WorkerLine::BrokerEnd { worker, stream } => {
                worker.validate(&self.worker).map_err(|e| e.to_string())?;
                let mut pending = self.pending.lock().await;
                if let Some(entry) = pending.get_mut(&stream) {
                    if let Some(terminal) = entry.terminal.take() {
                        let _ = terminal.send(None);
                    }
                }
                pending.remove(&stream);
            }
            WorkerLine::BrokerError {
                worker,
                stream,
                message,
            } => {
                worker.validate(&self.worker).map_err(|e| e.to_string())?;
                self.fail_stream(stream, message).await;
            }
            _ => return Err("不是 worker broker 回复".to_string()),
        }
        Ok(())
    }

    async fn fail_stream(&self, stream: u64, message: String) {
        let mut pending = self.pending.lock().await;
        if let Some(entry) = pending.get_mut(&stream) {
            if let Some(head) = entry.head.take() {
                let _ = head.send(Err(message));
            } else if let Some(terminal) = entry.terminal.take() {
                let _ = terminal.send(Some(message));
            }
        }
        pending.remove(&stream);
    }

    pub async fn open(
        &self,
        target: String,
        method: String,
        path: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    ) -> Result<BrokerStream, String> {
        if body.len() > MAX_BODY {
            return Err("请求体超过 broker 上限".to_string());
        }
        let stream = self.next.fetch_add(1, Ordering::Relaxed);
        let (head_tx, head_rx) = oneshot::channel();
        let (terminal_tx, terminal_rx) = oneshot::channel();
        let (data_tx, data_rx) = mpsc::channel(16);
        self.pending.lock().await.insert(
            stream,
            PendingStream {
                head: Some(head_tx),
                data: data_tx,
                terminal: Some(terminal_tx),
                expected_offset: 0,
            },
        );
        let mut headers = headers;
        headers.retain(|(name, _)| !name.eq_ignore_ascii_case("authorization"));
        headers.push((
            "authorization".to_string(),
            format!("Bearer {}", self.token),
        ));
        if let Err(error) = self
            .outgoing
            .send(WorkerLine::BrokerOpen {
                worker: self.worker.clone(),
                stream,
                target,
                method,
                path,
                headers,
                body: BASE64.encode(body),
            })
            .await
        {
            self.pending.lock().await.remove(&stream);
            return Err(format!("broker 控制通道已关闭: {error}"));
        }
        Ok(BrokerStream {
            client: self.clone(),
            stream,
            head_rx: Some(head_rx),
            terminal_rx,
            data_rx,
        })
    }

    pub async fn cancel_all(&self) {
        let streams: Vec<_> = self
            .pending
            .lock()
            .await
            .drain()
            .map(|(id, _)| id)
            .collect();
        for stream in streams {
            let _ = self
                .outgoing
                .send(WorkerLine::BrokerCancel {
                    worker: self.worker.clone(),
                    stream,
                })
                .await;
        }
    }
}

pub struct BrokerStream {
    client: BrokerClient,
    stream: u64,
    head_rx: Option<oneshot::Receiver<HeadResult>>,
    terminal_rx: oneshot::Receiver<Option<String>>,
    data_rx: mpsc::Receiver<Vec<u8>>,
}

impl BrokerStream {
    pub async fn head(&mut self) -> HeadResult {
        let receiver = self.head_rx.take().ok_or("broker 响应头已接收")?;
        receiver
            .await
            .map_err(|_| "broker 回复通道已关闭".to_string())?
    }

    pub async fn data(&mut self) -> Result<Option<Vec<u8>>, String> {
        if let Some(data) = self.data_rx.recv().await {
            return Ok(Some(data));
        }
        match self.terminal_rx.try_recv() {
            Ok(Some(_)) => Err("broker 上游流中断".to_string()),
            Ok(None) | Err(_) => Ok(None),
        }
    }
}

impl Drop for BrokerStream {
    fn drop(&mut self) {
        let client = self.client.clone();
        let stream = self.stream;
        let worker = client.worker.clone();
        let outgoing = client.outgoing.clone();
        tokio::spawn(async move {
            client.pending.lock().await.remove(&stream);
            let _ = outgoing
                .send(WorkerLine::BrokerCancel { worker, stream })
                .await;
        });
    }
}

pub async fn bind() -> Result<TcpListener, String> {
    TcpListener::bind(("127.0.0.1", BROKER_PORT))
        .await
        .map_err(|e| format!("broker 监听失败: {e}"))
}

pub async fn serve(
    client: BrokerClient,
    listener: TcpListener,
    streams: usize,
    stop: CancellationToken,
) -> Result<(), String> {
    if streams == 0 || streams > MAX_BROKER_STREAMS {
        return Err("broker 流上限无效".to_string());
    }
    let semaphore = Arc::new(tokio::sync::Semaphore::new(streams));
    loop {
        let permit = tokio::select! {
            _ = stop.cancelled() => return Ok(()),
            permit = semaphore.clone().acquire_owned() => match permit { Ok(p) => p, Err(_) => return Ok(()) },
        };
        let (socket, _) = tokio::select! {
            _ = stop.cancelled() => return Ok(()),
            result = listener.accept() => result.map_err(|e| e.to_string())?,
        };
        let client = client.clone();
        tokio::spawn(async move {
            let _permit = permit;
            if let Err(error) = handle_http(socket, client).await {
                tracing::warn!(%error, "broker shim 请求失败");
            }
        });
    }
}

async fn handle_http(socket: TcpStream, client: BrokerClient) -> Result<(), String> {
    let (read, mut write) = socket.into_split();
    let mut reader = BufReader::new(read);
    let head = tokio::time::timeout(HEADER_TIMEOUT, async {
        let mut head = Vec::new();
        loop {
            let byte = reader.read_u8().await.map_err(|e| e.to_string())?;
            head.push(byte);
            if head.len() > MAX_HEAD {
                return Err("broker HTTP 头过大".to_string());
            }
            if head.ends_with(b"\r\n\r\n") {
                return Ok::<_, String>(head);
            }
        }
    })
    .await
    .map_err(|_| "broker HTTP 头读取超时".to_string())??;
    let head_text = String::from_utf8(head).map_err(|_| "broker HTTP 头无效".to_string())?;
    let mut lines = head_text.split("\r\n");
    let request = lines.next().ok_or("缺少请求行")?;
    let mut parts = request.split_whitespace();
    let method = parts.next().ok_or("缺少方法")?.to_string();
    let path = parts.next().ok_or("缺少路径")?.to_string();
    let mut headers = Vec::new();
    let mut content_length = 0usize;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("content-length") {
            content_length = value
                .trim()
                .parse()
                .map_err(|_| "Content-Length 无效".to_string())?;
        } else {
            headers.push((name.to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    if content_length > MAX_BODY {
        return Err("请求体过大".to_string());
    }
    let mut body = vec![0u8; content_length];
    tokio::time::timeout(BODY_TIMEOUT, reader.read_exact(&mut body))
        .await
        .map_err(|_| "broker 请求体读取超时".to_string())?
        .map_err(|e| e.to_string())?;
    let (target, path) = broker_target(&path)?;
    let mut stream = client.open(target, method, path, headers, body).await?;
    let mut client_read = reader.into_inner();
    let (status, response_headers) = match stream.head().await {
        Ok(head) => head,
        Err(_) => {
            let body =
                serde_json::to_vec(&serde_json::json!({"error": "broker upstream unavailable"}))
                    .map_err(|e| e.to_string())?;
            write_response_head(
                &mut write,
                502,
                "application/json",
                vec![],
                Some(body.len()),
            )
            .await?;
            write_http(&mut write, &body).await?;
            return Ok(());
        }
    };
    let content_type = response_headers
        .iter()
        .find(|(k, _)| k == "content-type")
        .map(|(_, v)| v.clone())
        .unwrap_or_else(|| "application/octet-stream".to_string());
    write_response_head(&mut write, status, &content_type, response_headers, None).await?;
    loop {
        let chunk = tokio::select! {
            chunk = stream.data() => chunk?,
            byte = client_read.read_u8() => {
                return match byte {
                    Ok(0) => Err("broker HTTP 客户端已断开".to_string()),
                    Ok(_) => Err("broker HTTP 不支持流水线请求".to_string()),
                    Err(error) => Err(format!("broker HTTP 客户端读取失败: {error}")),
                };
            }
        };
        let Some(chunk) = chunk else { break };
        write_http(&mut write, format!("{:x}\r\n", chunk.len()).as_bytes()).await?;
        write_http(&mut write, &chunk).await?;
        write_http(&mut write, b"\r\n").await?;
    }
    write_http(&mut write, b"0\r\n\r\n").await
}

async fn write_http(write: &mut (impl AsyncWriteExt + Unpin), bytes: &[u8]) -> Result<(), String> {
    tokio::time::timeout(WRITE_TIMEOUT, async {
        write.write_all(bytes).await.map_err(|e| e.to_string())?;
        write.flush().await.map_err(|e| e.to_string())
    })
    .await
    .map_err(|_| "broker HTTP 响应写入超时".to_string())?
}

fn broker_target(path: &str) -> Result<(String, String), String> {
    let route = path.split('?').next().unwrap_or(path);
    let mut segments = route.trim_start_matches('/').split('/').peekable();
    let mut target = String::new();
    if segments.peek() == Some(&"subagent") {
        segments.next();
        loop {
            let Some(segment) = segments.peek().copied() else {
                return Err("broker 子 Agent 缺少目标".to_string());
            };
            if matches!(segment, "model" | "mcp") {
                break;
            }
            let id = decode_resource_id(segment)?;
            target.push_str("subagent:");
            target.push_str(&id);
            target.push('/');
            segments.next();
        }
    }
    let kind = match segments.next() {
        Some("model") => "model",
        Some("mcp") => "mcp",
        _ => return Err("broker 目标无效".to_string()),
    };
    let id = decode_resource_id(segments.next().ok_or("broker 缺少 id")?)?;
    target.push_str(kind);
    target.push(':');
    target.push_str(&id);
    Ok((target, path.to_string()))
}

fn decode_resource_id(segment: &str) -> Result<String, String> {
    let id = urlencoding::decode(segment)
        .map_err(|e| e.to_string())?
        .into_owned();
    if id.is_empty()
        || matches!(id.as_str(), "." | "..")
        || id.contains(['/', '\\'])
        || id.chars().any(char::is_control)
    {
        return Err("broker id 无效".to_string());
    }
    Ok(id)
}

async fn write_response_head(
    write: &mut (impl AsyncWriteExt + Unpin),
    status: u16,
    content_type: &str,
    headers: Vec<(String, String)>,
    fixed_length: Option<usize>,
) -> Result<(), String> {
    let mut head =
        format!("HTTP/1.1 {status} OK\r\ncontent-type: {content_type}\r\nconnection: close\r\n");
    if let Some(length) = fixed_length {
        head.push_str(&format!("content-length: {length}\r\n"));
    } else {
        head.push_str("transfer-encoding: chunked\r\n");
    }
    for (name, value) in headers {
        if matches!(
            name.as_str(),
            "content-type" | "transfer-encoding" | "connection" | "content-length"
        ) {
            continue;
        }
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    write_http(write, head.as_bytes()).await
}

#[cfg(test)]
#[path = "broker_shim.test.rs"]
mod tests;
