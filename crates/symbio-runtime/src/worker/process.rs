use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use crate::root_factory::RootFactory;
use async_trait::async_trait;
use serde_json::{json, Value};
use symbio_platform::kernel::exec::ExecAbortSignal;
use symbio_platform::kernel::plugin::{
    Plugin, PluginError, PluginFrame, PluginInvokeRequest, PluginInvokeRequestExt,
    PluginMessageWire, PluginPayload, PluginPayloadWire, PluginSimpleRequest,
};
use symbio_platform::sdk::event_bus::EventBus;
use symbio_platform::sdk::keys::{PATH, SESSION_ID};
use symbio_plugin_api::routes::{
    ROUTE_EVENT_BUS_SUBSCRIBE, ROUTE_SESSION_WORKER_READY, ROUTE_VDFS_ROOT, ROUTE_VDFS_WATCH,
};
use symbio_plugin_api::vocab::PORT_SESSION_SUBAGENT;
use symbio_plugin_api::{
    port_result, PortBindings, PortErrorClass, PortExecContext, PortExecutor, PortInput,
    PortOutcome, VdfsCaller, EVENT_BUS_KIND_SYSTEM, PORT_BINDINGS, VDFS_CALLER,
};
use tokio::io::{stdin, stdout, AsyncRead, AsyncWrite, BufReader};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tracing::{error, warn};

use super::broker_shim;
use super::container::MAX_BROKER_STREAMS;
use super::epoch::EpochRegistry;
use super::event_relay::is_relayed;
use super::protocol::{read_line, write_line, WorkerLine, EVENT_CHANNEL_CAPACITY};
use super::{
    DurableInbox, ReconciliationOutcome, RecoveryReport, RecoverySnapshot, RequestId, WorkerError,
    WorkerId, WorkerRecovery, WorkerStartup,
};

type SubagentReplies = Arc<StdMutex<HashMap<RequestId, oneshot::Sender<Result<(), String>>>>>;

#[derive(Debug, Clone)]
pub struct WorkerBootstrap {
    pub worker: WorkerId,
    pub homedir: PathBuf,
    pub session_dir: PathBuf,
}

struct WorkerRuntime {
    root: Arc<dyn Plugin>,
    inbox: Arc<DurableInbox>,
    worker: WorkerId,
    launcher: Arc<StdioLauncher>,
    homedir: PathBuf,
}

struct PluginRecovery {
    runtime: Arc<WorkerRuntime>,
}

#[async_trait]
impl WorkerRecovery for PluginRecovery {
    async fn recover(&self, worker: &WorkerId) -> Result<RecoveryReport, WorkerError> {
        let runtime = &self.runtime;
        let ctx = Arc::new(PluginSimpleRequest::new(None, None));
        crate::runtime::attach_vdfs_protected_roots(ctx.as_ref(), &runtime.homedir);
        ctx.set(PATH, ROUTE_SESSION_WORKER_READY.to_string());
        ctx.set(SESSION_ID, worker.session_id().to_string());
        ctx.set(VDFS_CALLER, VdfsCaller::Manager);
        ctx.set_payload(json!({}))
            .map_err(|error| WorkerError::Invalid(error.to_string()))?;
        install_worker_ports(&ctx, Arc::clone(&runtime.launcher));
        runtime
            .root
            .clone()
            .route(ctx)
            .await
            .map_err(|error| WorkerError::Invalid(error.to_string()))?;
        let snapshot = reconcile_interrupted(&runtime.inbox, worker, &runtime.inbox.recover()?)?;
        Ok(RecoveryReport { snapshot })
    }
}

fn reconcile_interrupted(
    inbox: &DurableInbox,
    current: &WorkerId,
    snapshot: &RecoverySnapshot,
) -> Result<RecoverySnapshot, WorkerError> {
    for interrupted in &snapshot.interrupted {
        inbox.reconcile(
            current,
            interrupted.worker(),
            interrupted.item().id(),
            ReconciliationOutcome::Abandoned,
            Some("worker 中断，结果未知，不自动重放".to_string()),
        )?;
    }
    let mut recovered = snapshot.clone();
    recovered.interrupted.clear();
    Ok(recovered)
}

