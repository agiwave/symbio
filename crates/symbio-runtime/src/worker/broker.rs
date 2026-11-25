use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;
use reqwest::header::{HeaderName, HeaderValue};
use reqwest::redirect::Policy;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

use super::container::{BrokerTarget, MAX_BROKER_STREAMS};
use super::protocol::WorkerLine;
use super::WorkerId;

const CHUNK: usize = 48 * 1024;
const HOP_HEADERS: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailer",
    "transfer-encoding",
    "upgrade",
    "host",
    "content-length",
];

struct PumpRequest {
    method: String,
    url: reqwest::Url,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    outgoing: mpsc::Sender<WorkerLine>,
}

#[derive(Default)]
pub struct BrokerRegistry {
    streams: Arc<Mutex<HashMap<u64, JoinHandle<()>>>>,
}

impl BrokerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn handle(
        &self,
        line: WorkerLine,
        expected: &WorkerId,
        targets: &HashMap<String, BrokerTarget>,
        outgoing: mpsc::Sender<WorkerLine>,
        max_streams: usize,
        token: &str,
    ) -> Result<(), String> {
        if token.is_empty() {
            return Err("broker shim 令牌为空".to_string());
        }
        if max_streams == 0 || max_streams > MAX_BROKER_STREAMS {
            return Err("broker 并发流上限无效".to_string());
        }
        match line {
            WorkerLine::BrokerOpen {
                worker,
                stream,
                target,
                method,
                path,
                mut headers,
                body,
            } => {
                worker.validate(expected).map_err(|e| e.to_string())?;
                let authenticated = match headers
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
                {
                    Some((_, value)) => value.as_str() == format!("Bearer {token}"),
                    None => false,
                };
                if !authenticated {
                    return Err("broker shim 令牌无效".to_string());
                }
                headers.retain(|(name, _)| !name.eq_ignore_ascii_case("authorization"));
                let body = BASE64
                    .decode(body)
                    .map_err(|e| format!("broker 请求体编码无效: {e}"))?;
                if body.len() > 8 * 1024 * 1024 {
                    return Err("broker 请求体超过上限".to_string());
                }
                let target = targets.get(&target).ok_or("broker 目标未登记")?;
                if !target.accepts_path(&path) {
                    return Err("broker 路径不在目标白名单".to_string());
                }
                if !matches!(method.as_str(), "GET" | "POST") {
                    return Err("broker 方法不允许".to_string());
                }
                let url = target.url_for(&path)?;
                let mut streams = self.streams.lock().await;
                if streams.len() >= max_streams {
                    return Err("broker 并发流超过上限".to_string());
                }
                if streams.contains_key(&stream) {
                    return Err("broker stream id 重复".to_string());
                }
                let worker = worker.clone();
                let target = target.clone();
                let task_worker = worker.clone();
                let registry = Arc::clone(&self.streams);
                let task = tokio::spawn(async move {
                    let result = pump(
                        task_worker.clone(),
                        stream,
                        target,
                        PumpRequest {
                            method,
                            url,
                            headers,
                            body,
                            outgoing: outgoing.clone(),
                        },
                    )
                    .await;
                    registry.lock().await.remove(&stream);
                    let frame = match result {
                        Ok(()) => WorkerLine::BrokerEnd {
                            worker: task_worker,
                            stream,
                        },
                        Err(message) => WorkerLine::BrokerError {
                            worker: task_worker,
                            stream,
                            message,
                        },
                    };
                    if let Err(error) = outgoing.send(frame).await {
                        tracing::warn!(%error, stream, "broker 输出通道已关闭");
                    }
                });
                streams.insert(stream, task);
            }
            WorkerLine::BrokerCancel { worker, stream } => {
                worker.validate(expected).map_err(|e| e.to_string())?;
                if let Some(task) = self.streams.lock().await.remove(&stream) {
                    task.abort();
                }
            }
            WorkerLine::BrokerHead { .. }
            | WorkerLine::BrokerData { .. }
            | WorkerLine::BrokerEnd { .. }
            | WorkerLine::BrokerError { .. } => {
                return Err("worker 发出方向错误的 broker 帧".to_string());
            }
            _ => return Err("不是 broker 帧".to_string()),
        }
        Ok(())
    }

    pub async fn stop(&self) {
        let tasks: Vec<_> = self
            .streams
            .lock()
            .await
            .drain()
            .map(|(_, task)| task)
            .collect();
        for task in tasks {
            task.abort();
        }
    }
}

async fn pump(
    worker: WorkerId,
    stream: u64,
    target: BrokerTarget,
    request: PumpRequest,
) -> Result<(), String> {
    let PumpRequest {
        method,
        url,
        headers,
        body,
        outgoing,
    } = request;
    let method = reqwest::Method::from_bytes(method.as_bytes()).map_err(|e| e.to_string())?;
    let mut request_headers = reqwest::header::HeaderMap::new();
    for (name, value) in headers {
        let lower = name.to_ascii_lowercase();
        if HOP_HEADERS.contains(&lower.as_str()) || is_credential_header(&lower) {
            continue;
        }
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(&value),
        ) {
            request_headers.insert(name, value);
        }
    }
    for (name, value) in target.headers {
        if let (Ok(name), Ok(value)) = (
            HeaderName::from_bytes(name.as_bytes()),
            HeaderValue::from_str(&value),
        ) {
            request_headers.insert(name, value);
        }
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(Policy::custom(|attempt| {
            if attempt
                .previous()
                .first()
                .is_some_and(|previous| previous.origin() == attempt.url().origin())
            {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(180))
        .timeout(Duration::from_secs(target.timeout_secs.max(1)))
        .build()
        .map_err(|e| e.to_string())?;
    let mut response = client
        .request(method, url)
        .headers(request_headers)
        .body(body)
        .send()
        .await
        .map_err(|_| "broker 上游连接失败".to_string())?;
    let status = response.status().as_u16();
    let mut response_headers = Vec::new();
    for (name, value) in response.headers() {
        let lower = name.as_str().to_ascii_lowercase();
        if HOP_HEADERS.contains(&lower.as_str())
            || is_credential_header(&lower)
            || matches!(lower.as_str(), "location" | "content-location")
        {
            continue;
        }
        if let Ok(value) = value.to_str() {
            response_headers.push((lower, value.to_string()));
        }
    }
    outgoing
        .send(WorkerLine::BrokerHead {
            worker: worker.clone(),
            stream,
            status,
            headers: response_headers,
        })
        .await
        .map_err(|_| "broker 输出通道已关闭".to_string())?;
    let mut offset = 0u64;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "broker 上游读取失败".to_string())?
    {
        for chunk in chunk.chunks(CHUNK) {
            let length = chunk.len();
            outgoing
                .send(WorkerLine::BrokerData {
                    worker: worker.clone(),
                    stream,
                    offset,
                    data: BASE64.encode(chunk),
                })
                .await
                .map_err(|_| "broker 输出通道已关闭".to_string())?;
            offset = offset.checked_add(length as u64).ok_or("broker 响应过长")?;
        }
    }
    Ok(())
}

fn is_credential_header(name: &str) -> bool {
    matches!(
        name,
        "authorization"
            | "proxy-authorization"
            | "cookie"
            | "set-cookie"
            | "x-api-key"
            | "x-goog-api-key"
            | "api-key"
    )
}

#[cfg(test)]
#[path = "broker.test.rs"]
mod tests;
