use std::time::Duration;

use super::*;

#[test]
fn query_param_decodes_server_paths() {
    assert_eq!(
        query_param("/api/v1/host/directories?path=C%3A%5Ctmp", "path").as_deref(),
        Some("C:\\tmp")
    );
}

#[test]
fn traversal_is_rejected_before_asset_lookup() {
    let relative = Path::new("../secret");
    assert!(relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_))));
}

#[tokio::test]
async fn serves_spa_fallback_and_static_assets() {
    let temp = tempfile::tempdir().expect("tempdir");
    std::fs::write(temp.path().join("index.html"), "<div>app</div>").expect("index");
    std::fs::create_dir(temp.path().join("assets")).expect("assets");
    std::fs::write(temp.path().join("assets/app.js"), "console.log(1)").expect("asset");
    let ingress = ServerIngress::new(
        Arc::new(RuntimeHost::from_root(Arc::new(NoopHost))),
        std::fs::canonicalize(temp.path()).expect("canonical tempdir"),
    );
    let fallback = ingress.static_response("/sessions/123").await;
    assert_eq!(fallback.status, 200);
    assert_eq!(fallback.content_type, "text/html; charset=utf-8");
    let asset = ingress.static_response("/assets/app.js").await;
    assert_eq!(asset.status, 200);
    assert_eq!(asset.content_type, "text/javascript; charset=utf-8");
    assert_eq!(ingress.static_response("/%2e%2e/secret").await.status, 403);
}

#[tokio::test]
async fn invoke_and_events_routes_reach_runtime() {
    let temp = tempfile::tempdir().expect("tempdir");
    let ingress = ServerIngress::new(
        Arc::new(RuntimeHost::from_root(Arc::new(NoopHost))),
        std::fs::canonicalize(temp.path()).expect("canonical tempdir"),
    );
    let reply = ingress
        .handle(
            HttpRequest {
                method: "POST".to_string(),
                path: "/api/v1/invoke".to_string(),
                headers: [("content-type".to_string(), "application/json".to_string())]
                    .into_iter()
                    .collect(),
                body: serde_json::to_vec(&json!({
                    "metadata": { "path": "vdfs/root" },
                    "payload": {},
                }))
                .expect("request"),
            },
            CancellationToken::new(),
        )
        .await;
    let IngressReply::Response(response) = reply else {
        panic!("invoke must return an HTTP response");
    };
    assert_eq!(response.status, 200);

    let reply = ingress
        .handle(
            HttpRequest {
                method: "GET".to_string(),
                path: "/api/v1/events".to_string(),
                headers: Default::default(),
                body: Vec::new(),
            },
            CancellationToken::new(),
        )
        .await;
    let IngressReply::Response(response) = reply else {
        panic!("non-channel subscription must return an HTTP error");
    };
    assert_eq!(response.status, 500);
}

#[tokio::test]
async fn invoke_rejects_non_json_content_type() {
    let temp = tempfile::tempdir().expect("tempdir");
    let ingress = ServerIngress::new(
        Arc::new(RuntimeHost::from_root(Arc::new(NoopHost))),
        std::fs::canonicalize(temp.path()).expect("canonical tempdir"),
    );
    let reply = ingress
        .handle(
            HttpRequest {
                method: "POST".to_string(),
                path: "/api/v1/invoke".to_string(),
                headers: [("content-type".to_string(), "text/plain".to_string())]
                    .into_iter()
                    .collect(),
                body: serde_json::to_vec(&json!({
                    "metadata": { "path": "vdfs/root" },
                    "payload": {},
                }))
                .expect("request"),
            },
            CancellationToken::new(),
        )
        .await;
    let IngressReply::Response(response) = reply else {
        panic!("non-JSON invoke must return an HTTP response");
    };
    assert_eq!(response.status, 415);
}

