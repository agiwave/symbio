use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use symbio_platform::kernel::plugin::{
    Plugin, PluginFrame, PluginMessageWire, PluginPayload, PluginSimpleRequest,
};
use symbio_platform::sdk::event_bus::EventBus;
use symbio_platform::sdk::keys::{PATH, SESSION_ID, WORKDIR};
use symbio_platform::sdk::vdfs::{declared_addr_root, join_addr};
use symbio_plugin_api::routes::{ROUTE_VDFS_ACTION, ROUTE_VDFS_DELETE, ROUTE_VDFS_WRITE};
use symbio_plugin_api::{
    event_bus_build_envelope, PluginInvokeRequestExt, SymbioKey, VdfsCaller, EVENT_BUS_KIND_SYSTEM,
    EVENT_BUS_RESYNC_MARKER_TYPE, VDFS_CALLER,
};
use tokio::io::{split, AsyncRead, AsyncWrite, BufReader};
use tokio::process::Child;
use tokio::sync::{mpsc, oneshot, Mutex, RwLock as AsyncRwLock};
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use super::broker::BrokerRegistry;
use super::container::{self, BrokerTarget, WorkerConfig, WorkerMode};
use super::epoch::EpochRegistry;
use super::event_relay::{is_relayed, mark_relayed};
use super::id::validate_session_id;
use super::process::run_worker_with_io;
use super::protocol::{
    read_line, write_line, WorkerLine, CONTROL_CHANNEL_CAPACITY, MAX_LINE_BYTES,
};
use super::{
    BuildId, DurableInbox, InboxRequest, RequestId, WorkerBootstrap, WorkerError, WorkerId,
};

const WORKER_READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);
const WORKER_SHUTDOWN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// 会话读数口只读事实源，不碰 worker 的 WAL 写令牌；即使会话存在也留控制面。
const CONTROL_PLANE_SESSION_READS: &[&str] = &["session/stats", "session/changes"];

/// 只读 VDFS 投影永远留控制面：读方一旦派给 worker，就会让长驻读进程长期持有
/// WAL 写令牌，真正要写的进程反而拿到 `NotTheWriter`。
const CONTROL_PLANE_VDFS_READS: &[&str] = &[
    symbio_plugin_api::routes::ROUTE_VDFS_READ,
    symbio_plugin_api::routes::ROUTE_VDFS_LIST,
    symbio_plugin_api::routes::ROUTE_VDFS_STAT,
    symbio_plugin_api::routes::ROUTE_VDFS_TREE,
    symbio_plugin_api::routes::ROUTE_VDFS_SEARCH,
];

const CONTROL_PREFIXES: &[&str] = &[
    "home/",
    "event_bus/",
    "gateway/",
    "plugin_manager/",
    "sandbox/",
    "setting/",
    "ui_prefs/",
    "telegram/",
    "model/",
    "classify/",
    "compose/",
    "hook/",
    "skill/",
    "heartbeat/",
];

#[derive(Debug)]
enum Outgoing {
    Request {
        id: RequestId,
        message: PluginMessageWire,
    },
    Event {
        frame: PluginFrame,
    },
    Line(WorkerLine),
}

struct WorkerState {
    worker: WorkerId,
    /// host 进程内执行体与宿主共享事件总线；它的事件已经直接投递过，不再回灌。
    local: bool,
    sender: mpsc::Sender<Outgoing>,
    pending: Mutex<HashMap<RequestId, oneshot::Sender<Result<Value, String>>>>,
    resync_pending: AtomicBool,
    stop: CancellationToken,
    done: CancellationToken,
}

impl WorkerState {
    async fn fail_pending(&self, error: &str) {
        let mut pending = self.pending.lock().await;
        for (_, reply) in pending.drain() {
            let _ = reply.send(Err(error.to_string()));
        }
    }
}

async fn send_outgoing(worker: &WorkerState, outgoing: Outgoing) -> Result<(), String> {
    tokio::select! {
        _ = worker.stop.cancelled() => Err("worker 已停止".to_string()),
        result = worker.sender.send(outgoing) => result.map_err(|_| "worker 已退出".to_string()),
    }
}

fn relay_event(worker: &Arc<WorkerState>, frame: PluginFrame) {
    let kind = frame_kind(&frame);
    match worker.sender.try_send(Outgoing::Event { frame }) {
        Ok(()) => {}
        Err(mpsc::error::TrySendError::Closed(_)) => {
            warn!(worker = %worker.worker, "跨 worker 事件投递失败：通道已关闭");
        }
        Err(mpsc::error::TrySendError::Full(_)) => {
            if worker.resync_pending.swap(true, Ordering::SeqCst) {
                return;
            }
            let Some(kind) = kind else {
                worker.resync_pending.store(false, Ordering::SeqCst);
                warn!(worker = %worker.worker, "跨 worker 事件无法补送 resync：缺少频道");
                return;
            };
            warn!(worker = %worker.worker, "跨 worker 事件通道已满，补送 resync");
            let worker = Arc::clone(worker);
            tokio::spawn(async move {
                let marker = PluginFrame::data(event_bus_build_envelope(
                    &kind,
                    None,
                    json!({ "type": EVENT_BUS_RESYNC_MARKER_TYPE }),
                ));
                if let Err(error) = send_outgoing(&worker, Outgoing::Event { frame: marker }).await
                {
                    warn!(%error, worker = %worker.worker, "跨 worker resync 投递失败");
                }
                worker.resync_pending.store(false, Ordering::SeqCst);
            });
        }
    }
}

