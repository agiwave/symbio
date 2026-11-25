use std::collections::BTreeMap;
use std::fs::{self, File};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::epoch::EpochRegistry;
use super::id::{validate_session_id, BuildId, Epoch, RequestId, WorkerId};
use super::{
    create_dir_durable, exclusive_lock, io_error, now_ms, read_json, read_optional_json,
    remove_durable, rename_durable, sync_parent, write_json_atomic, WorkerError,
};

const PENDING: &str = "pending";
const CLAIMED: &str = "claimed";
const COMPLETED: &str = "completed";
const STATE_LOCK: &str = "state.lock";
const SEQUENCE_FILE: &str = "sequence.json";
const RECORD_VERSION: u8 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InboxRequest {
    id: RequestId,
    message: Value,
    params: Value,
    workdir: Option<String>,
}

impl InboxRequest {
    pub fn new(
        id: impl Into<String>,
        message: Value,
        params: Value,
        workdir: Option<String>,
    ) -> Result<Self, WorkerError> {
        Ok(Self {
            id: RequestId::parse(id)?,
            message,
            params,
            workdir,
        })
    }

    pub fn id(&self) -> &RequestId {
        &self.id
    }

    pub fn message(&self) -> &Value {
        &self.message
    }

    pub fn params(&self) -> &Value {
        &self.params
    }