async fn route_message(
    root: &Arc<dyn Plugin>,
    message: PluginMessageWire,
    caller: VdfsCaller,
    launcher: Arc<StdioLauncher>,
    homedir: &Path,
) -> Result<PluginPayloadWire, String> {
    let ctx = Arc::new(PluginSimpleRequest::from_parts(
        message.payload,
        message.metadata.as_object(),
    ));
    crate::runtime::attach_vdfs_protected_roots(ctx.as_ref(), homedir);
    ctx.set(VDFS_CALLER, caller);
    install_worker_ports(&ctx, launcher);
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

impl WorkerRuntime {
    async fn execute(
        &self,
        id: &super::RequestId,
        message: PluginMessageWire,
        caller: VdfsCaller,
    ) -> Result<PluginPayloadWire, String> {
        self.inbox
            .claim(&self.worker, id)
            .map_err(|error| error.to_string())?;
        let result = route_message(
            &self.root,
            message,
            caller,
            Arc::clone(&self.launcher),
            &self.homedir,
        )
        .await;
        self.inbox
            .complete(&self.worker, id)
            .map_err(|error| error.to_string())?;
        result
    }
}

#[derive(serde::Deserialize)]
struct SubagentRequest {
    child_session_id: String,
    message: PluginMessageWire,
}

struct StdioLauncher {
    tx: std::sync::Weak<mpsc::Sender<WorkerLine>>,
    pending: SubagentReplies,
    worker: WorkerId,
}

impl StdioLauncher {
    fn fail_all(&self, error: &str) {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for (_, reply) in pending.drain() {
            let _ = reply.send(Err(error.to_string()));
        }
    }

    async fn launch(
        &self,
        request: SubagentRequest,
        caller: VdfsCaller,
        abort: ExecAbortSignal,
    ) -> Result<(), String> {
        let id = RequestId::generated(
            's',
            &[
                std::process::id() as u64,
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_nanos() as u64,
            ],
        );
        let (tx, rx) = oneshot::channel();
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(id.clone(), tx);
        let mut pending = PendingSubagent {
            pending: Arc::clone(&self.pending),
            tx: self.tx.clone(),
            id: id.clone(),
            worker: self.worker.clone(),
            cancel_on_drop: true,
        };
        let result = async {
            let output = self
                .tx
                .upgrade()
                .ok_or_else(|| "worker 输出通道已关闭".to_string())?;
            output
                .send(WorkerLine::StartSubagent {
                    id: id.clone(),
                    child_session_id: request.child_session_id,
                    message: request.message,
                    caller,
                })
                .await
                .map_err(|_| "worker 输出通道已关闭".to_string())?;
            let reply = rx
                .await
                .map_err(|_| "控制面没有回应子会话启动".to_string())?;
            pending.cancel_on_drop = false;
            reply
        };
        let result = tokio::select! {
            _ = abort.cancelled() => Err("子会话启动已中止".to_string()),
            result = result => result,
        };
        result
    }
}

struct PendingSubagent {
    pending: SubagentReplies,
    tx: std::sync::Weak<mpsc::Sender<WorkerLine>>,
    id: RequestId,
    worker: WorkerId,
    cancel_on_drop: bool,
}

impl Drop for PendingSubagent {
    fn drop(&mut self) {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&self.id);
        if self.cancel_on_drop {
            if let Some(tx) = self.tx.upgrade() {
                let _ = tx.try_send(WorkerLine::CancelSubagent {
                    id: self.id.clone(),
                    worker: self.worker.clone(),
                });
            }
        }
    }
}

#[async_trait]
impl PortExecutor for StdioLauncher {
    async fn execute(&self, ctx: PortExecContext, input: PortInput) -> PortOutcome {
        if input.port != PORT_SESSION_SUBAGENT {
            return PortOutcome::failed(
                PortErrorClass::NonRetryable,
                format!("未知内部端口 {}", input.port),
            );
        }
        let request = match serde_json::from_value::<SubagentRequest>(input.payload) {
            Ok(request) => request,
            Err(error) => {
                return PortOutcome::failed(
                    PortErrorClass::NonRetryable,
                    format!("子会话启动载荷无效: {error}"),
                );
            }
        };
        let caller = ctx
            .ctx
            .as_ref()
            .and_then(|ctx| ctx.get(VDFS_CALLER))
            .unwrap_or(VdfsCaller::Model);
        port_result(
            self.launch(request, caller, ctx.abort)
                .await
                .map(|_| Value::Null)
                .map_err(PluginError::InternalError),
        )
    }
}

fn install_worker_ports<T: PluginInvokeRequest + 'static>(
    ctx: &Arc<T>,
    launcher: Arc<StdioLauncher>,
) {
    let bindings = PortBindings::default();
    bindings.bind(
        &[PORT_SESSION_SUBAGENT.to_string()],
        "server.worker",
        Some(launcher),
    );
    ctx.set(PORT_BINDINGS, Arc::new(bindings));
}

