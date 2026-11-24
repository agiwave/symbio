use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use symbio::init::create_root_plugin;
use symbio::symbio_kernel::plugin::{
    Plugin, PluginInvokeRequestExt, PluginMessageWire, PluginPayload, PluginPayloadWire,
    PluginSimpleRequest,
};
use symbio::symbio_plugin_api::routes::{ROUTE_EVENT_BUS_SUBSCRIBE, ROUTE_SESSION_WORKER_READY};
use symbio::symbio_plugin_api::EVENT_BUS_KIND_SYSTEM;
use symbio::symbio_plugin_api::{VdfsCaller, VDFS_CALLER};
use symbio::symbio_sdk::keys::{PATH, SESSION_ID};
use tokio::io::{stdin, stdout, BufReader};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{error, warn};

use super::epoch::EpochRegistry;
use super::protocol::{read_line, write_line, WorkerLine};
use super::{DurableInbox, RecoveryReport, WorkerError, WorkerId, WorkerRecovery, WorkerStartup};

#[derive(Debug, Clone)]
pub struct WorkerBootstrap {
    pub worker: WorkerId,
    pub homedir: PathBuf,
}

struct PluginRecovery {
    root: Arc<dyn Plugin>,
    inbox: Arc<DurableInbox>,
}

#[async_trait]
impl WorkerRecovery for PluginRecovery {
    async fn recover(&self, worker: &WorkerId) -> Result<RecoveryReport, WorkerError> {
        let ctx = Arc::new(PluginSimpleRequest::new(None, None));
        ctx.set(PATH, ROUTE_SESSION_WORKER_READY.to_string());
        ctx.set(SESSION_ID, worker.session_id().to_string());
        ctx.set(VDFS_CALLER, VdfsCaller::Manager);
        ctx.set_payload(json!({}))
            .map_err(|error| WorkerError::Invalid(error.to_string()))?;
        self.root
            .clone()
            .route(ctx)
            .await
            .map_err(|error| WorkerError::Invalid(error.to_string()))?;
        Ok(RecoveryReport {
            snapshot: self.inbox.recover()?,
        })
    }
}

async fn route(
    root: &Arc<dyn Plugin>,
    message: PluginMessageWire,
) -> Result<PluginPayloadWire, String> {
    let ctx = Arc::new(PluginSimpleRequest::from_parts(
        message.payload,
        message.metadata.as_object(),
    ));
    ctx.set(VDFS_CALLER, VdfsCaller::Manager);
    match root
        .clone()
        .route(ctx)
        .await
        .map_err(|error| error.to_string())?
    {
        PluginPayload::Data(value) => Ok(PluginPayloadWire::Data(value.serialize()?)),
        PluginPayload::Empty => Ok(PluginPayloadWire::Data(Value::Null)),
        PluginPayload::Session(_) => Err("worker 不提供跨进程会话通道".to_string()),
    }
}

async fn execute(
    root: &Arc<dyn Plugin>,
    inbox: &DurableInbox,
    worker: &WorkerId,
    id: &super::RequestId,
    message: PluginMessageWire,
) -> Result<PluginPayloadWire, String> {
    inbox.claim(worker, id).map_err(|error| error.to_string())?;
    let result = route(root, message).await;
    inbox
        .complete(worker, id)
        .map_err(|error| error.to_string())?;
    result
}