fn frame_kind(frame: &PluginFrame) -> Option<String> {
    frame
        .value()?
        .get("data")?
        .get("kind")?
        .as_str()
        .map(str::to_string)
}

struct SubagentStart {
    owner: WorkerId,
    cancel: CancellationToken,
    done: oneshot::Receiver<()>,
}

struct SupervisorInner {
    homedir: RwLock<PathBuf>,
    build_id: BuildId,
    config: WorkerConfig,
    workers: Mutex<HashMap<String, Arc<WorkerState>>>,
    subagents: Mutex<HashMap<RequestId, SubagentStart>>,
    subagent_lifecycle: Mutex<CancellationToken>,
    routing: AsyncRwLock<()>,
    spawn_lock: Mutex<()>,
    stopping: AtomicBool,
    serial: AtomicU64,
}

pub struct WorkerSupervisor {
    inner: Arc<SupervisorInner>,
}

impl WorkerSupervisor {
    pub fn new(homedir: PathBuf) -> Result<Self, WorkerError> {
        let config = WorkerConfig::from_env()?;
        Ok(Self {
            inner: Arc::new(SupervisorInner {
                homedir: RwLock::new(homedir),
                build_id: build_id()?,
                config,
                workers: Mutex::new(HashMap::new()),
                subagents: Mutex::new(HashMap::new()),
                subagent_lifecycle: Mutex::new(CancellationToken::new()),
                routing: AsyncRwLock::new(()),
                spawn_lock: Mutex::new(()),
                stopping: AtomicBool::new(false),
                serial: AtomicU64::new(0),
            }),
        })
    }

    pub fn session_for(&self, message: &PluginMessageWire) -> Result<Option<String>, WorkerError> {
        let path = message
            .metadata
            .get(PATH.name())
            .and_then(Value::as_str)
            .unwrap_or_default();
        if CONTROL_PREFIXES
            .iter()
            .any(|prefix| path.starts_with(prefix))
        {
            return Ok(None);
        }
        if CONTROL_PLANE_VDFS_READS.contains(&path) {
            return Ok(None);
        }
        if CONTROL_PLANE_SESSION_READS.contains(&path) {
            return Ok(None);
        }
        let Some(session_id) = session_for_message(&self.inner, message)? else {
            return Ok(None);
        };
        validate_session_id(session_id)?;
        Ok(Some(session_id.to_string()))
    }

    pub async fn invoke(
        &self,
        message: PluginMessageWire,
    ) -> Result<Option<PluginMessageWire>, String> {
        let routing = self.inner.routing.read().await;
        ensure_running(&self.inner)?;
        let Some(session_id) = self
            .session_for(&message)
            .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        let session_dir = find_session_dir(&self.session_root(), &session_id)
            .ok_or_else(|| format!("会话 {session_id} 目录不存在"))?;
        let worker = get_or_spawn_at(&self.inner, &session_id, session_dir.clone()).await?;
        let id = self.next_request_id();
        let reply_rx = deliver(&session_id, session_dir, &worker, id, message).await?;
        drop(routing);
        let result = reply_rx.await.map_err(|_| "worker 已退出".to_string())??;
        Ok(Some(PluginMessageWire {
            metadata: json!({}),
            payload: result,
        }))
    }

