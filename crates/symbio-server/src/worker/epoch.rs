use std::fs::File;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::id::{validate_session_id, BuildId, Epoch, WorkerId};
use super::{read_optional_json, try_exclusive_lock, write_json_atomic, WorkerError};

const EPOCH_FILE: &str = "epoch.json";
const EPOCH_LOCK: &str = "epoch.lock";

#[derive(Debug, Serialize, Deserialize)]
struct EpochRecord {
    version: u8,
    worker: WorkerId,
}

pub struct EpochRegistry {
    session_id: String,
    session_dir: PathBuf,
}

impl EpochRegistry {
    pub fn new(
        session_id: impl Into<String>,
        session_dir: impl Into<PathBuf>,
    ) -> Result<Self, WorkerError> {
        let session_id = session_id.into();
        validate_session_id(&session_id)?;
        Ok(Self {
            session_id,
            session_dir: session_dir.into(),
        })
    }

    pub fn allocate(&self, build_id: BuildId) -> Result<WorkerId, WorkerError> {
        let _lock = self.lock()?;
        let next = match self.current_unlocked()? {
            Some(worker) => worker.epoch().value().checked_add(1),
            None => Some(1),
        }
        .ok_or_else(|| WorkerError::Invalid("epoch 已耗尽".to_string()))?;
        let epoch =
            Epoch::new(next).ok_or_else(|| WorkerError::Invalid("epoch 已耗尽".to_string()))?;
        let worker = WorkerId::new(&self.session_id, epoch, build_id)?;
        write_json_atomic(
            &self.worker_dir().join(EPOCH_FILE),
            &EpochRecord {
                version: 1,
                worker: worker.clone(),
            },
        )?;
        Ok(worker)
    }

    pub fn current(&self) -> Result<Option<WorkerId>, WorkerError> {
        let _lock = self.lock()?;
        self.current_unlocked()
    }

    pub(super) fn check_current(&self, worker: &WorkerId) -> Result<File, WorkerError> {
        let lock = self.lock()?;
        match self.current_unlocked()? {
            Some(current) => worker.validate(&current)?,
            None => {
                return Err(WorkerError::Invalid(
                    "会话尚未分配 worker epoch".to_string(),
                ))
            }
        }
        Ok(lock)
    }

    fn lock(&self) -> Result<File, WorkerError> {
        try_exclusive_lock(&self.worker_dir().join(EPOCH_LOCK))
    }

    fn current_unlocked(&self) -> Result<Option<WorkerId>, WorkerError> {
        let path = self.worker_dir().join(EPOCH_FILE);
        let Some(record) = read_optional_json::<EpochRecord>(&path)? else {
            return Ok(None);
        };
        if record.version != 1 {
            return Err(WorkerError::Corrupt {
                path,
                message: format!("未知 epoch 记录版本 {}", record.version),
            });
        }
        if record.worker.session_id() != self.session_id {
            return Err(WorkerError::Corrupt {
                path,
                message: "epoch 记录属于另一个会话".to_string(),
            });
        }
        Ok(Some(record.worker))
    }

    fn worker_dir(&self) -> PathBuf {
        self.session_dir.join("worker")
    }
}