pub async fn run_worker(
    bootstrap: WorkerBootstrap,
    root_factory: Arc<dyn RootFactory>,
) -> Result<(), String> {
    run_worker_with_io(bootstrap, root_factory, stdin(), stdout()).await
}

/// 会话执行体主循环：stdio 与进程内 duplex 共用同一帧协议与恢复语义。
pub(crate) async fn run_worker_with_io<I, O>(
    bootstrap: WorkerBootstrap,
    root_factory: Arc<dyn RootFactory>,
    input: I,
    output: O,
) -> Result<(), String>
where
    I: AsyncRead + Unpin + Send + 'static,
    O: AsyncWrite + Unpin + Send + 'static,
{
    let WorkerBootstrap {
        worker,
        homedir,
        session_dir,
    } = bootstrap;
    std::env::set_var("SYMBIO_HOMEDIR", &homedir);
    std::fs::create_dir_all(&homedir)
        .map_err(|error| format!("创建系统目录失败 {}: {error}", homedir.display()))?;

    let homedir = std::fs::canonicalize(&homedir)
        .map_err(|error| format!("解析系统目录失败 {}: {error}", homedir.display()))?;
    let session_dir = resolve_session_dir(&homedir, &session_dir, worker.session_id())?;
    let root_ctx = Arc::new(PluginSimpleRequest::new(None, None));
    crate::runtime::attach_vdfs_protected_roots(root_ctx.as_ref(), &homedir);
    let root = root_factory.create_root(root_ctx).await;
    let inbox = Arc::new(
        DurableInbox::new(worker.session_id(), &session_dir).map_err(|error| error.to_string())?,
    );
    let (out_tx, mut out_rx) = mpsc::channel::<WorkerLine>(EVENT_CHANNEL_CAPACITY);
    let out_tx = Arc::new(out_tx);
    let subagent_pending: SubagentReplies = Arc::new(StdMutex::new(HashMap::new()));
    let runtime = Arc::new(WorkerRuntime {
        root: Arc::clone(&root),
        inbox: Arc::clone(&inbox),
        worker: worker.clone(),
        launcher: Arc::new(StdioLauncher {
            tx: Arc::downgrade(&out_tx),
            pending: Arc::clone(&subagent_pending),
            worker: worker.clone(),
        }),
        homedir: homedir.clone(),
    });
    let recovery: Arc<dyn WorkerRecovery> = Arc::new(PluginRecovery {
        runtime: Arc::clone(&runtime),
    });
    let ready = WorkerStartup::new(
        recovery,
        EpochRegistry::new(worker.session_id(), &session_dir).map_err(|error| error.to_string())?,
    )
    .ready(worker.clone())
    .await
    .map_err(|error| error.to_string())?;

    let mut stdout = output;
    let writer = tokio::spawn(async move {
        while let Some(line) = out_rx.recv().await {
            if let Err(error) = write_line(&mut stdout, &line).await {
                error!(%error, "worker stdout 关闭");
                break;
            }
        }
    });
    let event_rx = worker_events(&root, &worker, &homedir).await?;
    let broker = if std::env::var_os("SYMBIO_BROKER_TOKEN").is_some() {
        let token = std::env::var("SYMBIO_BROKER_TOKEN").map_err(|e| e.to_string())?;
        if token.len() < 32 || token.chars().any(char::is_whitespace) {
            return Err("broker 令牌必须是至少 32 字符且无空白的随机值".to_string());
        }
        let streams = std::env::var("SYMBIO_BROKER_STREAMS")
            .map_err(|_| "broker 模式缺少 SYMBIO_BROKER_STREAMS".to_string())?
            .parse::<usize>()
            .map_err(|_| "SYMBIO_BROKER_STREAMS 必须是整数".to_string())?;
        if streams == 0 || streams > MAX_BROKER_STREAMS {
            return Err(format!(
                "SYMBIO_BROKER_STREAMS 必须在 1..={MAX_BROKER_STREAMS}"
            ));
        }
        let listener = broker_shim::bind().await?;
        let client = broker_shim::BrokerClient::new(worker.clone(), out_tx.clone(), token);
        let stop = CancellationToken::new();
        let task_client = client.clone();
        let task_stop = stop.clone();
        let server = tokio::spawn(async move {
            broker_shim::serve(task_client, listener, streams, task_stop).await
        });
        Some((client, stop, server))
    } else {
        None
    };
    let stop_events = CancellationToken::new();
    let event_tx = (*out_tx).clone();
    let event_stop = stop_events.clone();
    let events = tokio::spawn(async move {
        forward_events(event_rx, event_tx, event_stop).await;
    });
    if let Err(error) = out_tx
        .send(WorkerLine::Ready {
            worker: worker.clone(),
        })
        .await
    {
        return Err(format!("worker 输出通道已关闭: {error}"));
    }

    let execution = Arc::new(tokio::sync::Mutex::new(()));
    let recovery_gate = Arc::clone(&execution).lock_owned().await;
    let mut tasks = JoinSet::new();
    let recovery_runtime = Arc::clone(&runtime);
    tasks.spawn(async move {
        let _gate = recovery_gate;
        for pending in ready.recovery().snapshot.pending.clone() {
            let message = PluginMessageWire {
                metadata: pending.request().message().clone(),
                payload: pending.request().params().clone(),
            };
            if let Err(error) = recovery_runtime
                .execute(pending.request().id(), message, pending.request().caller())
                .await
            {
                warn!(%error, "恢复请求执行失败");
            }
        }
    });

    let mut stdin = BufReader::new(input);
    loop {
        let line: Option<String> = read_line(&mut stdin)
            .await
            .map_err(|error| error.to_string())?;
        let Some(line) = line else {
            break;
        };
        let line: WorkerLine =
            serde_json::from_str(&line).map_err(|error| format!("无效 worker 请求帧: {error}"))?;
        match line {
            WorkerLine::Request {
                id,
                worker: claimed,
                message,
                caller,
            } => {
                claimed
                    .validate(&worker)
                    .map_err(|error| error.to_string())?;
                let runtime = Arc::clone(&runtime);
                let execution = Arc::clone(&execution);
                let out_tx = out_tx.clone();
                tasks.spawn(async move {
                    let _gate = execution.lock().await;
                    let result = runtime.execute(&id, message, caller).await;
                    let result = result.and_then(|payload| {
                        serde_json::to_value(payload).map_err(|e| e.to_string())
                    });
                    if let Err(error) = out_tx.send(WorkerLine::Response { id, result }).await {
                        warn!(%error, "worker 响应通道已关闭");
                    }
                });
            }
            WorkerLine::SubagentStarted { id, result } => {
                if let Some(tx) = subagent_pending
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove(&id)
                {
                    let _ = tx.send(result);
                }
            }
            WorkerLine::Shutdown { worker: claimed } => {
                claimed
                    .validate(&worker)
                    .map_err(|error| error.to_string())?;
                break;
            }
            WorkerLine::DeliverEvent { frame } => EventBus::try_publish_frame(frame),
            WorkerLine::BrokerHead { .. }
            | WorkerLine::BrokerData { .. }
            | WorkerLine::BrokerEnd { .. }
            | WorkerLine::BrokerError { .. } => {
                let Some((client, _, _)) = &broker else {
                    return Err("未启用 broker".to_string());
                };
                client.receive(line).await?;
            }
            WorkerLine::Ready { .. }
            | WorkerLine::Response { .. }
            | WorkerLine::Event { .. }
            | WorkerLine::StartSubagent { .. }
            | WorkerLine::CancelSubagent { .. }
            | WorkerLine::BrokerOpen { .. }
            | WorkerLine::BrokerCancel { .. } => {
                return Err("worker 收到方向错误的帧".to_string());
            }
        }
    }

    runtime.launcher.fail_all("worker 已停止");
    if let Some((client, stop, server)) = broker {
        client.cancel_all().await;
        stop.cancel();
        if let Err(error) = server.await {
            warn!(%error, "worker broker 任务异常退出");
        }
    }
    stop_events.cancel();
    while let Some(result) = tasks.join_next().await {
        if let Err(error) = result {
            warn!(%error, "worker 请求任务异常退出");
        }
    }
    drop(runtime);
    drop(out_tx);
    if let Err(error) = events.await {
        warn!(%error, "worker 事件任务异常退出");
    }
    if let Err(error) = writer.await {
        warn!(%error, "worker 输出任务异常退出");
    }
    Ok(())
}

