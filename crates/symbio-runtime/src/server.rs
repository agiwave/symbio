use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};
use symbio_http::{
    payload_response, CancellationToken, HttpRequest, HttpResponse, Ingress, IngressReply,
    ServerConfig as HttpServerConfig, ServerHandle,
};
use symbio_platform::kernel::plugin::{PluginMessageWire, PluginPayload};
use symbio_plugin_api::routes::ROUTE_HOME_RELOAD;
use tokio::sync::{Notify, RwLock};

use crate::directories::list_directories;
use crate::RuntimeHost;

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind: String,
    pub port: u16,
    pub token: String,
    pub max_connections: usize,
    pub frontend_dist: PathBuf,
    pub allowed_origin: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".to_string(),
            port: 9231,
            token: String::new(),
            max_connections: 64,
            frontend_dist: PathBuf::from("tauri/dist"),
            allowed_origin: None,
        }
    }
}

pub struct ServerIngress {
    host: Arc<RuntimeHost>,
    frontend_dist: PathBuf,
    runtime_gate: Arc<RwLock<()>>,
    realtime: Arc<RealtimeConnections>,
}

impl ServerIngress {
    pub fn new(host: Arc<RuntimeHost>, frontend_dist: PathBuf) -> Self {
        Self {
            host,
            frontend_dist,
            runtime_gate: Arc::new(RwLock::new(())),
            realtime: Arc::new(RealtimeConnections::default()),
        }
    }
}

pub async fn start(config: ServerConfig, host: Arc<RuntimeHost>) -> Result<ServerHandle, String> {
    let frontend_dist = std::fs::canonicalize(&config.frontend_dist)
        .map_err(|e| format!("前端静态目录不可用 {}: {e}", config.frontend_dist.display()))?;
    if !frontend_dist.is_dir() {
        return Err(format!("前端静态路径不是目录: {}", frontend_dist.display()));
    }
    let ingress = Arc::new(ServerIngress::new(host, frontend_dist));
    symbio_http::start(
        HttpServerConfig {
            bind: config.bind,
            port: config.port,
            token: config.token,
            max_connections: config.max_connections,
            cors: false,
            allowed_origin: config.allowed_origin,
        },
        ingress,
    )
    .await
}

#[async_trait]
impl Ingress for ServerIngress {
    fn is_public(&self, request: &HttpRequest) -> bool {
        let path = request_path(&request.path);
        path == "/api/v1/health" || !path.starts_with("/api/")
    }

    async fn handle(&self, request: HttpRequest, cancel: CancellationToken) -> IngressReply {
        let path = request_path(&request.path);
        match (request.method.as_str(), path) {
            ("GET", "/api/v1/health") => IngressReply::Response(HttpResponse::json(
                200,
                json!({
                    "ok": true,
                    "deployment": "server",
                    "version": env!("CARGO_PKG_VERSION"),
                }),
            )),
            ("GET", "/api/v1/host/runtime") => {
                let _guard = self.runtime_gate.read().await;
                match self.host.homedir().await {
                    Ok(mut info) => {
                        if let Some(object) = info.as_object_mut() {
                            object.insert(
                                "deployment".to_string(),
                                Value::String("server".to_string()),
                            );
                            object.insert(
                                "capabilities".to_string(),
                                json!({ "directory_browser": true }),
                            );
                        }
                        IngressReply::Response(HttpResponse::json(200, &info))
                    }
                    Err(error) => {
                        IngressReply::Response(HttpResponse::json(500, json!({ "error": error })))
                    }
                }
            }
            ("GET", "/api/v1/host/directories") => {
                let _guard = self.runtime_gate.read().await;
                let requested = query_param(&request.path, "path");
                let listing =
                    tokio::task::spawn_blocking(move || list_directories(requested.as_deref()))
                        .await
                        .map_err(|e| format!("目录浏览任务失败: {e}"))
                        .and_then(|result| result);
                match listing {
                    Ok(listing) => IngressReply::Response(HttpResponse::json(200, &listing)),
                    Err(error) => {
                        IngressReply::Response(HttpResponse::json(400, json!({ "error": error })))
                    }
                }
            }
            ("POST", "/api/v1/invoke") => {
                if !request.is_json() {
                    return error_reply(415, "Content-Type must be application/json");
                }
                let message = match serde_json::from_slice::<PluginMessageWire>(&request.body) {
                    Ok(message) => message,
                    Err(error) => {
                        return error_reply(400, &format!("invalid PluginMessageWire: {error}"));
                    }
                };
                match self.invoke(message, cancel).await {
                    Ok(payload) => match payload_response(payload).await {
                        Ok(response) => IngressReply::Response(response),
                        Err(error) => error_reply(500, &error),
                    },
                    Err(error) => error_reply(400, &error),
                }
            }
            ("GET", "/api/v1/events") => {
                let _guard = self.runtime_gate.read().await;
                match self.subscribe().await {
                    Ok(channel) => {
                        self.realtime.enter(cancel);
                        IngressReply::Events(channel)
                    }
                    Err(error) => error_reply(500, &error),
                }
            }
            ("GET", "/api/v1/ws") => {
                let upgraded = request
                    .headers
                    .get("upgrade")
                    .is_some_and(|value| value.eq_ignore_ascii_case("websocket"));
                if !upgraded {
                    return error_reply(400, "WebSocket upgrade required");
                }
                let _guard = self.runtime_gate.read().await;
                self.realtime.enter(cancel);
                IngressReply::Upgrade
            }
            ("GET", _) => IngressReply::Response(self.static_response(path).await),
            _ => IngressReply::Response(HttpResponse::json(404, json!({ "error": "not found" }))),
        }
    }