pub async fn run_worker(bootstrap: WorkerBootstrap) -> Result<(), String> {
    let WorkerBootstrap { worker, homedir } = bootstrap;
    std::env::set_var("SYMBIO_HOMEDIR", &homedir);
    std::fs::create_dir_all(&homedir)
        .map_err(|error| format!("创建系统目录失败 {}: {error}", homedir.display()))?;

    let session_dir = session_dir(&homedir, worker.session_id());
    let root = create_root_plugin().await;
    let inbox = Arc::new(
        DurableInbox::new(worker.session_id(), &session_dir).map_err(|error| error.to_string())?,
    );
    let recovery: Arc<dyn WorkerRecovery> = Arc::new(PluginRecovery {
        root: Arc::clone(&root),
        inbox: Arc::clone(&inbox),
    });
    let ready = WorkerStartup::new(
        recovery,
        EpochRegistry::new(worker.session_id(), &session_dir).map_err(|error| error.to_string())?,
    )
    .ready(worker.clone())
    .await
    .map_err(|error| error.to_string())?;

    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<WorkerLine>();
    let mut stdout = stdout();
    let writer = tokio::spawn(async move {
        while let Some(line) = out_rx.recv().await {
            if let Err(error) = write_line(&mut stdout, &line).await {
                error!(%error, "worker stdout 关闭");
                break;
            }
        }
    });
    out_tx
        .send(WorkerLine::Ready {
            worker: worker.clone(),
        })
        .map_err(|_| "worker 输出通道已关闭".to_string())?;

    let stop_events = CancellationToken::new();
    let event_root = Arc::clone(&root);
    let event_tx = out_tx.clone();
    let event_stop = stop_events.clone();
    let events = tokio::spawn(async move {
        forward_events(event_root, event_tx, event_stop).await;
    });

    for pending in ready.recovery().snapshot.pending.clone() {
        let message = PluginMessageWire {
            metadata: pending.request().message().clone(),
            payload: pending.request().params().clone(),
        };
        if let Err(error) = execute(&root, &inbox, &worker, pending.request().id(), message).await {
            warn!(%error, "恢复请求执行失败");
        }
    }

    let mut stdin = BufReader::new(stdin());
    loop {
        let line: Option<String> = read_line(&mut stdin)
            .await
            .map_err(|error| error.to_string())?;
        let Some(line) = line else { break };
        let line: WorkerLine =
            serde_json::from_str(&line).map_err(|error| format!("无效 worker 请求帧: {error}"))?;
        let WorkerLine::Request {
            id,
            worker: claimed,
            message,
        } = line
        else {
            return Err("worker 收到非请求帧".to_string());
        };
        claimed
            .validate(&worker)
            .map_err(|error| error.to_string())?;
        let result = execute(&root, &inbox, &worker, &id, message).await;
        let result =
            result.and_then(|payload| serde_json::to_value(payload).map_err(|e| e.to_string()));
        out_tx
            .send(WorkerLine::Response { id, result })
            .map_err(|_| "worker 输出通道已关闭".to_string())?;
    }

    stop_events.cancel();
    drop(out_tx);
    if let Err(error) = events.await {
        warn!(%error, "worker 事件任务异常退出");
    }
    if let Err(error) = writer.await {
        warn!(%error, "worker 输出任务异常退出");
    }
    let _ = ready;
    Ok(())
}

async fn forward_events(
    root: Arc<dyn Plugin>,
    tx: mpsc::UnboundedSender<WorkerLine>,
    stop: CancellationToken,
) {
    let ctx = Arc::new(PluginSimpleRequest::new(None, None));
    ctx.set(PATH, ROUTE_EVENT_BUS_SUBSCRIBE.to_string());
    ctx.set(VDFS_CALLER, VdfsCaller::Manager);
    if let Err(error) = ctx.set_payload(json!({})) {
        error!(%error, "worker 订阅事件失败");
        return;
    }
    let Ok(PluginPayload::Session(channel)) = root.clone().route(ctx).await else {
        error!("worker 事件订阅未返回会话通道");
        return;
    };
    let mut rx: mpsc::Receiver<symbio::symbio_kernel::plugin::PluginFrame> = channel.rx;
    loop {
        tokio::select! {
            _ = stop.cancelled() => break,
            frame = rx.recv() => {
                let Some(frame) = frame else { break };
                let is_system = frame
                    .value()
                    .and_then(|value| value.get("data"))
                    .and_then(|inner| inner.get("kind"))
                    .and_then(Value::as_str)
                    == Some(EVENT_BUS_KIND_SYSTEM);
                if is_system || tx.send(WorkerLine::Event { frame }).is_err() {
                    break;
                }
            }
        }
    }
}

fn session_dir(homedir: &Path, session_id: &str) -> PathBuf {
    homedir.join("session").join(session_id)
}