    pub async fn invoke_delete(
        &self,
        message: PluginMessageWire,
        root: &Arc<dyn Plugin>,
    ) -> Result<Option<PluginPayload>, String> {
        let route = message
            .metadata
            .get(PATH.name())
            .and_then(Value::as_str)
            .unwrap_or_default();
        if route != ROUTE_VDFS_DELETE || !session_node_in(&message.payload) {
            return Ok(None);
        }
        let _routing = self.inner.routing.write().await;
        ensure_running(&self.inner)?;
        let session_id = session_for_message(&self.inner, &message)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "删除请求缺少会话归属".to_string())?;
        self.stop_session_tree(session_id).await;
        let context = Arc::new(PluginSimpleRequest::from_parts(
            message.payload,
            message.metadata.as_object(),
        ));
        context.set(VDFS_CALLER, VdfsCaller::Manager);
        root.clone()
            .route(context)
            .await
            .map(Some)
            .map_err(|error| error.to_string())
    }

    pub async fn shutdown(&self) {
        self.inner.stopping.store(true, Ordering::SeqCst);
        let _routing = self.inner.routing.write().await;
        self.stop_workers().await;
    }

    pub async fn set_homedir(&self, homedir: PathBuf) {
        let _routing = self.inner.routing.write().await;
        self.stop_workers().await;
        *self
            .inner
            .homedir
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = homedir;
    }

    async fn stop_workers(&self) {
        let mut lifecycle = self.inner.subagent_lifecycle.lock().await;
        std::mem::replace(&mut *lifecycle, CancellationToken::new()).cancel();
        drop(lifecycle);
        let workers: Vec<_> = self
            .inner
            .workers
            .lock()
            .await
            .drain()
            .map(|(_, worker)| worker)
            .collect();
        stop_worker_states(workers).await;
        let starts: Vec<_> = self.inner.subagents.lock().await.drain().collect();
        for (_, start) in starts {
            start.cancel.cancel();
            if let Err(error) = start.done.await {
                warn!(%error, "子会话启动任务异常退出");
            }
        }
    }

    async fn stop_session_tree(&self, session_id: &str) {
        let Some(session_dir) = find_session_dir(&self.session_root(), session_id) else {
            return;
        };
        let ids: std::collections::HashSet<_> = session_dirs(&session_dir)
            .into_iter()
            .filter_map(|dir| session_id_in_file(&dir))
            .collect();
        let mut starts = self.inner.subagents.lock().await;
        let (cancelled, remaining): (Vec<_>, Vec<_>) = std::mem::take(&mut *starts)
            .into_iter()
            .partition(|(_, start)| ids.contains(start.owner.session_id()));
        *starts = remaining.into_iter().collect();
        drop(starts);
        for (_, start) in cancelled {
            start.cancel.cancel();
            if let Err(error) = start.done.await {
                warn!(%error, "子会话启动任务异常退出");
            }
        }
        let mut workers = self.inner.workers.lock().await;
        let stopped: Vec<_> = workers
            .extract_if(|id, _| ids.contains(id))
            .map(|(_, worker)| worker)
            .collect();
        drop(workers);
        stop_worker_states(stopped).await;
    }
}

async fn stop_worker_states(workers: Vec<Arc<WorkerState>>) {
    let drained = tokio::time::timeout(WORKER_SHUTDOWN_TIMEOUT, async {
        for worker in &workers {
            let shutdown = WorkerLine::Shutdown {
                worker: worker.worker.clone(),
            };
            if let Err(error) = send_outgoing(worker, Outgoing::Line(shutdown)).await {
                debug!(%error, worker = %worker.worker, "worker 退出通道已关闭");
            }
        }
        for worker in &workers {
            worker.done.cancelled().await;
        }
    })
    .await;
    if drained.is_err() {
        warn!("worker 正常退出超时，转为强制停止");
        for worker in &workers {
            worker.stop.cancel();
        }
        for worker in &workers {
            worker.fail_pending("worker 已停止").await;
        }
        for worker in workers {
            worker.done.cancelled().await;
        }
    }
}

fn session_dirs(dir: &Path) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        if session_id_in_file(&dir).is_some() {
            dirs.push(dir.clone());
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            }
        }
    }
    dirs
}

fn ensure_running(inner: &SupervisorInner) -> Result<(), String> {
    if inner.stopping.load(Ordering::SeqCst) {
        Err("运行时正在关闭".to_string())
    } else {
        Ok(())
    }
}

async fn cancel_subagent(
    inner: &SupervisorInner,
    owner: &WorkerId,
    id: &RequestId,
) -> Result<(), WorkerError> {
    let starts = inner.subagents.lock().await;
    match starts.get(id) {
        Some(start) if &start.owner == owner => {
            start.cancel.cancel();
            Ok(())
        }
        Some(_) => Err(WorkerError::Invalid("子会话取消发送者不匹配".to_string())),
        None => Ok(()),
    }
}

fn session_for_message<'a>(
    inner: &SupervisorInner,
    message: &'a PluginMessageWire,
) -> Result<Option<&'a str>, WorkerError> {
    let session_root = inner
        .homedir
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .join("session");
    let declared = message
        .metadata
        .get(SESSION_ID.name())
        .and_then(Value::as_str);
    let targeted = session_path_in(&message.payload);
    if let Some(session_id) = targeted {
        // metadata.session_id 也用作 native IPC 的事件通道名；显式 VDFS 路径才是资源归属。
        let creating = message.metadata.get(PATH.name()).and_then(Value::as_str)
            == Some(ROUTE_VDFS_WRITE)
            && message.payload.get("create").and_then(Value::as_bool) == Some(true)
            && session_node_path(session_id).as_deref()
                == message.payload.get("path").and_then(Value::as_str);
        if creating && find_session_dir(&session_root, session_id).is_none() {
            return Ok(None);
        }
        return find_session_dir(&session_root, session_id)
            .map(|_| Some(session_id))
            .ok_or_else(|| WorkerError::Invalid(format!("请求会话不存在: {session_id}")));
    }
    let route = message
        .metadata
        .get(PATH.name())
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !route.starts_with("session/") {
        return Ok(None);
    }
    let session_id =
        declared.ok_or_else(|| WorkerError::Invalid("请求缺少会话归属".to_string()))?;
    find_session_dir(&session_root, session_id)
        .map(|_| Some(session_id))
        .ok_or_else(|| WorkerError::Invalid(format!("请求会话不存在: {session_id}")))
}

