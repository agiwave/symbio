use std::sync::Arc;

use serde_json::{json, Value};
use symbio::init::create_root_plugin;
use symbio::symbio_kernel::plugin::{
    Plugin, PluginError, PluginInvokeRequestExt, PluginMessageWire, PluginPayload,
    PluginSimpleRequest, PluginStopReason,
};
use symbio::symbio_plugin_api::routes::ROUTE_HOME_GET_HOMEDIR;
use symbio::symbio_plugin_api::{SymbioKey, VdfsCaller, VDFS_CALLER};
use symbio::symbio_sdk::keys::TRACE_ID;
use tracing::{debug, error};

/// One runtime tree shared by embedded clients and the standalone Server.
pub struct RuntimeHost {
    root: Arc<dyn Plugin>,
}

impl RuntimeHost {
    pub async fn create() -> Self {
        Self {
            root: create_root_plugin().await,
        }
    }

    pub fn from_root(root: Arc<dyn Plugin>) -> Self {
        Self { root }
    }

    pub fn root(&self) -> Arc<dyn Plugin> {
        Arc::clone(&self.root)
    }

    pub async fn invoke(&self, message: PluginMessageWire) -> Result<PluginPayload, String> {
        let path = message
            .metadata
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let trace_id = message
            .metadata
            .get(TRACE_ID.name())
            .and_then(Value::as_str)
            .unwrap_or("-");
        let context = Arc::new(PluginSimpleRequest::from_parts(
            message.payload,
            message.metadata.as_object(),
        ));
        context.set(VDFS_CALLER, VdfsCaller::Manager);
        debug!(trace_id = %trace_id, path = %path, "RuntimeHost route");
        self.root.clone().route(context).await.map_err(|e| {
            error!(trace_id = %trace_id, path = %path, error = %e, "RuntimeHost route failed");
            e.to_string()
        })
    }

    pub async fn subscribe(&self) -> Result<symbio::symbio_kernel::plugin::PluginChannel, String> {
        let context = Arc::new(PluginSimpleRequest::new(None, None));
        context.set(VDFS_CALLER, VdfsCaller::Manager);
        context.set(
            symbio::symbio_plugin_api::PATH,
            symbio::symbio_plugin_api::routes::ROUTE_EVENT_BUS_SUBSCRIBE.to_string(),
        );
        context
            .set_payload(json!({}))
            .map_err(|e| format!("构造事件订阅失败: {e}"))?;
        match self
            .root
            .clone()
            .route(context)
            .await
            .map_err(|e| e.to_string())?
        {
            PluginPayload::Session(channel) => Ok(channel),
            _ => Err("event_bus/subscribe 返回了非会话载荷".to_string()),
        }
    }

    pub async fn homedir(&self) -> Result<Value, String> {
        let payload = self
            .invoke(PluginMessageWire {
                metadata: json!({ "path": ROUTE_HOME_GET_HOMEDIR }),
                payload: json!({}),
            })
            .await?;
        payload.get::<Value>()
    }

    pub async fn shutdown(self) -> Result<(), PluginError> {
        self.root.stop(PluginStopReason::Shutdown).await
    }
}
