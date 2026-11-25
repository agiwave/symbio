use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};
use symbio::init::create_root_plugin;
use symbio::symbio_kernel::plugin::{
    Plugin, PluginError, PluginInvokeRequestExt, PluginMessageWire, PluginPayload,
    PluginPayloadWire, PluginSimpleRequest, PluginStopReason,
};
use symbio::symbio_plugin_api::routes::{ROUTE_HOME_GET_HOMEDIR, ROUTE_HOME_RELOAD};
use symbio::symbio_plugin_api::{SymbioKey, VdfsCaller, VDFS_CALLER};
use symbio::symbio_sdk::keys::{PATH, TRACE_ID};
use tracing::{debug, error};

use crate::worker::WorkerSupervisor;

/// One runtime tree shared by embedded clients and the standalone Server.
pub struct RuntimeHost {
    root: Arc<dyn Plugin>,
    supervisor: Option<Arc<WorkerSupervisor>>,
}

impl RuntimeHost {
    pub async fn create() -> Result<Self, String> {
        let root = create_root_plugin().await;
        let bootstrap = Self {
            root,
            supervisor: None,
        };
        let homedir = bootstrap
            .homedir()
            .await
            .ok()
            .and_then(|info| {
                info.get("homedir")
                    .and_then(Value::as_str)
                    .map(PathBuf::from)
            })
            .filter(|path| path.is_absolute())
            .unwrap_or_else(|| {
                std::env::var_os("SYMBIO_HOMEDIR")
                    .map(PathBuf::from)
                    .or_else(|| dirs::home_dir().map(|home| home.join(".symbio")))
                    .unwrap_or_else(|| PathBuf::from(".symbio"))
            });
        let supervisor = WorkerSupervisor::new(homedir).map_err(|error| error.to_string())?;
        Ok(Self {
            root: bootstrap.root,
            supervisor: Some(Arc::new(supervisor)),
        })
    }

    pub fn from_root(root: Arc<dyn Plugin>) -> Self {
        Self {
            root,
            supervisor: None,
        }
    }

    pub fn root(&self) -> Arc<dyn Plugin> {
        Arc::clone(&self.root)
    }

    pub async fn invoke(&self, message: PluginMessageWire) -> Result<PluginPayload, String> {
        if let Some(supervisor) = &self.supervisor {
            if let Some(payload) = supervisor
                .invoke_delete(message.clone(), &self.root)
                .await?
            {
                return Ok(payload);
            }
            if let Some(reply) = supervisor.invoke(message.clone()).await? {
                return match serde_json::from_value::<PluginPayloadWire>(reply.payload)
                    .map_err(|error| error.to_string())?
                {
                    PluginPayloadWire::Data(value) => Ok(PluginPayload::new(&value)),
                    PluginPayloadWire::Connection(_) => {
                        Err("worker 不应返回跨进程会话通道".to_string())
                    }
                };
            }
        }
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
        let is_reload = path == ROUTE_HOME_RELOAD;
        debug!(trace_id = %trace_id, path = %path, "RuntimeHost route");
        let result = self.root.clone().route(context).await.map_err(|e| {
            error!(trace_id = %trace_id, path = %path, error = %e, "RuntimeHost route failed");
            e.to_string()
        });
        if is_reload && result.is_ok() {
            if let Some(supervisor) = &self.supervisor {
                if let Ok(info) = self.control(ROUTE_HOME_GET_HOMEDIR).await {
                    if let Some(home) = info.get("homedir").and_then(Value::as_str) {
                        supervisor.set_homedir(PathBuf::from(home)).await;
                    }
                }
            }
        }
        result
    }

    async fn control(&self, path: &str) -> Result<Value, String> {
        let context = Arc::new(PluginSimpleRequest::new(None, None));
        context.set(PATH, path.to_string());
        context.set(VDFS_CALLER, VdfsCaller::Manager);
        context
            .set_payload(json!({}))
            .map_err(|error| error.to_string())?;
        let payload = self
            .root
            .clone()
            .route(context)
            .await
            .map_err(|error| error.to_string())?;
        payload.get::<Value>()
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
        self.control(ROUTE_HOME_GET_HOMEDIR).await
    }

    pub async fn shutdown(&self) -> Result<(), PluginError> {
        if let Some(supervisor) = &self.supervisor {
            supervisor.shutdown().await;
        }
        self.root.clone().stop(PluginStopReason::Shutdown).await
    }
}