fn session_node_path(session_id: &str) -> Option<String> {
    let root = declared_addr_root()?;
    Some(join_addr(&join_addr(root, "session"), session_id))
}

fn session_path_target(payload: &Value) -> Option<(&str, usize)> {
    let path = payload.get("path")?.as_str()?;
    if path.starts_with('/') {
        return None;
    }
    let segments: Vec<_> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let root_segments: Vec<_> = declared_addr_root()
        .unwrap_or_default()
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let index = if segments.first() == Some(&"session") {
        0
    } else if !root_segments.is_empty()
        && segments.len() > root_segments.len()
        && segments.starts_with(&root_segments)
        && segments[root_segments.len()] == "session"
    {
        root_segments.len()
    } else {
        return None;
    };
    let mut id = *segments.get(index + 1)?;
    let mut end = index + 1;
    if segments.get(end + 1) == Some(&"subsession") {
        id = *segments.get(end + 2)?;
        end += 2;
    }
    Some((id, end))
}

fn session_path_in(payload: &Value) -> Option<&str> {
    session_path_target(payload).map(|(id, _)| id)
}

fn session_node_in(payload: &Value) -> bool {
    let Some(path) = payload.get("path").and_then(Value::as_str) else {
        return false;
    };
    let count = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .count();
    session_path_target(payload).is_some_and(|(_, end)| end + 1 == count)
}

impl WorkerSupervisor {
    fn session_root(&self) -> PathBuf {
        self.inner
            .homedir
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .join("session")
    }

    fn next_request_id(&self) -> RequestId {
        let serial = self.inner.serial.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        RequestId::generated('r', &[std::process::id() as u64, nanos as u64, serial])
    }
}

struct DispatchCleanup {
    worker: Arc<WorkerState>,
    id: RequestId,
    inbox: Option<DurableInbox>,
    armed: bool,
}

impl DispatchCleanup {
    fn new(worker: Arc<WorkerState>, id: RequestId, inbox: DurableInbox) -> Self {
        Self {
            worker,
            id,
            inbox: Some(inbox),
            armed: true,
        }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for DispatchCleanup {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Some(inbox) = self.inbox.take() else {
            return;
        };
        // Drop 不能等待异步锁；取消只撤销 reply 槽，pending 记录交给同步 inbox 清理。
        let worker = Arc::clone(&self.worker);
        let id = self.id.clone();
        tokio::spawn(async move {
            worker.pending.lock().await.remove(&id);
            let _ = inbox.cancel_pending(&id);
        });
    }
}

async fn deliver(
    session_id: &str,
    session_dir: PathBuf,
    worker: &Arc<WorkerState>,
    id: RequestId,
    message: PluginMessageWire,
) -> Result<oneshot::Receiver<Result<Value, String>>, String> {
    let inbox = DurableInbox::new(session_id, session_dir).map_err(|error| error.to_string())?;
    inbox
        .enqueue(
            InboxRequest::new(
                id.as_str(),
                message.metadata.clone(),
                message.payload.clone(),
                message
                    .metadata
                    .get(WORKDIR.name())
                    .and_then(Value::as_str)
                    .map(str::to_string),
            )
            .map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
    let (reply_tx, reply_rx) = oneshot::channel();
    worker.pending.lock().await.insert(id.clone(), reply_tx);
    let mut cleanup = DispatchCleanup::new(Arc::clone(worker), id, inbox);
    send_outgoing(
        &cleanup.worker,
        Outgoing::Request {
            id: cleanup.id.clone(),
            message,
        },
    )
    .await?;
    cleanup.disarm();
    Ok(reply_rx)
}

async fn get_or_spawn_at(
    inner: &Arc<SupervisorInner>,
    session_id: &str,
    session_dir: PathBuf,
) -> Result<Arc<WorkerState>, String> {
    ensure_running(inner)?;
    if let Some(worker) = inner.workers.lock().await.get(session_id) {
        return Ok(Arc::clone(worker));
    }
    let _guard = inner.spawn_lock.lock().await;
    ensure_running(inner)?;
    if let Some(worker) = inner.workers.lock().await.get(session_id) {
        return Ok(Arc::clone(worker));
    }
    let registry = EpochRegistry::new(session_id, &session_dir).map_err(|e| e.to_string())?;
    let worker_id = registry
        .allocate(inner.build_id.clone())
        .map_err(|e| e.to_string())?;
    spawn_worker_erased(inner, &worker_id, session_id, session_dir).await
}

fn spawn_worker_erased<'a>(
    inner: &'a Arc<SupervisorInner>,
    worker: &'a WorkerId,
    map_key: &'a str,
    session_dir: PathBuf,
) -> Pin<Box<dyn Future<Output = Result<Arc<WorkerState>, String>> + Send + 'a>> {
    Box::pin(spawn_worker(inner, worker, map_key, session_dir))
}