async fn worker_events(
    root: &Arc<dyn Plugin>,
    worker: &WorkerId,
    homedir: &Path,
) -> Result<mpsc::Receiver<PluginFrame>, String> {
    let ctx = Arc::new(PluginSimpleRequest::new(None, None));
    crate::runtime::attach_vdfs_protected_roots(ctx.as_ref(), homedir);
    ctx.set(PATH, ROUTE_VDFS_ROOT.to_string());
    ctx.set(VDFS_CALLER, VdfsCaller::Manager);
    ctx.set_payload(json!({}))
        .map_err(|error| format!("构造 VDFS 根查询失败: {error}"))?;
    let payload = root
        .clone()
        .route(ctx)
        .await
        .map_err(|error| format!("查询 VDFS 根失败: {error}"))?;
    let root_addr = payload
        .get::<Value>()
        .map_err(|error| format!("读取 VDFS 根失败: {error}"))?
        .get("path")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| "VDFS 根查询未返回根地址".to_string())?
        .to_string();

    let address = session_address(&root_addr, worker.session_id())?;

    let ctx = Arc::new(PluginSimpleRequest::new(None, None));
    crate::runtime::attach_vdfs_protected_roots(ctx.as_ref(), homedir);
    ctx.set(PATH, ROUTE_VDFS_WATCH.to_string());
    ctx.set(SESSION_ID, worker.session_id().to_string());
    ctx.set(VDFS_CALLER, VdfsCaller::Manager);
    ctx.set_payload(json!({ "path": address }))
        .map_err(|error| format!("构造会话 watch 失败: {error}"))?;
    root.clone()
        .route(ctx)
        .await
        .map_err(|error| format!("订阅会话变更失败: {error}"))?;

    let ctx = Arc::new(PluginSimpleRequest::new(None, None));
    crate::runtime::attach_vdfs_protected_roots(ctx.as_ref(), homedir);
    ctx.set(PATH, ROUTE_EVENT_BUS_SUBSCRIBE.to_string());
    ctx.set(VDFS_CALLER, VdfsCaller::Manager);
    ctx.set_payload(json!({}))
        .map_err(|error| format!("构造事件订阅失败: {error}"))?;
    match root.clone().route(ctx).await {
        Ok(PluginPayload::Session(channel)) => Ok(channel.rx),
        Ok(_) => Err("worker 事件订阅未返回会话通道".to_string()),
        Err(error) => Err(format!("worker 事件订阅失败: {error}")),
    }
}

