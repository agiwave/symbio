use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};
use symbio_http::{
    ingress_policy::{reserve_idempotency, Idempotency},
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
    /// `None` = 用编译期内嵌的 `packages/ui/dist`（默认）；`Some` = 从外部磁盘目录读。
    pub frontend_dist: Option<PathBuf>,
    pub allowed_origin: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1".to_string(),
            port: 9231,
            token: String::new(),
            max_connections: 64,
            frontend_dist: None,
            allowed_origin: None,
        }
    }
}

/// 编译期内嵌 UI 的注入接口。
///
/// 由**交付物入口**实现并注入（CLI 用自己的 `embedded_ui` 模块），runtime 只认这条
/// 窄接口、不依赖任何具体资源 crate。只有 CLI serve 需要内嵌 UI，它经此接口注入，
/// 嵌入，不需要也不应该经过这里——否则桌面二进制会被迫多嵌一份同样的 dist。
pub trait EmbeddedUi: Send + Sync + 'static {
    /// 按 URL 路径取资源；查不到返回 `None`（由调用方决定 SPA 回退）。
    fn get(path: &str) -> Option<&'static [u8]>;
    /// 按资源路径给出 MIME 类型。
    fn content_type(path: &str) -> &'static str;
}

/// 静态资源来源：CLI 默认内嵌（二进制自带 UI），显式 `--frontend-dist` 才走磁盘。
enum FrontendSource {
    /// 入口注入的内嵌资源查表函数（见 [`EmbeddedUi`]）。
    Embedded {
        get: fn(&str) -> Option<&'static [u8]>,
        content_type: fn(&str) -> &'static str,
    },
    Disk(PathBuf),
}

pub struct ServerIngress {
    host: Arc<RuntimeHost>,
    frontend: FrontendSource,
    runtime_gate: Arc<RwLock<()>>,
    realtime: Arc<RealtimeConnections>,
    idempotency: Arc<Idempotency>,
}

impl ServerIngress {
    /// 磁盘资源构造器（测试夹具与显式 `--frontend-dist` 路径）。
    pub fn new(host: Arc<RuntimeHost>, frontend_dist: PathBuf) -> Self {
        Self::with_source(host, FrontendSource::Disk(frontend_dist))
    }

    /// 内嵌资源构造器：入口注入的内嵌资源表（CLI → 自有 `embedded_ui` 模块）。
    pub fn embedded<U: EmbeddedUi>(host: Arc<RuntimeHost>) -> Self {
        Self::with_source(
            host,
            FrontendSource::Embedded {
                get: U::get,
                content_type: U::content_type,
            },
        )
    }

    fn with_source(host: Arc<RuntimeHost>, frontend: FrontendSource) -> Self {
        Self {
            host,
            frontend,
            runtime_gate: Arc::new(RwLock::new(())),
            realtime: Arc::new(RealtimeConnections::default()),
            idempotency: Idempotency::new(),
        }
    }
}

/// 启动浏览器 Server。
///
/// `U` 由交付物入口注入：CLI 传自己的 `embedded_ui` 适配类型。runtime 不持有
/// 任何具体资源 crate 的依赖——桌面端因此不会被迫嵌入第二份 UI。
pub async fn start<U: EmbeddedUi>(
    config: ServerConfig,
    host: Arc<RuntimeHost>,
) -> Result<ServerHandle, String> {
    let ingress = match &config.frontend_dist {
        None => Arc::new(ServerIngress::embedded::<U>(host)),
        Some(dist) => {
            let canonical = std::fs::canonicalize(dist)
                .map_err(|e| format!("前端静态目录不可用 {}: {e}", dist.display()))?;
            if !canonical.is_dir() {
                return Err(format!("前端静态路径不是目录: {}", canonical.display()));
            }
            Arc::new(ServerIngress::new(host, canonical))
        }
    };
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
                let ticket = match reserve_idempotency(&request, &self.idempotency) {
                    Ok(ticket) => ticket,
                    Err(response) => return IngressReply::Response(response),
                };
                let message = match serde_json::from_slice::<PluginMessageWire>(&request.body) {
                    Ok(message) => message,
                    Err(error) => {
                        if let Some(ticket) = &ticket {
                            ticket.release();
                        }
                        return error_reply(400, &format!("invalid PluginMessageWire: {error}"));
                    }
                };
                match self.invoke(message, cancel).await {
                    Ok(payload) => match payload_response(payload).await {
                        Ok(response) => IngressReply::Response(response),
                        Err(error) => {
                            if let Some(ticket) = &ticket {
                                ticket.release();
                            }
                            error_reply(500, &error)
                        }
                    },
                    Err(error) => {
                        if let Some(ticket) = &ticket {
                            ticket.release();
                        }
                        error_reply(400, &error)
                    }
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
        match &self.frontend {
            FrontendSource::Embedded { .. } => self.embedded_static_response(&decoded),
            FrontendSource::Disk(root) => self.disk_static_response(root, relative).await,
        }
    }

    /// 内嵌资源：查入口注入的编译期资源表；SPA 路由未命中时回退 index.html。
    // panic-allow crates/symbio-runtime/src/server.rs::embedded_static_response: 本函数只由 static_response 在 FrontendSource::Embedded 分支调用，Disk 分支不可达；不标注会被 panic 审计判红。
    fn embedded_static_response(&self, decoded: &str) -> HttpResponse {
        let (get, content_type) = match &self.frontend {
            FrontendSource::Embedded { get, content_type } => (*get, *content_type),
            FrontendSource::Disk(_) => unreachable!("embedded_static_response 只服务内嵌来源"),
        };
        let key = decoded.trim_start_matches('/');
        // 根路径（""）在资源表里映射到 index.html，MIME 必须按**实际资源名**取；
        // 直接用空串会让 content_type 落到 application/octet-stream。
        let resource = if key.is_empty() { "index.html" } else { key };
        let (path, body) = match get(resource) {
            Some(body) => (resource, body),
            None => ("index.html", get("index.html").unwrap_or_default()),
        };
        if body.is_empty() {
            return HttpResponse::json(404, json!({ "error": "frontend asset not found" }));
        }
        static_ok(path, content_type(path), body)
    }

    /// 磁盘资源：保留原有 canonicalize 防穿越 + SPA 回退，供显式 `--frontend-dist`。
    async fn disk_static_response(&self, root: &Path, relative: &Path) -> HttpResponse {
        let mut candidate = root.join(relative);
        if candidate.is_dir() || !candidate.exists() {
            candidate = root.join("index.html");
        }
        let canonical = match tokio::fs::canonicalize(&candidate).await {
            Ok(path) if path.starts_with(root) => path,
            Ok(_) => return HttpResponse::json(403, json!({ "error": "forbidden" })),
            Err(_) => {
                return HttpResponse::json(404, json!({ "error": "frontend asset not found" }))
            }
        };
        match tokio::fs::read(&canonical).await {
            Ok(body) => static_ok(
                canonical.to_string_lossy().as_ref(),
                content_type(&canonical),
                &body,
            ),
            Err(error) => HttpResponse::json(
                500,
                json!({ "error": format!("read frontend asset failed: {error}") }),
            ),
        }
    }
}

/// 两条资源来源共用的 200 响应：不可变资源长缓存，HTML 不缓存。
fn static_ok(path: &str, content_type: &str, body: &[u8]) -> HttpResponse {
    let immutable = path.rsplit_once('.').is_some_and(|(_, ext)| ext != "html");
    HttpResponse {
        status: 200,
        content_type: content_type.to_string(),
        body: body.to_vec(),
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