async fn start_subagent(
    inner: &Arc<SupervisorInner>,
    parent: &WorkerId,
    child_session_id: &str,
    message: PluginMessageWire,
    cancel: &CancellationToken,
    dispatched: &AtomicBool,
) -> Result<(), String> {
    let lifecycle = inner.subagent_lifecycle.lock().await.clone();
    let routing = tokio::select! {
        _ = lifecycle.cancelled() => return Err("子会话启动已取消".to_string()),
        _ = cancel.cancelled() => return Err("子会话启动已取消".to_string()),
        routing = async { inner.routing.read().await } => routing,
    };
    ensure_running(inner)?;
    validate_session_id(child_session_id).map_err(|error| error.to_string())?;
    let session_root = inner
        .homedir
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .join("session");
    let parent_dir = find_session_dir(&session_root, parent.session_id())
        .ok_or_else(|| format!("父会话 {} 目录不存在", parent.session_id()))?;
    let registry =
        EpochRegistry::new(parent.session_id(), &parent_dir).map_err(|e| e.to_string())?;
    let current = registry
        .current()
        .map_err(|e| e.to_string())?
        .ok_or("父 worker 未登记")?;
    parent
        .validate(&current)
        .map_err(|error| error.to_string())?;

    let child_dir = find_session_dir(&session_root, child_session_id)
        .ok_or_else(|| format!("子会话 {child_session_id} 目录不存在"))?;
    let owner = session_owner(&child_dir)
        .ok_or_else(|| format!("子会话 {child_session_id} 元数据不可读"))?;
    if owner != parent.session_id() {
        return Err(format!(
            "子会话 {child_session_id} 不属于父会话 {}",
            parent.session_id()
        ));
    }
    let path = message
        .metadata
        .get(PATH.name())
        .and_then(Value::as_str)
        .unwrap_or_default();
    if path != ROUTE_VDFS_WRITE && path != ROUTE_VDFS_ACTION {
        return Err("子会话启动只允许 vdfs/write 或 vdfs/action".to_string());
    }
    if session_path_in(&message.payload) != Some(child_session_id) {
        return Err("子会话启动载荷的会话归属不一致".to_string());
    }
    if message
        .metadata
        .get(SESSION_ID.name())
        .and_then(Value::as_str)
        != Some(child_session_id)
    {
        return Err("子会话启动消息的会话归属不一致".to_string());
    }

    if cancel.is_cancelled() {
        return Err("子会话启动已取消".to_string());
    }
    let handle = get_or_spawn_at(inner, child_session_id, child_dir.clone()).await?;
    if cancel.is_cancelled() {
        return Err("子会话启动已取消".to_string());
    }
    let id = RequestId::generated(
        's',
        &[
            std::process::id() as u64,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64,
            inner.serial.fetch_add(1, Ordering::Relaxed),
        ],
    );
    let reply_rx = deliver(child_session_id, child_dir, &handle, id, message).await?;
    dispatched.store(true, Ordering::SeqCst);
    drop(routing);
    let _ = tokio::select! {
        _ = cancel.cancelled() => return Err("子会话启动已取消".to_string()),
        result = reply_rx => result.map_err(|_| "子 worker 已退出".to_string())?,
    };
    Ok(())
}

fn session_prefix_in(payload: &Value) -> Option<String> {
    let path = payload.get("path")?.as_str()?;
    let segments: Vec<_> = path
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let (_, end) = session_path_target(payload)?;
    Some(segments[..=end].join("/"))
}

async fn abort_started_subagent(inner: &SupervisorInner, child_session_id: &str, prefix: &str) {
    let Some(worker) = inner.workers.lock().await.get(child_session_id).cloned() else {
        return;
    };
    let session_root = inner
        .homedir
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .join("session");
    let Some(session_dir) = find_session_dir(&session_root, child_session_id) else {
        return;
    };
    let id = RequestId::generated(
        'a',
        &[
            std::process::id() as u64,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64,
            inner.serial.fetch_add(1, Ordering::Relaxed),
        ],
    );
    let message = PluginMessageWire {
        metadata: json!({ "path": ROUTE_VDFS_ACTION, "session_id": child_session_id }),
        payload: json!({ "path": prefix, "action": "abort" }),
    };
    let Ok(reply_rx) = deliver(child_session_id, session_dir, &worker, id, message).await else {
        return;
    };
    match tokio::time::timeout(WORKER_SHUTDOWN_TIMEOUT, reply_rx).await {
        Ok(Ok(Ok(_))) => {}
        Ok(Ok(Err(error))) => warn!(%error, %child_session_id, "子会话中止动作失败"),
        Ok(Err(_)) => warn!(%child_session_id, "子会话中止回复通道已关闭"),
        Err(_) => warn!(%child_session_id, "子会话中止动作超时"),
    }
}

fn find_session_dir(session_root: &Path, session_id: &str) -> Option<PathBuf> {
    validate_session_id(session_id).ok()?;
    let direct = session_root.join(session_id);
    if session_id_in_file(&direct).as_deref() == Some(session_id) {
        return Some(direct);
    }
    fn walk(dir: &Path, session_id: &str) -> Option<PathBuf> {
        let nested = dir.join("sessions");
        if !nested.is_dir() {
            return None;
        }
        let entries = std::fs::read_dir(&nested).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            if session_id_in_file(&path).as_deref() == Some(session_id) {
                return Some(path);
            }
            if let Some(found) = walk(&path, session_id) {
                return Some(found);
            }
        }
        None
    }
    let entries = std::fs::read_dir(session_root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = walk(&path, session_id) {
                return Some(found);
            }
        }
    }
    None
}