async fn forward_events(
    mut rx: mpsc::Receiver<PluginFrame>,
    tx: mpsc::Sender<WorkerLine>,
    stop: CancellationToken,
) {
    loop {
        tokio::select! {
            _ = stop.cancelled() => break,
            frame = rx.recv() => {
                let Some(frame) = frame else { break };
                let is_system = frame
                    .value()
                    .and_then(|value| value.get("data"))
                    .and_then(|bus| bus.get("kind"))
                    .and_then(Value::as_str)
                    == Some(EVENT_BUS_KIND_SYSTEM);
                if is_system || is_relayed(&frame) {
                    continue;
                }
                if let Err(error) = tokio::select! {
                    _ = stop.cancelled() => Err("worker 事件转发已停止".to_string()),
                    result = tx.send(WorkerLine::Event { frame }) => result.map_err(|_| "worker 输出通道已关闭".to_string()),
                } {
                    warn!(%error, "worker 事件转发失败");
                    break;
                }
            }
        }
    }
}

fn session_address(root_addr: &str, session_id: &str) -> Result<String, String> {
    super::id::validate_session_id(session_id).map_err(|error| error.to_string())?;
    Ok(format!(
        "{}/session/{session_id}",
        root_addr.trim_end_matches('/')
    ))
}

fn resolve_session_dir(
    homedir: &Path,
    session_dir: &Path,
    session_id: &str,
) -> Result<PathBuf, String> {
    let session_dir = std::fs::canonicalize(session_dir)
        .map_err(|error| format!("解析会话目录失败 {}: {error}", session_dir.display()))?;
    let session_root = homedir.join("session");
    if !session_dir.starts_with(&session_root) {
        return Err("会话目录不在系统会话根内".to_string());
    }
    let metadata: Value = serde_json::from_slice(
        &std::fs::read(session_dir.join("session.json"))
            .map_err(|error| format!("读取会话元数据失败: {error}"))?,
    )
    .map_err(|error| format!("解析会话元数据失败: {error}"))?;
    if metadata.get("id").and_then(Value::as_str) != Some(session_id) {
        return Err("会话目录与 worker 会话身份不一致".to_string());
    }
    Ok(session_dir)
}

#[cfg(test)]
#[path = "process.test.rs"]
mod tests;
