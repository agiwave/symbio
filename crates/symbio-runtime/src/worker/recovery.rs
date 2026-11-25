use std::fs::File;
use std::sync::Arc;

use async_trait::async_trait;

use super::{EpochRegistry, RecoverySnapshot, WorkerError, WorkerId};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecoveryReport {
    pub snapshot: RecoverySnapshot,
}

#[async_trait]
pub trait WorkerRecovery: Send + Sync {
    /// 返回前必须完成 WAL 恢复并持有跨进程写者令牌；令牌由返回的恢复对象持有至 drop。
    async fn recover(&self, worker: &WorkerId) -> Result<RecoveryReport, WorkerError>;
}

pub struct WorkerStartup {
    recovery: Arc<dyn WorkerRecovery>,
    epochs: EpochRegistry,
}

impl WorkerStartup {
    pub fn new(recovery: Arc<dyn WorkerRecovery>, epochs: EpochRegistry) -> Self {
        Self { recovery, epochs }
    }

    pub async fn ready(self, worker: WorkerId) -> Result<WorkerReady, WorkerError> {
        let worker_lease = self.epochs.acquire_current(&worker)?;
        let report = self.recovery.recover(&worker).await?;
        if !report.snapshot.interrupted.is_empty() {
            return Err(WorkerError::RecoveryRequired {
                session_id: worker.session_id().to_string(),
                interrupted: report.snapshot.interrupted,
            });
        }
        Ok(WorkerReady {
            worker,
            report,
            _recovery: self.recovery,
            _worker_lease: worker_lease,
        })
    }
}

pub struct WorkerReady {
    worker: WorkerId,
    report: RecoveryReport,
    _recovery: Arc<dyn WorkerRecovery>,
    _worker_lease: File,
}

impl WorkerReady {
    pub fn worker(&self) -> &WorkerId {
        &self.worker
    }

    pub fn recovery(&self) -> &RecoveryReport {
        &self.report
    }
}