fn session_id_in_file(dir: &Path) -> Option<String> {
    let value: Value =
        serde_json::from_slice(&std::fs::read(dir.join("session.json")).ok()?).ok()?;
    value.get("id").and_then(Value::as_str).map(str::to_string)
}

fn session_owner(dir: &Path) -> Option<String> {
    let value: Value =
        serde_json::from_slice(&std::fs::read(dir.join("session.json")).ok()?).ok()?;
    value
        .pointer("/metadata/parent_session_id")
        .and_then(Value::as_str)
        .map(str::to_string)
}

async fn spawn_worker(
    inner: &Arc<SupervisorInner>,
    worker: &WorkerId,
    map_key: &str,
    session_dir: PathBuf,
) -> Result<Arc<WorkerState>, String> {
    let homedir = inner
        .homedir
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone();
    let (mut child, container_runtime, container_cleanup, targets, broker_token) =
        if inner.config.mode == WorkerMode::Container {
            let mut launch = container::prepare_container(
                &inner.config,
                worker.build_id(),
                worker,
                &homedir,
                &session_dir,
            )
            .await
            .map_err(|e| e.to_string())?;
            let cleanup = Some((inner.config.clone(), launch.name.clone()));
            let token = launch.token.clone();
            let targets = launch.targets.clone();
            let child = launch.spawn().await?;
            let runtime = launch.runtime;
            (Some(child), Some(runtime), cleanup, targets, token)
        } else {
            (None, None, None, HashMap::new(), String::new())
        };

    // D-555：host 落点是进程内执行体；只有 container 仍派生独立进程。
    // 两条路径共用 WorkerLine 帧协议、durable inbox、epoch 与 WAL 租约语义。
    let stdin: Box<dyn AsyncWrite + Unpin + Send>;
    let stdout: Box<dyn AsyncRead + Unpin + Send>;
    let mut executor = None;
    if let Some(ref mut child) = child {
        stdin = Box::new(child.stdin.take().ok_or("worker stdin 不可用")?);
        stdout = Box::new(child.stdout.take().ok_or("worker stdout 不可用")?);
    } else {
        let (control, unit) = tokio::io::duplex(MAX_LINE_BYTES);
        let (control_read, control_write) = split(control);
        let (unit_read, unit_write) = split(unit);
        let bootstrap = WorkerBootstrap {
            worker: worker.clone(),
            homedir: homedir.clone(),
            session_dir: session_dir.clone(),
        };
        let executor_worker = worker.clone();
        executor = Some(tokio::spawn(async move {
            if let Err(error) = run_worker_with_io(bootstrap, unit_read, unit_write).await {
                warn!(%error, worker = %executor_worker, "host 执行体退出");
            }
        }));
        stdin = Box::new(control_write);
        stdout = Box::new(control_read);
    }
    let child_id = child.as_ref().map(|child| child.id());

    let (sender, outgoing) = mpsc::channel::<Outgoing>(CONTROL_CHANNEL_CAPACITY);
    let stop = CancellationToken::new();
    let state = Arc::new(WorkerState {
        worker: worker.clone(),
        local: child_id.is_none(),
        sender,
        pending: Mutex::new(HashMap::new()),
        resync_pending: AtomicBool::new(false),
        stop: stop.clone(),
        done: CancellationToken::new(),
    });
    inner
        .workers
        .lock()
        .await
        .insert(map_key.to_string(), Arc::clone(&state));
    let (ready_tx, ready_rx) = oneshot::channel::<Result<(), String>>();

    let writer_worker = worker.clone();
    let writer_stop = stop.clone();
    let brokers = BrokerRegistry::new();
    let (broker_tx, mut broker_rx) = mpsc::channel::<WorkerLine>(CONTROL_CHANNEL_CAPACITY);
    tokio::spawn(async move {
        let mut stdin = stdin;
        let mut outgoing = outgoing;
        loop {
            let out = tokio::select! {
                _ = writer_stop.cancelled() => break,
                out = outgoing.recv() => match out {
                    Some(out) => out,
                    None => break,
                },
                frame = broker_rx.recv() => {
                    let Some(line) = frame else { break };
                    if let Err(error) = write_line(&mut stdin, &line).await {
                        warn!(%error, "worker broker stdin 写入失败");
                        writer_stop.cancel();
                        break;
                    }
                    continue;
                },
            };
            let line = match out {
                Outgoing::Request { id, message } => WorkerLine::Request {
                    id,
                    worker: writer_worker.clone(),
                    message,
                },
                Outgoing::Event { frame } => WorkerLine::DeliverEvent {
                    frame: mark_relayed(frame),
                },
                Outgoing::Line(line) => line,
            };
            let shutdown = matches!(&line, WorkerLine::Shutdown { .. });
            if let Err(error) = write_line(&mut stdin, &line).await {
                warn!(%error, "worker stdin 写入失败");
                writer_stop.cancel();
                break;
            }
            if shutdown {
                break;
            }
        }
    });

    let reader_stop = stop.clone();
    let reader = WorkerReader {
        inner: Arc::clone(inner),
        map_key: map_key.to_string(),
        expected: worker.clone(),
        done: state.done.clone(),
        state: state.clone(),
        child,
        stdout,
        executor,
        ready: ready_tx,
        targets,
        broker_token,
        broker_tx,
        brokers,
        container_cleanup,
        container_runtime,
    };
    tokio::spawn(async move { read_worker(reader, reader_stop).await });

    let ready = tokio::time::timeout(WORKER_READY_TIMEOUT, ready_rx).await;
    let error = match ready {
        Ok(Ok(Ok(()))) => {
            match child_id {
                Some(pid) => info!(worker = %worker, pid, "worker 已就绪"),
                None => info!(worker = %worker, "会话执行体已就绪（进程内）"),
            }
            return Ok(state);
        }
        Ok(Ok(Err(error))) => error,
        Ok(Err(_)) => "worker 启动失败".to_string(),
        Err(_) => "worker 启动超时".to_string(),
    };
    let mut workers = inner.workers.lock().await;
    if workers
        .get(map_key)
        .is_some_and(|current| Arc::ptr_eq(current, &state))
    {
        workers.remove(map_key);
    }
    drop(workers);
    stop.cancel();
    state.fail_pending(&error).await;
    if tokio::time::timeout(std::time::Duration::from_secs(5), state.done.cancelled())
        .await
        .is_err()
    {
        warn!(worker = %worker, "worker 启动失败后仍未退出");
    }
    Err(error)
}