    async fn invoke(
        &self,
        message: PluginMessageWire,
        cancel: CancellationToken,
    ) -> Result<PluginPayload, String> {
        let reload =
            message.metadata.get("path").and_then(Value::as_str) == Some(ROUTE_HOME_RELOAD);
        if reload {
            let _guard = tokio::select! {
                _ = cancel.cancelled() => return Err("request cancelled".to_string()),
                guard = async { self.runtime_gate.write().await } => guard,
            };
            self.realtime.cancel_and_wait(Some(&cancel)).await;
            // 重载会改写全局插件树；取得写锁后必须完成，不能因调用方断开而留下半次切换。
            return self.host.invoke(message).await;
        }
        let _guard = tokio::select! {
            _ = cancel.cancelled() => return Err("request cancelled".to_string()),
            guard = async { self.runtime_gate.read().await } => guard,
        };
        tokio::select! {
            biased;
            _ = cancel.cancelled() => Err("request cancelled".to_string()),
            result = self.host.invoke(message) => result,
        }
    }

    async fn realtime_closed(&self, cancel: CancellationToken) {
        self.realtime.close(cancel);
    }
}

#[derive(Default)]
struct RealtimeConnections {
    tokens: Mutex<Vec<CancellationToken>>,
    active: AtomicUsize,
    closed: Notify,
}

impl RealtimeConnections {
    fn enter(&self, cancel: CancellationToken) {
        self.tokens
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .push(cancel.clone());
        self.active.fetch_add(1, Ordering::AcqRel);
    }

    fn close(&self, cancel: CancellationToken) {
        let mut tokens = self
            .tokens
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let before = tokens.len();
        tokens.retain(|token| token != &cancel);
        let removed = tokens.len() != before;
        drop(tokens);
        if removed {
            self.active.fetch_sub(1, Ordering::AcqRel);
            self.closed.notify_waiters();
        }
    }

    async fn cancel_and_wait(&self, except: Option<&CancellationToken>) {
        let tokens = self
            .tokens
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let remaining = tokens.iter().filter(|token| except == Some(*token)).count();
        for token in tokens {
            if except.is_none_or(|except| &token != except) {
                token.cancel();
            }
        }
        loop {
            let closed = self.closed.notified();
            tokio::pin!(closed);
            closed.as_mut().enable();
            if self.active.load(Ordering::Acquire) <= remaining {
                return;
            }
            closed.await;
        }
    }
}

impl ServerIngress {
    async fn subscribe(&self) -> Result<symbio_platform::kernel::plugin::PluginChannel, String> {
        self.host.subscribe().await
    }

    async fn static_response(&self, request_path: &str) -> HttpResponse {
        let decoded = match urlencoding::decode(request_path) {
            Ok(path) => path.into_owned(),
            Err(_) => return HttpResponse::json(400, json!({ "error": "invalid path encoding" })),
        };
        let relative = Path::new(decoded.trim_start_matches('/'));
        if relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            return HttpResponse::json(403, json!({ "error": "forbidden" }));
        }
        let mut candidate = self.frontend_dist.join(relative);
        if candidate.is_dir() || !candidate.exists() {
            candidate = self.frontend_dist.join("index.html");
        }
        let canonical = match tokio::fs::canonicalize(&candidate).await {
            Ok(path) if path.starts_with(&self.frontend_dist) => path,
            Ok(_) => return HttpResponse::json(403, json!({ "error": "forbidden" })),
            Err(_) => {
                return HttpResponse::json(404, json!({ "error": "frontend asset not found" }))
            }
        };
        match tokio::fs::read(&canonical).await {
            Ok(body) => {
                let immutable = canonical
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension != "html");
                HttpResponse {
                    status: 200,
                    content_type: content_type(&canonical).to_string(),
                    body,
                    headers: vec![(
                        "Cache-Control".to_string(),
                        if immutable {
                            "public, max-age=31536000, immutable"
                        } else {
                            "no-cache"
                        }
                        .to_string(),
                    )],
                }
            }
            Err(error) => HttpResponse::json(
                500,
                json!({ "error": format!("read frontend asset failed: {error}") }),
            ),
        }
    }
}

fn error_reply(status: u16, error: &str) -> IngressReply {
    IngressReply::Response(HttpResponse::json(status, json!({ "error": error })))
}

fn request_path(path: &str) -> &str {
    path.split('?').next().unwrap_or(path)
}

fn query_param(path: &str, key: &str) -> Option<String> {
    let query = path.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let (name, value) = pair.split_once('=')?;
        (name == key)
            .then(|| {
                urlencoding::decode(value)
                    .ok()
                    .map(|value| value.into_owned())
            })
            .flatten()
    })
}

fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|extension| extension.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("ico") => "image/x-icon",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
#[path = "server.test.rs"]
mod tests;
