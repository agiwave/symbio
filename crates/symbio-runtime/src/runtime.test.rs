use super::*;
use async_trait::async_trait;
use serde_json::json;
use std::sync::Mutex;
use symbio_platform::kernel::plugin::{
    Plugin, PluginInvokeRequest, PluginInvokeResponse, PluginMeta,
};

type CapturedCalls = Arc<Mutex<Vec<(Option<VdfsCaller>, Vec<String>)>>>;

struct CaptureRoot {
    calls: CapturedCalls,
}

#[async_trait]
impl Plugin for CaptureRoot {
    fn meta(&self) -> PluginMeta {
        PluginMeta::new("capture-root", "capture-root")
    }

    async fn route(
        self: Arc<Self>,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        let roots = ctx
            .get_raw(HOST_VDFS_PROTECTED_ROOTS)
            .and_then(|value| value.downcast::<Vec<String>>().ok())
            .map(|roots| roots.as_ref().clone())
            .unwrap_or_default();
        let caller = ctx.get(VDFS_CALLER);
        self.calls.lock().unwrap().push((caller, roots));
        Ok(PluginPayload::Empty)
    }

    async fn traverse(
        self: Arc<Self>,
        _path: String,
        _ctx: Arc<dyn PluginInvokeRequest>,
    ) -> PluginInvokeResponse<PluginPayload> {
        Ok(PluginPayload::Empty)
    }
}

#[tokio::test]
async fn control_plane_context_receives_plugin_roots_without_protecting_the_workspace() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path();
    for dir in ["model", "work", "worker", "ordinary"] {
        std::fs::create_dir_all(home.join(dir)).expect("directory");
    }
    for manifest in ["PLUGIN.yml", "model/PLUGIN.yml", "work/PLUGIN.yml"] {
        std::fs::write(home.join(manifest), "{}").expect("manifest");
    }
    let calls = Arc::new(Mutex::new(Vec::new()));
    let host = RuntimeHost {
        root: Arc::new(CaptureRoot {
            calls: Arc::clone(&calls),
        }),
        supervisor: None,
        homedir: Arc::new(tokio::sync::RwLock::new(home.to_path_buf())),
    };
    let message = PluginMessageWire {
        metadata: json!({ "path": "vdfs/read" }),
        payload: json!({}),
    };

    host.invoke(message).await.unwrap();

    let mut expected = vec![
        home.join("PLUGIN.yml").to_string_lossy().into_owned(),
        home.join("model").to_string_lossy().into_owned(),
        home.join("worker").to_string_lossy().into_owned(),
        home.join("work")
            .join(PLUGIN_FILE)
            .to_string_lossy()
            .into_owned(),
    ];
    expected.sort();
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        &[(Some(VdfsCaller::Manager), expected)]
    );
}

#[tokio::test]
async fn dispatcher_leaves_model_control_routes_to_the_gateway_parent() {
    let temp = tempfile::tempdir().expect("tempdir");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let host = Arc::new(RuntimeHost {
        root: Arc::new(CaptureRoot {
            calls: Arc::clone(&calls),
        }),
        supervisor: None,
        homedir: Arc::new(tokio::sync::RwLock::new(temp.path().to_path_buf())),
    });
    let dispatcher = RuntimeDispatcher {
        host: OnceLock::new(),
    };
    assert!(dispatcher.host.set(Arc::downgrade(&host)).is_ok());

    for path in [ROUTE_HOME_GET_HOMEDIR, "session/stats", "session/changes"] {
        let message = PluginMessageWire {
            metadata: json!({ "path": path }),
            payload: json!({}),
        };
        assert!(
            dispatcher
                .invoke_session(message, VdfsCaller::Model)
                .await
                .is_none(),
            "path={path}"
        );
    }
    assert!(calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn dispatcher_routes_model_worker_session_calls_through_runtime() {
    let temp = tempfile::tempdir().expect("tempdir");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let host = Arc::new(RuntimeHost {
        root: Arc::new(CaptureRoot {
            calls: Arc::clone(&calls),
        }),
        supervisor: None,
        homedir: Arc::new(tokio::sync::RwLock::new(temp.path().to_path_buf())),
    });
    let dispatcher = RuntimeDispatcher {
        host: OnceLock::new(),
    };
    assert!(dispatcher.host.set(Arc::downgrade(&host)).is_ok());
    let message = PluginMessageWire {
        metadata: json!({ "path": "session/consume", "session_id": "s1" }),
        payload: json!({}),
    };

    let result = dispatcher
        .invoke_session(message, VdfsCaller::Model)
        .await
        .expect("session call")
        .expect("session result");
    assert!(matches!(result, PluginPayload::Empty));
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, Some(VdfsCaller::Model));
}

#[tokio::test]
async fn dispatcher_preserves_model_identity_for_gateway_vdfs_calls() {
    let temp = tempfile::tempdir().expect("tempdir");
    let home = temp.path();
    std::fs::create_dir_all(home.join("model")).expect("model dir");
    std::fs::write(home.join("model/PLUGIN.yml"), "{}").expect("manifest");
    let calls = Arc::new(Mutex::new(Vec::new()));
    let host = Arc::new(RuntimeHost {
        root: Arc::new(CaptureRoot {
            calls: Arc::clone(&calls),
        }),
        supervisor: None,
        homedir: Arc::new(tokio::sync::RwLock::new(home.to_path_buf())),
    });
    let dispatcher = RuntimeDispatcher {
        host: OnceLock::new(),
    };
    assert!(dispatcher.host.set(Arc::downgrade(&host)).is_ok());
    let message = PluginMessageWire {
        metadata: json!({ "path": "vdfs/read", VDFS_CALLER.name(): "manager" }),
        payload: json!({}),
    };

    let result = dispatcher
        .invoke_session(message, VdfsCaller::Model)
        .await
        .expect("vdfs call")
        .expect("vdfs result");
    assert!(matches!(result, PluginPayload::Empty));
    let calls = calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, Some(VdfsCaller::Model));
    assert!(calls[0]
        .1
        .contains(&home.join("model").to_string_lossy().into_owned()));
}