struct WorkerReader {
    inner: Arc<SupervisorInner>,
    map_key: String,
    expected: WorkerId,
    done: CancellationToken,
    state: Arc<WorkerState>,
    child: Option<Child>,
    stdout: Box<dyn AsyncRead + Unpin + Send>,
    executor: Option<tokio::task::JoinHandle<()>>,
    ready: oneshot::Sender<Result<(), String>>,
    targets: HashMap<String, BrokerTarget>,
    broker_token: String,
    broker_tx: mpsc::Sender<WorkerLine>,
    brokers: BrokerRegistry,
    container_cleanup: Option<(WorkerConfig, String)>,
    container_runtime: Option<tempfile::TempDir>,
}

async fn read_worker(reader: WorkerReader, stop: CancellationToken) {
    let WorkerReader {
        inner,
        map_key,
        expected,
        done,
        state,
        child,
        stdout,
        executor,
        ready,
        targets,
        broker_token,
        broker_tx,
        brokers,
        container_cleanup,
        container_runtime,
    } = reader;
    let mut ready = Some(ready);
    let mut reader = BufReader::new(stdout);
    let mut failed = None;
    loop {
        tokio::select! {
            _ = stop.cancelled() => break,
            line = read_line(&mut reader) => {
                let line: Result<Option<String>, WorkerError> = line;
                let line = match line {
                    Ok(Some(line)) => line,
                    Ok(None) => break,
                    Err(error) => {
                        failed = Some(error.to_string());
                        break;
                    }
                };
                let Ok(frame) = serde_json::from_str::<WorkerLine>(&line) else {
                    failed = Some("worker 发出无效帧".to_string());
                    break;
                };
                match frame {
                    WorkerLine::Ready { worker } => {
                        if let Err(error) = worker.validate(&expected) {
                            failed = Some(error.to_string());
                            break;
                        }
                        if let Some(tx) = ready.take() {
                            let _ = tx.send(Ok(()));
                        } else {
                            failed = Some("worker 重复发送 Ready".to_string());
                            break;
                        }
                    }
                    WorkerLine::Response { id, result } => {
                        let reply = state.pending.lock().await.remove(&id);
                        if let Some(tx) = reply {
                            let _ = tx.send(result);
                        }
                    }
                    WorkerLine::Event { frame } => {
                        route_worker_event(&inner, &state, &expected, frame).await;
                    }
                    WorkerLine::StartSubagent { id, child_session_id, message } => {
                        if let Err(error) = ensure_running(&inner) {
                            if let Err(error) = send_outgoing(
                                &state,
                                Outgoing::Line(WorkerLine::SubagentStarted { id, result: Err(error) }),
                            )
                            .await
                            {
                                warn!(%error, "子会话启动拒绝回复投递失败");
                            }
                            continue;
                        }
                        let cancel = CancellationToken::new();
                        let dispatched = Arc::new(AtomicBool::new(false));
                        let (done_tx, done_rx) = oneshot::channel();
                        let parent = expected.clone();
                        inner.subagents.lock().await.insert(id.clone(), SubagentStart {
                            owner: parent.clone(),
                            cancel: cancel.clone(),
                            done: done_rx,
                        });
                        let start_inner = Arc::clone(&inner);
                        let parent_state = Arc::clone(&state);
                        let task_id = id.clone();
                        let session_prefix = session_prefix_in(&message.payload);
                        tokio::spawn(async move {
                            let result = start_subagent(
                                &start_inner,
                                &parent,
                                &child_session_id,
                                message,
                                &cancel,
                                &dispatched,
                            )
                            .await;
                            if result.is_err()
                                && cancel.is_cancelled()
                                && dispatched.load(Ordering::SeqCst)
                            {
                                if let Some(prefix) = session_prefix {
                                    abort_started_subagent(&start_inner, &child_session_id, &prefix).await;
                                }
                            }
                            start_inner.subagents.lock().await.remove(&task_id);
                            let _ = done_tx.send(());
                            if let Err(error) = send_outgoing(
                                &parent_state,
                                Outgoing::Line(WorkerLine::SubagentStarted { id, result }),
                            )
                            .await
                            {
                                warn!(%error, "子会话启动回复投递失败");
                            }
                        });
                    }
                    WorkerLine::BrokerOpen { .. } | WorkerLine::BrokerCancel { .. } => {
                        if let Err(error) = brokers.handle(
                            frame,
                            &expected,
                            &targets,
                            broker_tx.clone(),
                            inner.config.max_broker_streams,
                            &broker_token,
                        ).await {
                            failed = Some(error);
                            break;
                        }
                    }
                    WorkerLine::CancelSubagent { id, worker } => {
                        let result = match worker.validate(&expected) {
                            Ok(()) => cancel_subagent(&inner, &expected, &id).await,
                            Err(error) => Err(error),
                        };
                        if let Err(error) = result {
                            failed = Some(error.to_string());
                            break;
                        }
                    }
                    WorkerLine::Request { .. }
                    | WorkerLine::SubagentStarted { .. }
                    | WorkerLine::DeliverEvent { .. }
                    | WorkerLine::Shutdown { .. }
                    | WorkerLine::BrokerHead { .. }
                    | WorkerLine::BrokerData { .. }
                    | WorkerLine::BrokerEnd { .. }
                    | WorkerLine::BrokerError { .. } => {
                        failed = Some("worker 发出方向错误的帧".to_string());
                        break;
                    }
                }
            }
        }
    }
    if let Some(tx) = ready {
        let _ = tx.send(Err(failed
            .clone()
            .unwrap_or_else(|| "worker 提前退出".to_string())));
    }
    let error = failed.unwrap_or_else(|| "worker 已退出".to_string());
    state.fail_pending(&error).await;
    let mut workers = inner.workers.lock().await;
    if workers
        .get(&map_key)
        .is_some_and(|current| Arc::ptr_eq(current, &state))
    {
        workers.remove(&map_key);
    }
    drop(workers);
    stop.cancel();
    brokers.stop().await;
    drop(broker_tx);
    if let Some(mut child) = child {
        if let Err(error) = child.kill().await {
            warn!(%error, "停止 worker 进程失败");
        }
    }
    if let Some(handle) = executor {
        if !handle.is_finished() {
            handle.abort();
        }
        if let Err(error) = handle.await {
            warn!(%error, "host 执行体任务异常退出");
        }
    }
    if let Some((config, name)) = container_cleanup {
        if let Err(error) = container::remove_container(&config, &name).await {
            warn!(%error, container = %name, "清理容器失败，可能仍有挂载与 WAL 锁");
        }
    }
    drop(container_runtime);
    info!(worker = %expected, "worker 已退出");
    done.cancel();
}