    pub fn workdir(&self) -> Option<&str> {
        self.workdir.as_deref()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InboxItem {
    request: InboxRequest,
    accepted_at_ms: u64,
    accepted_by_pid: u32,
    accept_seq: u64,
}

impl InboxItem {
    pub fn id(&self) -> &RequestId {
        self.request.id()
    }

    pub fn request(&self) -> &InboxRequest {
        &self.request
    }

    pub fn accepted_at_ms(&self) -> u64 {
        self.accepted_at_ms
    }

    pub fn accepted_by_pid(&self) -> u32 {
        self.accepted_by_pid
    }

    pub fn accept_seq(&self) -> u64 {
        self.accept_seq
    }

    fn order_key(&self) -> (u64, &str) {
        (self.accept_seq, self.request.id.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReconciliationOutcome {
    Confirmed,
    Abandoned,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Completion {
    outcome: CompletionOutcome,
    at_ms: u64,
    note: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum CompletionOutcome {
    Executed,
    Reconciled(ReconciliationOutcome),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct InboxRecord {
    version: u8,
    item: InboxItem,
    #[serde(default)]
    completion: Option<Completion>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InterruptedItem {
    worker: WorkerId,
    item: InboxItem,
}

impl InterruptedItem {
    pub fn worker(&self) -> &WorkerId {
        &self.worker
    }

    pub fn item(&self) -> &InboxItem {
        &self.item
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecoverySnapshot {
    pub pending: Vec<InboxItem>,
    pub interrupted: Vec<InterruptedItem>,
}

#[derive(Debug, Serialize, Deserialize)]
struct SequenceRecord {
    version: u8,
    next: u64,
}

#[derive(Debug)]
struct LocatedRecord {
    state: &'static str,
    worker: Option<WorkerId>,
    path: PathBuf,
    item: InboxItem,
}

pub struct DurableInbox {
    session_id: String,
    root: PathBuf,
    epochs: EpochRegistry,
}

impl DurableInbox {
    pub fn new(
        session_id: impl Into<String>,
        session_dir: impl Into<PathBuf>,
    ) -> Result<Self, WorkerError> {
        let session_id = session_id.into();
        validate_session_id(&session_id)?;
        let session_dir = session_dir.into();
        let inbox = Self {
            epochs: EpochRegistry::new(&session_id, &session_dir)?,
            session_id,
            root: session_dir.join("worker").join("inbox"),
        };
        inbox.ensure_dirs()?;
        Ok(inbox)
    }

    pub fn enqueue(&self, request: InboxRequest) -> Result<InboxItem, WorkerError> {
        let _state_lock = self.lock()?;
        if let Some(existing) = self.find_accepted(request.id())? {
            if existing.request == request {
                return Ok(existing);
            }
            return Err(WorkerError::RequestConflict(request.id().clone()));
        }
        let sequence = self.next_accept_seq()?;
        let item = InboxItem {
            request,
            accepted_at_ms: now_ms(),
            accepted_by_pid: std::process::id(),
            accept_seq: sequence,
        };
        write_json_atomic(
            &self.pending_dir().join(record_name(item.id())),
            &self.record(&item, None),
        )?;
        Ok(item)
    }

    pub fn claim_next(&self, worker: &WorkerId) -> Result<Option<InboxItem>, WorkerError> {
        let _state_lock = self.lock()?;
        let _epoch_lock = self.epochs.check_current(worker)?;
        let mut pending = self.pending()?;
        pending.sort_by(|left, right| left.order_key().cmp(&right.order_key()));
        let worker_dir = self.worker_dir(CLAIMED, worker)?;
        for item in pending {
            let source = self.pending_dir().join(record_name(item.id()));
            let target = worker_dir.join(record_name(item.id()));
            match rename_durable(&source, &target) {
                Ok(()) => return Ok(Some(item)),
                Err(error) if is_not_found(&error) => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(None)
    }

    pub fn claim(&self, worker: &WorkerId, request: &RequestId) -> Result<InboxItem, WorkerError> {
        let _state_lock = self.lock()?;
        let _epoch_lock = self.epochs.check_current(worker)?;
        let source = self.pending_dir().join(record_name(request));
        let target = self.worker_dir(CLAIMED, worker)?.join(record_name(request));
        let record = read_record(&source)?;
        self.validate_record(&record, &source, request, false)?;
        rename_durable(&source, &target)?;
        Ok(record.item)
    }

    pub fn complete(&self, worker: &WorkerId, request: &RequestId) -> Result<(), WorkerError> {
        let _state_lock = self.lock()?;
        let _epoch_lock = self.epochs.check_current(worker)?;
        let source = self.worker_dir(CLAIMED, worker)?.join(record_name(request));
        let completed_dir = self.worker_dir(COMPLETED, worker)?;
        let target = completed_dir.join(record_name(request));
        let source_record = match read_optional_json::<InboxRecord>(&source)? {
            Some(record) => record,
            None => {
                let receipt = read_optional_json::<InboxRecord>(&target)?
                    .ok_or_else(|| not_claimed(worker, request))?;
                self.validate_receipt(&receipt, &target, request)?;
                return if receipt
                    .completion
                    .is_some_and(|completion| completion.outcome == CompletionOutcome::Executed)
                {
                    Ok(())
                } else {
                    Err(WorkerError::RequestConflict(request.clone()))
                };
            }
        };
        self.validate_record(&source_record, &source, request, false)?;
        if let Some(receipt) = read_optional_json::<InboxRecord>(&target)? {
            self.validate_receipt(&receipt, &target, request)?;
            if receipt.item == source_record.item
                && receipt
                    .completion
                    .is_some_and(|completion| completion.outcome == CompletionOutcome::Executed)
            {
                return remove_durable(&source);
            }
            return Err(WorkerError::RequestConflict(request.clone()));
        }
        write_json_atomic(
            &target,
            &self.record(
                &source_record.item,
                Some(Completion {
                    outcome: CompletionOutcome::Executed,
                    at_ms: now_ms(),
                    note: None,
                }),
            ),
        )?;
        remove_durable(&source)
    }

    pub fn reconcile(
        &self,
        current: &WorkerId,
        interrupted: &WorkerId,
        request: &RequestId,
        outcome: ReconciliationOutcome,
        note: Option<String>,
    ) -> Result<(), WorkerError> {
        let _state_lock = self.lock()?;
        let _epoch_lock = self.epochs.check_current(current)?;
        self.validate_worker(interrupted)?;
        let source = self
            .worker_dir(CLAIMED, interrupted)?
            .join(record_name(request));
        let target = self
            .worker_dir(COMPLETED, interrupted)?
            .join(record_name(request));
        let completion = Completion {
            outcome: CompletionOutcome::Reconciled(outcome),
            at_ms: now_ms(),
            note,
        };
        let source_record = match read_optional_json::<InboxRecord>(&source)? {
            Some(record) => record,
            None => {
                let receipt = read_optional_json::<InboxRecord>(&target)?
                    .ok_or_else(|| not_claimed(interrupted, request))?;
                self.validate_receipt(&receipt, &target, request)?;
                return if receipt
                    .completion
                    .is_some_and(|existing| existing.outcome == completion.outcome)
                {
                    Ok(())
                } else {
                    Err(WorkerError::RequestConflict(request.clone()))
                };
            }
        };
        self.validate_record(&source_record, &source, request, false)?;
        if let Some(receipt) = read_optional_json::<InboxRecord>(&target)? {
            self.validate_receipt(&receipt, &target, request)?;
            if receipt.item == source_record.item
                && receipt
                    .completion
                    .is_some_and(|existing| existing.outcome == completion.outcome)
            {
                return remove_durable(&source);
            }
            return Err(WorkerError::RequestConflict(request.clone()));
        }
        write_json_atomic(&target, &self.record(&source_record.item, Some(completion)))?;
        remove_durable(&source)
    }

    pub fn cancel_pending(&self, request: &RequestId) -> Result<bool, WorkerError> {
        let _state_lock = self.lock()?;
        let path = self.pending_dir().join(record_name(request));
        match remove_durable(&path) {
            Ok(()) => Ok(true),
            Err(error) if is_not_found(&error) => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub fn recover(&self) -> Result<RecoverySnapshot, WorkerError> {
        let _state_lock = self.lock()?;
        let mut records: BTreeMap<RequestId, Vec<LocatedRecord>> = BTreeMap::new();
        let mut accept_seqs: BTreeMap<u64, RequestId> = BTreeMap::new();
        for state in [PENDING, CLAIMED, COMPLETED] {
            let dirs = if state == PENDING {
                vec![self.pending_dir()]
            } else {
                self.worker_dirs(state)?
            };
            for dir in dirs {
                let worker = if state == PENDING {
                    None
                } else {
                    Some(
                        self.worker_from_key(
                            dir.file_name()
                                .and_then(|name| name.to_str())
                                .ok_or_else(|| WorkerError::Corrupt {
                                    path: dir.clone(),
                                    message: "worker 目录名不是合法文本".to_string(),
                                })?,
                        )?,
                    )
                };
                for path in json_files(&dir)? {
                    let record = read_record(&path)?;
                    self.validate_record(&record, &path, record.item.id(), state == COMPLETED)?;
                    match accept_seqs.entry(record.item.accept_seq) {
                        std::collections::btree_map::Entry::Vacant(entry) => {
                            entry.insert(record.item.request.id.clone());
                        }
                        std::collections::btree_map::Entry::Occupied(entry) => {
                            if entry.get() != record.item.id() {
                                return Err(WorkerError::Corrupt {
                                    path: path.clone(),
                                    message: "两个请求复用了同一个接受序号".to_string(),
                                });
                            }
                        }
                    }
                    records
                        .entry(record.item.request.id.clone())
                        .or_default()
                        .push(LocatedRecord {
                            state,
                            worker: worker.clone(),
                            path,
                            item: record.item,
                        });
                }
            }
        }
        for (id, locations) in &records {
            if locations.len() <= 1 {
                continue;
            }
            let completed = locations
                .iter()
                .find(|location| location.state == COMPLETED);
            let claimed = locations.iter().find(|location| location.state == CLAIMED);
            let interrupted_finish = match (completed, claimed) {
                (Some(completed), Some(claimed))
                    if locations.len() == 2
                        && completed.worker == claimed.worker
                        && locations
                            .iter()
                            .all(|location| location.item == locations[0].item) =>
                {
                    Some(claimed)
                }
                _ => None,
            };
            let Some(claimed) = interrupted_finish else {
                return Err(WorkerError::Corrupt {
                    path: self.root.clone(),
                    message: format!("请求 {id} 同时存在多份持久记录"),
                });
            };
            remove_durable(&claimed.path)?;
        }
        let mut pending = self.pending()?;
        pending.sort_by(|left, right| left.order_key().cmp(&right.order_key()));
        let mut interrupted = Vec::new();
        for worker_dir in self.worker_dirs(CLAIMED)? {
            let worker = self.worker_from_key(
                worker_dir
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| WorkerError::Corrupt {
                        path: worker_dir.clone(),
                        message: "认领目录名不是合法 WorkerId".to_string(),
                    })?,
            )?;
            for path in json_files(&worker_dir)? {
                interrupted.push(InterruptedItem {
                    worker: worker.clone(),
                    item: read_record(&path)?.item,
                });
            }
        }
        interrupted.sort_by(|left, right| {
            left.worker
                .cmp(&right.worker)
                .then_with(|| left.item.order_key().cmp(&right.item.order_key()))
        });
        Ok(RecoverySnapshot {
            pending,
            interrupted,
        })
    }

    fn next_accept_seq(&self) -> Result<u64, WorkerError> {
        let path = self.root.join(SEQUENCE_FILE);
        let next = match read_optional_json::<SequenceRecord>(&path)? {
            Some(record) if record.version == 1 && record.next != 0 => record.next,
            Some(record) => {
                return Err(WorkerError::Corrupt {
                    path,
                    message: format!("无效 inbox 序号记录: {record:?}"),
                })
            }
            None => 1,
        };
        let following = next
            .checked_add(1)
            .ok_or_else(|| WorkerError::Invalid("inbox 接受序号已耗尽".to_string()))?;
        write_json_atomic(
            &path,
            &SequenceRecord {
                version: 1,
                next: following,
            },
        )?;
        Ok(next)
    }

    fn validate_record(
        &self,
        record: &InboxRecord,
        path: &Path,
        request: &RequestId,
        completed: bool,
    ) -> Result<(), WorkerError> {
        if record.item.accept_seq == 0 || record.item.accepted_by_pid == 0 {
            return Err(WorkerError::Corrupt {
                path: path.to_path_buf(),
                message: "inbox 接受元数据无效".to_string(),
            });
        }
        if record.item.request.id.as_str() != request.as_str() {
            return Err(WorkerError::Corrupt {
                path: path.to_path_buf(),
                message: "inbox 记录与 request_id 不一致".to_string(),
            });
        }
        match (&record.completion, completed) {
            (None, false) => Ok(()),
            (Some(_), true) => Ok(()),
            _ => Err(WorkerError::Corrupt {
                path: path.to_path_buf(),
                message: "inbox 记录的完成状态与目录不一致".to_string(),
            }),
        }
    }

    fn validate_receipt(
        &self,
        record: &InboxRecord,
        path: &Path,
        request: &RequestId,
    ) -> Result<(), WorkerError> {
        self.validate_record(record, path, request, true)
    }

    fn pending(&self) -> Result<Vec<InboxItem>, WorkerError> {
        json_files(&self.pending_dir())?
            .iter()
            .map(|path| Ok(read_record(path)?.item))
            .collect()
    }

    fn find_accepted(&self, id: &RequestId) -> Result<Option<InboxItem>, WorkerError> {
        let mut found = Vec::new();
        let direct = self.pending_dir().join(record_name(id));
        if let Some(record) = read_optional_json::<InboxRecord>(&direct)? {
            self.validate_record(&record, &direct, id, false)?;
            found.push(record.item);
        }
        for state in [CLAIMED, COMPLETED] {
            for worker_dir in self.worker_dirs(state)? {
                let path = worker_dir.join(record_name(id));
                if let Some(record) = read_optional_json::<InboxRecord>(&path)? {
                    self.validate_record(&record, &path, id, state == COMPLETED)?;
                    found.push(record.item);
                }
            }
        }
        match found.as_slice() {
            [] => Ok(None),
            [item] => Ok(Some(item.clone())),
            _ => Err(WorkerError::Corrupt {
                path: self.root.clone(),
                message: format!("请求 {id} 同时存在多份持久记录"),
            }),
        }
    }

    fn worker_dirs(&self, state: &str) -> Result<Vec<PathBuf>, WorkerError> {
        let root = self.root.join(state);
        let entries = match fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(io_error("枚举 worker 目录", &root, error)),
        };
        let mut dirs = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| io_error("枚举 worker 目录", &root, error))?;
            let path = entry.path();
            let kind = entry
                .file_type()
                .map_err(|error| io_error("读取 worker 目录类型", &path, error))?;
            if kind.is_dir() {
                self.worker_from_key(entry.file_name().to_str().ok_or_else(|| {
                    WorkerError::Corrupt {
                        path: path.clone(),
                        message: "worker 目录名不是合法文本".to_string(),
                    }
                })?)?;
                dirs.push(path);
            }
        }
        dirs.sort();
        Ok(dirs)
    }

    fn worker_from_key(&self, key: &str) -> Result<WorkerId, WorkerError> {
        let (epoch, build_id) = key.split_once('-').ok_or_else(|| WorkerError::Corrupt {
            path: self.root.clone(),
            message: format!("worker 目录名缺少 epoch/build 分界: {key}"),
        })?;
        let epoch = epoch
            .strip_prefix('e')
            .and_then(|value| u64::from_str_radix(value, 16).ok())
            .and_then(Epoch::new)
            .ok_or_else(|| WorkerError::Corrupt {
                path: self.root.clone(),
                message: format!("worker 目录名 epoch 无效: {key}"),
            })?;
        WorkerId::new(
            &self.session_id,
            epoch,
            BuildId::parse(build_id).map_err(|_| WorkerError::Corrupt {
                path: self.root.clone(),
                message: format!("worker 目录名 build_id 无效: {key}"),
            })?,
        )
    }

    fn worker_dir(&self, state: &str, worker: &WorkerId) -> Result<PathBuf, WorkerError> {
        self.validate_worker(worker)?;
        Ok(self.root.join(state).join(worker.key()))
    }

    fn validate_worker(&self, worker: &WorkerId) -> Result<(), WorkerError> {
        if worker.session_id() == self.session_id {
            Ok(())
        } else {
            Err(WorkerError::Invalid(
                "worker 属于另一个会话的 inbox".to_string(),
            ))
        }
    }

    fn ensure_dirs(&self) -> Result<(), WorkerError> {
        for state in [PENDING, CLAIMED, COMPLETED] {
            let path = self.root.join(state);
            create_dir_durable(&path)?;
            sync_parent(&path)?;
            sync_parent(&self.root)?;
        }
        Ok(())
    }

    fn lock(&self) -> Result<File, WorkerError> {
        exclusive_lock(&self.root.join(STATE_LOCK))
    }

    fn pending_dir(&self) -> PathBuf {
        self.root.join(PENDING)
    }

    fn record(&self, item: &InboxItem, completion: Option<Completion>) -> InboxRecord {
        InboxRecord {
            version: RECORD_VERSION,
            item: item.clone(),
            completion,
        }
    }
}

fn not_claimed(worker: &WorkerId, request: &RequestId) -> WorkerError {
    WorkerError::NotClaimed {
        worker: worker.clone(),
        request: request.clone(),
    }
}

fn is_not_found(error: &WorkerError) -> bool {
    matches!(
        error,
        WorkerError::Io { kind, .. } if *kind == std::io::ErrorKind::NotFound
    )
}

fn record_name(id: &RequestId) -> String {
    format!("{}.json", id.as_str())
}

fn json_files(dir: &Path) -> Result<Vec<PathBuf>, WorkerError> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(io_error("枚举 inbox", dir, error)),
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| io_error("枚举 inbox", dir, error))?;
        let path = entry.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            paths.push(path);
        }
    }
    paths.sort();
    Ok(paths)
}

fn read_record(path: &Path) -> Result<InboxRecord, WorkerError> {
    let record = read_json::<InboxRecord>(path)?;
    if record.version != RECORD_VERSION {
        return Err(WorkerError::Corrupt {
            path: path.to_path_buf(),
            message: format!("未知 inbox 记录版本 {}", record.version),
        });
    }
    let Some(stem) = path.file_stem().and_then(|stem| stem.to_str()) else {
        return Err(WorkerError::Corrupt {
            path: path.to_path_buf(),
            message: "inbox 文件名不是合法文本".to_string(),
        });
    };
    if record.item.request.id.as_str() != stem {
        return Err(WorkerError::Corrupt {
            path: path.to_path_buf(),
            message: "inbox 文件名与 request_id 不一致".to_string(),
        });
    }
    Ok(record)
}
