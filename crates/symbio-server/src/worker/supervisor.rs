use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use symbio::symbio_kernel::plugin::{PluginFrame, PluginMessageWire};
use symbio::symbio_plugin_api::SymbioKey;
use symbio::symbio_plugin_api::EVENT_BUS_KIND_SYSTEM;
use symbio::symbio_sdk::event_bus::EventBus;
use symbio::symbio_sdk::keys::{PATH, SESSION_ID, WORKDIR};
use tokio::io::BufReader;
use tokio::process::{Child, Command};
use tokio::sync::{mpsc, oneshot, Mutex};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use super::epoch::EpochRegistry;
use super::id::validate_session_id;
use super::protocol::{read_line, write_line, WorkerLine};
use super::{BuildId, DurableInbox, InboxRequest, RequestId, WorkerError, WorkerId};

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
struct Outgoing {
    id: RequestId,
    message: PluginMessageWire,
}

struct WorkerState {
    sender: mpsc::Sender<Outgoing>,
    pending: Mutex<HashMap<RequestId, oneshot::Sender<Result<Value, String>>>>,
    stop: CancellationToken,
}

impl WorkerState {
    fn fail_pending(&self, error: &str) {
        let Ok(mut pending) = self.pending.try_lock() else {
            return;
        };
        for (_, reply) in pending.drain() {
            let _ = reply.send(Err(error.to_string()));
        }
    }
}

pub struct WorkerSupervisor {
    homedir: std::sync::RwLock<PathBuf>,
    build_id: BuildId,
    workers: Mutex<HashMap<String, Arc<WorkerState>>>,
    spawn_lock: tokio::sync::Mutex<()>,
    serial: AtomicU64,
}

impl WorkerSupervisor {
    pub fn new(homedir: PathBuf) -> Result<Self, WorkerError> {
        let build_id = build_id()?;
        Ok(Self {
            homedir: std::sync::RwLock::new(homedir),
            build_id,
            workers: Mutex::new(HashMap::new()),
            spawn_lock: tokio::sync::Mutex::new(()),
            serial: AtomicU64::new(0),
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
        let Some(session_id) = self.session_for_message(message) else {
            return Ok(None);
        };
        validate_session_id(session_id)?;
        let dir = self.session_dir(session_id);
        Ok(if dir.is_dir() {
            Some(session_id.to_string())
        } else {
            None
        })
    }

    fn session_for_message<'a>(&self, message: &'a PluginMessageWire) -> Option<&'a str> {
        if let Some(session_id) = message
            .metadata
            .get(SESSION_ID.name())
            .and_then(Value::as_str)
            .filter(|id| self.session_dir(id).is_dir())
        {
            return Some(session_id);
        }
        let path = message.payload.get("path")?.as_str()?;
        let mut segments = path.split('/');
        while let Some(segment) = segments.next() {
            if segment == "session" {
                return segments.next().filter(|id| self.session_dir(id).is_dir());
            }
        }
        None
    }

    pub async fn invoke(
        &self,
        message: PluginMessageWire,
    ) -> Result<Option<PluginMessageWire>, String> {
        let Some(session_id) = self
            .session_for(&message)
            .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        let worker = self.get_or_spawn(&session_id).await?;
        let id = self.next_request_id();
        let workdir = message
            .metadata
            .get(WORKDIR.name())
            .and_then(Value::as_str)
            .map(str::to_string);
        let params = message.payload.clone();
        let inbox_request =
            InboxRequest::new(id.as_str(), message.metadata.clone(), params, workdir)
                .map_err(|error| error.to_string())?;
        let inbox = DurableInbox::new(&session_id, self.session_dir(&session_id))
            .map_err(|error| error.to_string())?;
        inbox
            .enqueue(inbox_request)
            .map_err(|error| error.to_string())?;
        let (reply_tx, reply_rx) = oneshot::channel();
        {
            let mut pending = worker.pending.lock().await;
            pending.insert(id.clone(), reply_tx);
        }
        if worker
            .sender
            .send(Outgoing {
                id: id.clone(),
                message,
            })
            .await
            .is_err()
        {
            return Err("worker 已退出".to_string());
        }
        let result = reply_rx.await.map_err(|_| "worker 已退出".to_string())??;
        Ok(Some(PluginMessageWire {
            metadata: json!({}),
            payload: result,
        }))
    }

    pub async fn shutdown(&self) {
        let workers: Vec<_> = self.workers.lock().await.drain().collect();
        for (_, worker) in workers {
            worker.stop.cancel();
            worker.fail_pending("worker 已停止");
        }
    }