#[tokio::test]
async fn homedir_reload_cancels_and_drains_realtime_connections() {
    let temp = tempfile::tempdir().expect("tempdir");
    let ingress = ServerIngress::new(
        Arc::new(RuntimeHost::from_root(Arc::new(NoopHost))),
        std::fs::canonicalize(temp.path()).expect("canonical tempdir"),
    );
    let first = CancellationToken::new();
    let second = CancellationToken::new();
    ingress.realtime.enter(first.clone());
    ingress.realtime.enter(second.clone());

    let registry = Arc::clone(&ingress.realtime);
    let waiting = tokio::spawn(async move { registry.cancel_and_wait(None).await });
    tokio::time::timeout(Duration::from_secs(1), async {
        while !first.is_cancelled() || !second.is_cancelled() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("realtime tokens should be cancelled");
    assert!(!waiting.is_finished());
    ingress.realtime_closed(first).await;
    assert_eq!(ingress.realtime.active.load(Ordering::Acquire), 1);
    assert!(!waiting.is_finished());
    ingress.realtime_closed(second).await;
    assert_eq!(ingress.realtime.active.load(Ordering::Acquire), 0);
    tokio::time::timeout(Duration::from_secs(1), waiting)
        .await
        .expect("realtime drain should finish")
        .expect("drain task");

    let token = CancellationToken::new();
    ingress.realtime.enter(token.clone());
    let ingress = Arc::new(ingress);
    let reloading = {
        let ingress = Arc::clone(&ingress);
        tokio::spawn(async move {
            ingress
                .invoke(
                    PluginMessageWire {
                        metadata: json!({ "path": ROUTE_HOME_RELOAD }),
                        payload: json!({}),
                    },
                    CancellationToken::new(),
                )
                .await
        })
    };
    tokio::time::timeout(Duration::from_secs(1), async {
        while !token.is_cancelled() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("reload should cancel realtime tokens");
    ingress.realtime_closed(token).await;
    tokio::time::timeout(Duration::from_secs(1), reloading)
        .await
        .expect("reload should finish after realtime drain")
        .expect("reload task")
        .expect("reload route");
    assert_eq!(ingress.realtime.active.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn websocket_reload_excludes_its_own_connection_from_drain() {
    let temp = tempfile::tempdir().expect("tempdir");
    let ingress = Arc::new(ServerIngress::new(
        Arc::new(RuntimeHost::from_root(Arc::new(NoopHost))),
        std::fs::canonicalize(temp.path()).expect("canonical tempdir"),
    ));
    let current = CancellationToken::new();
    let other = CancellationToken::new();
    ingress.realtime.enter(current.clone());
    ingress.realtime.enter(other.clone());
    let reloading = {
        let ingress = Arc::clone(&ingress);
        let current = current.clone();
        tokio::spawn(async move {
            ingress
                .invoke(
                    PluginMessageWire {
                        metadata: json!({ "path": ROUTE_HOME_RELOAD }),
                        payload: json!({}),
                    },
                    current,
                )
                .await
        })
    };
    tokio::time::timeout(Duration::from_secs(1), async {
        while !other.is_cancelled() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("other realtime connections should close");
    assert!(!current.is_cancelled());
    ingress.realtime_closed(other).await;
    tokio::time::timeout(Duration::from_secs(1), reloading)
        .await
        .expect("current WebSocket must not block its own reload")
        .expect("reload task")
        .expect("reload route");
    assert_eq!(ingress.realtime.active.load(Ordering::Acquire), 1);
    ingress.realtime_closed(current).await;
    assert_eq!(ingress.realtime.active.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn concurrent_websocket_reloads_cancel_gate_waiters() {
    let temp = tempfile::tempdir().expect("tempdir");
    let ingress = Arc::new(ServerIngress::new(
        Arc::new(RuntimeHost::from_root(Arc::new(NoopHost))),
        std::fs::canonicalize(temp.path()).expect("canonical tempdir"),
    ));
    let first = CancellationToken::new();
    let second = CancellationToken::new();
    ingress.realtime.enter(first.clone());
    ingress.realtime.enter(second.clone());
    let gate = ingress.runtime_gate.write().await;
    let spawn_reload = |token: CancellationToken| {
        let ingress = Arc::clone(&ingress);
        tokio::spawn(async move {
            ingress
                .invoke(
                    PluginMessageWire {
                        metadata: json!({ "path": ROUTE_HOME_RELOAD }),
                        payload: json!({}),
                    },
                    token,
                )
                .await
        })
    };
    let first_task = spawn_reload(first.clone());
    tokio::task::yield_now().await;
    let second_task = spawn_reload(second.clone());
    tokio::task::yield_now().await;
    drop(gate);

    tokio::time::timeout(Duration::from_secs(1), async {
        while !second.is_cancelled() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the queued reload must observe cancellation");
    assert!(!first.is_cancelled());
    ingress.realtime_closed(second).await;
    let cancelled = tokio::time::timeout(Duration::from_secs(1), second_task)
        .await
        .expect("cancelled gate waiter must return")
        .expect("reload task");
    assert!(matches!(cancelled, Err(ref error) if error == "request cancelled"));
    tokio::time::timeout(Duration::from_secs(1), first_task)
        .await
        .expect("the active reload must finish")
        .expect("reload task")
        .expect("reload route");
    ingress.realtime_closed(first).await;
    assert_eq!(ingress.realtime.active.load(Ordering::Acquire), 0);
}

#[tokio::test]
async fn cancelled_invocation_releases_the_runtime_gate() {
    let temp = tempfile::tempdir().expect("tempdir");
    let host = Arc::new(BlockingHost {
        started: Notify::new(),
    });
    let ingress = Arc::new(ServerIngress::new(
        Arc::new(RuntimeHost::from_root(host.clone())),
        std::fs::canonicalize(temp.path()).expect("canonical tempdir"),
    ));
    let cancel = CancellationToken::new();
    let invoking = {
        let ingress = Arc::clone(&ingress);
        let cancel = cancel.clone();
        tokio::spawn(async move {
            ingress
                .invoke(
                    PluginMessageWire {
                        metadata: json!({ "path": "block" }),
                        payload: json!({}),
                    },
                    cancel,
                )
                .await
        })
    };
    tokio::time::timeout(Duration::from_secs(1), host.started.notified())
        .await
        .expect("route should start");
    cancel.cancel();
    let result = tokio::time::timeout(Duration::from_secs(1), invoking)
        .await
        .expect("cancelled invocation must return")
        .expect("invoke task");
    assert!(matches!(result, Err(ref error) if error == "request cancelled"));
    assert!(ingress.runtime_gate.try_read().is_ok());
}

struct BlockingHost {
    started: Notify,
}

#[async_trait]
impl symbio_platform::kernel::plugin::Plugin for BlockingHost {
    fn meta(&self) -> symbio_platform::kernel::plugin::PluginMeta {
        symbio_platform::kernel::plugin::PluginMeta::new("blocking", "blocking")
    }

    async fn route(
        self: Arc<Self>,
        _context: Arc<dyn symbio_platform::kernel::plugin::PluginInvokeRequest>,
    ) -> symbio_platform::kernel::plugin::PluginInvokeResponse<PluginPayload> {
        self.started.notify_one();
        std::future::pending().await
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _context: Arc<dyn symbio_platform::kernel::plugin::PluginInvokeRequest>,
    ) -> symbio_platform::kernel::plugin::PluginInvokeResponse<PluginPayload> {
        Ok(PluginPayload::new(&Vec::<Value>::new()))
    }
}

struct NoopHost;

#[async_trait]
impl symbio_platform::kernel::plugin::Plugin for NoopHost {
    fn meta(&self) -> symbio_platform::kernel::plugin::PluginMeta {
        symbio_platform::kernel::plugin::PluginMeta::new("noop", "noop")
    }

    async fn route(
        self: Arc<Self>,
        _context: Arc<dyn symbio_platform::kernel::plugin::PluginInvokeRequest>,
    ) -> symbio_platform::kernel::plugin::PluginInvokeResponse<PluginPayload> {
        Ok(PluginPayload::new(&json!({ "ok": true })))
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _context: Arc<dyn symbio_platform::kernel::plugin::PluginInvokeRequest>,
    ) -> symbio_platform::kernel::plugin::PluginInvokeResponse<PluginPayload> {
        Ok(PluginPayload::new(&Vec::<Value>::new()))
    }
}