async fn route_worker_event(
    inner: &SupervisorInner,
    state: &WorkerState,
    expected: &WorkerId,
    frame: PluginFrame,
) {
    // host 执行体与宿主共用事件总线：插件发布时前端已经收到这一帧，
    // 监督者再投一次会让流式正文叠字。容器 worker 仍需监督者跨进程回灌。
    if !state.local {
        publish_frame(frame.clone());
    }
    let workers: Vec<_> = inner
        .workers
        .lock()
        .await
        .iter()
        .filter(|(id, worker)| expected.session_id() != **id && !worker.local)
        .map(|(id, worker)| (id.clone(), Arc::clone(worker)))
        .collect();
    for (_, worker) in workers {
        relay_event(&worker, frame.clone());
    }
}

fn publish_frame(frame: PluginFrame) {
    if is_relayed(&frame) {
        return;
    }
    let is_system = frame
        .value()
        .and_then(|value| value.get("data"))
        .and_then(|bus| bus.get("kind"))
        .and_then(Value::as_str)
        == Some(EVENT_BUS_KIND_SYSTEM);
    if !is_system {
        EventBus::try_publish_frame(frame);
    }
}

fn build_id() -> Result<BuildId, WorkerError> {
    let executable = std::env::current_exe().map_err(|error| WorkerError::Io {
        op: "读取程序路径",
        path: PathBuf::from("<current_exe>"),
        kind: error.kind(),
        message: error.to_string(),
    })?;
    let bytes = std::fs::read(&executable).map_err(|error| WorkerError::Io {
        op: "计算构建指纹",
        path: executable,
        kind: error.kind(),
        message: error.to_string(),
    })?;
    let digest = Sha256::digest(bytes);
    BuildId::parse(format!("{digest:x}"))
}

#[cfg(test)]
#[path = "supervisor.test.rs"]
mod tests;