    async fn get_or_spawn(&self, session_id: &str) -> Result<Arc<WorkerState>, String> {
        if let Some(worker) = self.workers.lock().await.get(session_id) {
            return Ok(Arc::clone(worker));
        }
        let _guard = self.spawn_lock.lock().await;
        if let Some(worker) = self.workers.lock().await.get(session_id) {
            return Ok(Arc::clone(worker));
        }
        let session_dir = self.session_dir(session_id);
        let registry = EpochRegistry::new(session_id, &session_dir).map_err(|e| e.to_string())?;
        let worker_id = registry
            .allocate(self.build_id.clone())
            .map_err(|e| e.to_string())?;
        let state = self.spawn(&worker_id).await?;
        self.workers
            .lock()
            .await
            .insert(session_id.to_string(), Arc::clone(&state));
        Ok(state)
    }

    async fn spawn(&self, worker: &WorkerId) -> Result<Arc<WorkerState>, String> {
        let executable = std::env::current_exe().map_err(|e| e.to_string())?;
        let mut command = Command::new(executable);
        command
            .arg("__worker")
            .arg("--session")
            .arg(worker.session_id())
            .arg("--epoch")
            .arg(worker.epoch().value().to_string())
            .arg("--build")
            .arg(worker.build_id().as_str())
            .arg("--homedir")
            .arg(
                self.homedir
                    .read()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .clone(),
            )
            .env("SYMBIO_PROCESS_ROLE", "worker")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|e| e.to_string())?;
        let stdin = child.stdin.take().ok_or("worker stdin 不可用")?;
        let stdout = child.stdout.take().ok_or("worker stdout 不可用")?;

        let (sender, outgoing) = mpsc::channel::<Outgoing>(64);
        let stop = CancellationToken::new();
        let state = Arc::new(WorkerState {
            sender,
            pending: Mutex::new(HashMap::new()),
            stop: stop.clone(),
        });
        let (ready_tx, ready_rx) = oneshot::channel::<Result<(), String>>();

        let writer_worker = worker.clone();
        tokio::spawn(async move {
            let mut stdin = stdin;
            let mut outgoing = outgoing;
            while let Some(out) = outgoing.recv().await {
                let line = WorkerLine::Request {
                    id: out.id,
                    worker: writer_worker.clone(),
                    message: out.message,
                };
                if let Err(_error) = write_line(&mut stdin, &line).await {
                    break;
                }
            }
        });

        let reader_state = Arc::clone(&state);
        let reader_stop = stop.clone();
        tokio::spawn(async move {
            read_worker(child, stdout, reader_state, ready_tx, reader_stop).await;
        });

        tokio::time::timeout(std::time::Duration::from_secs(60), ready_rx)
            .await
            .map_err(|_| "worker 启动超时".to_string())?
            .map_err(|_| "worker 启动失败".to_string())??;
        Ok(state)
    }

    fn session_dir(&self, session_id: &str) -> PathBuf {
        self.homedir
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .join("session")
            .join(session_id)
    }

    pub async fn set_homedir(&self, homedir: PathBuf) {
        *self
            .homedir
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = homedir;
        self.shutdown().await;
    }

    fn next_request_id(&self) -> RequestId {
        let serial = self.serial.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        RequestId::generated('r', &[std::process::id() as u64, nanos as u64, serial])
    }
}

async fn read_worker(
    mut child: Child,
    stdout: tokio::process::ChildStdout,
    state: Arc<WorkerState>,
    ready: oneshot::Sender<Result<(), String>>,
    stop: CancellationToken,
) {
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
                    WorkerLine::Ready { .. } => {
                        if let Some(tx) = ready.take() {
                            let _ = tx.send(Ok(()));
                        }
                    }
                    WorkerLine::Response { id, result } => {
                        let reply = state.pending.lock().await.remove(&id);
                        if let Some(tx) = reply {
                            let _ = tx.send(result);
                        }
                    }
                    WorkerLine::Event { frame } => publish_frame(frame),
                    _ => {}
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
    state.fail_pending(&error);
    if let Err(error) = child.kill().await {
        warn!(%error, "停止 worker 进程失败");
    }
}

fn publish_frame(frame: PluginFrame) {
    let Some(value) = frame.value() else {
        return;
    };
    let Some(bus) = value.get("data") else {
        return;
    };
    if bus.get("kind").and_then(Value::as_str) == Some(EVENT_BUS_KIND_SYSTEM) {
        return;
    }
    let kind = bus.get("kind").and_then(Value::as_str);
    let Some(kind) = kind else {
        return;
    };
    let session_id = bus.get("session_id").and_then(Value::as_str);
    let data = bus.get("data").cloned().unwrap_or(Value::Null);
    EventBus::try_publish(kind, session_id, data);
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
