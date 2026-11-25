mod broker;
mod broker_shim;
mod container;
mod entry;
mod epoch;
mod id;
mod inbox;
mod process;
mod protocol;
mod recovery;
mod supervisor;

pub use entry::run_worker_command;
pub use epoch::EpochRegistry;
pub use id::{BuildId, Epoch, Fenced, RequestId, WorkerId};
pub use inbox::{
    DurableInbox, InboxItem, InboxRequest, InterruptedItem, ReconciliationOutcome, RecoverySnapshot,
};
pub use process::{run_worker, WorkerBootstrap};
// protocol lines are implementation details; tests import the module directly.
pub use recovery::{RecoveryReport, WorkerReady, WorkerRecovery, WorkerStartup};
pub(crate) use supervisor::WorkerSupervisor;

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug)]
pub enum WorkerError {
    Invalid(String),
    Busy,
    Io {
        op: &'static str,
        path: PathBuf,
        kind: std::io::ErrorKind,
        message: String,
    },
    Corrupt {
        path: PathBuf,
        message: String,
    },
    RequestConflict(RequestId),
    NotClaimed {
        worker: WorkerId,
        request: RequestId,
    },
    StaleWorker {
        expected: WorkerId,
        actual: WorkerId,
    },
    RecoveryRequired {
        session_id: String,
        interrupted: Vec<InterruptedItem>,
    },
}

impl fmt::Display for WorkerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Invalid(message) => write!(f, "worker 协议无效: {message}"),
            Self::Busy => write!(f, "worker 资源已被其他持有者占用"),
            Self::Io {
                op,
                path,
                kind: _,
                message,
            } => {
                write!(f, "worker {op} 失败（{}）: {message}", path.display())
            }
            Self::Corrupt { path, message } => {
                write!(f, "worker 持久记录损坏（{}）: {message}", path.display())
            }
            Self::RequestConflict(id) => write!(f, "请求 id 已用于不同内容: {id}"),
            Self::NotClaimed { worker, request } => {
                write!(f, "worker {worker} 没有认领请求 {request}")
            }
            Self::StaleWorker { expected, actual } => {
                write!(f, "拒绝迟到 worker 帧: 期望 {expected}，实际 {actual}")
            }
            Self::RecoveryRequired {
                session_id,
                interrupted,
            } => write!(
                f,
                "会话 {session_id} 有 {} 条已认领但未完成的请求，必须先核对",
                interrupted.len()
            ),
        }
    }
}

impl std::error::Error for WorkerError {}

pub(super) fn io_error(op: &'static str, path: &Path, error: std::io::Error) -> WorkerError {
    WorkerError::Io {
        op,
        path: path.to_path_buf(),
        kind: error.kind(),
        message: error.to_string(),
    }
}

pub(super) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, WorkerError> {
    let bytes = std::fs::read(path).map_err(|error| io_error("读取", path, error))?;
    serde_json::from_slice(&bytes).map_err(|error| WorkerError::Corrupt {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

pub(super) fn read_optional_json<T: DeserializeOwned>(
    path: &Path,
) -> Result<Option<T>, WorkerError> {
    match std::fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|error| WorkerError::Corrupt {
                    path: path.to_path_buf(),
                    message: error.to_string(),
                })
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error("读取", path, error)),
    }
}

static STAGE_SERIAL: AtomicU64 = AtomicU64::new(0);

pub(super) fn create_dir_durable(path: &Path) -> Result<(), WorkerError> {
    let mut missing = Vec::new();
    let mut current = Some(path);
    while let Some(directory) = current.filter(|directory| !directory.exists()) {
        missing.push(directory.to_path_buf());
        current = directory.parent();
    }
    std::fs::create_dir_all(path).map_err(|error| io_error("创建目录", path, error))?;
    for directory in missing.into_iter().rev() {
        sync_parent(&directory)?;
        if let Some(parent) = directory.parent() {
            sync_parent(parent)?;
        }
    }
    Ok(())
}

pub(super) fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), WorkerError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    create_dir_durable(parent)?;
    let bytes =
        serde_json::to_vec(value).map_err(|error| WorkerError::Invalid(error.to_string()))?;
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let serial = STAGE_SERIAL.fetch_add(1, Ordering::Relaxed);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let staged = parent.join(format!(
        ".{name}.{}.{nonce}.{serial}.staged",
        std::process::id()
    ));
    let result = (|| -> std::io::Result<()> {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staged)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&staged, path)
    })();
    if let Err(error) = result {
        let _ = std::fs::remove_file(&staged);
        return Err(io_error("原子写入", path, error));
    }
    sync_parent(parent)?;
    Ok(())
}

pub(super) fn rename_durable(source: &Path, target: &Path) -> Result<(), WorkerError> {
    let source_parent = source.parent().unwrap_or_else(|| Path::new("."));
    let target_parent = target.parent().unwrap_or_else(|| Path::new("."));
    create_dir_durable(target_parent)?;
    std::fs::rename(source, target).map_err(|error| io_error("迁移记录", source, error))?;
    sync_parent(source_parent)?;
    sync_parent(target_parent)
}

pub(super) fn remove_durable(path: &Path) -> Result<(), WorkerError> {
    std::fs::remove_file(path).map_err(|error| io_error("删除记录", path, error))?;
    sync_parent(path.parent().unwrap_or_else(|| Path::new(".")))
}

pub(super) fn sync_parent(parent: &Path) -> Result<(), WorkerError> {
    #[cfg(unix)]
    {
        let directory = File::open(parent).map_err(|error| io_error("打开目录", parent, error))?;
        directory
            .sync_all()
            .map_err(|error| io_error("同步目录", parent, error))
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_BACKUP_SEMANTICS;
        let directory = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(parent)
            .map_err(|error| io_error("打开目录", parent, error))?;
        directory
            .sync_all()
            .map_err(|error| io_error("同步目录", parent, error))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = parent;
        Ok(())
    }
}

pub(super) fn exclusive_lock(path: &Path) -> Result<File, WorkerError> {
    if let Some(parent) = path.parent() {
        create_dir_durable(parent)?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|error| io_error("打开锁", path, error))?;
    file.lock().map_err(|_| WorkerError::Busy).map(|()| file)
}

pub(super) fn try_exclusive_lock(path: &Path) -> Result<File, WorkerError> {
    if let Some(parent) = path.parent() {
        create_dir_durable(parent)?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|error| io_error("打开锁", path, error))?;
    file.try_lock()
        .map_err(|_| WorkerError::Busy)
        .map(|()| file)
}

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(u64::MAX as u128) as u64
}

#[cfg(test)]
#[path = "mod.test.rs"]
mod tests;
