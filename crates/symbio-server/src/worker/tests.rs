use std::fs::{self, File, OpenOptions};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::json;
use symbio::symbio_kernel::plugin::PluginMessageWire;

use super::protocol::{read_line, write_line, WorkerLine};
use super::*;

fn build(tag: u8) -> BuildId {
    BuildId::parse(format!("{tag:064x}")).expect("build id")
}

fn worker(epoch: u64, tag: u8) -> WorkerId {
    WorkerId::new("session-1", Epoch::new(epoch).expect("epoch"), build(tag)).expect("worker id")
}

fn allocate(temp: &std::path::Path, tag: u8) -> WorkerId {
    EpochRegistry::new("session-1", temp)
        .expect("registry")
        .allocate(build(tag))
        .expect("allocate worker")
}

fn request(id: &str, text: &str) -> InboxRequest {
    InboxRequest::new(id, json!({ "text": text }), json!({}), None).expect("request")
}

fn retry_busy<T>(mut operation: impl FnMut() -> Result<T, WorkerError>) -> T {
    for _ in 0..1000 {
        match operation() {
            Err(WorkerError::Busy) => std::thread::yield_now(),
            result => return result.expect("inbox operation"),
        }
    }
    panic!("inbox lock remained busy");
}

#[test]
fn worker_identity_round_trips_and_rejects_invalid_components() {
    let original = worker(7, 3);
    let encoded = serde_json::to_string(&original).expect("serialize worker");
    let decoded: WorkerId = serde_json::from_str(&encoded).expect("deserialize worker");
    assert_eq!(decoded, original);
    assert!(WorkerId::new("../escape", original.epoch(), original.build_id().clone()).is_err());
    assert!(BuildId::parse("not-a-build-id").is_err());
    assert!(RequestId::parse("../request").is_err());
    assert!(serde_json::from_str::<Epoch>("0").is_err());
    assert!(serde_json::from_str::<WorkerId>(
        &json!({
            "session_id": "../escape",
            "epoch": 1,
            "build_id": build(1).as_str(),
        })
        .to_string()
    )
    .is_err());
}

#[test]
fn fenced_frames_reject_old_epoch_build_and_session() {
    let current = worker(9, 4);
    let frame = Fenced::new(current.clone(), json!({ "ok": true }));
    assert_eq!(frame.validate(&current).unwrap(), &json!({ "ok": true }));
    assert!(matches!(
        frame.validate(&worker(8, 4)),
        Err(WorkerError::StaleWorker { .. })
    ));
    assert!(matches!(
        frame.validate(&worker(9, 5)),
        Err(WorkerError::StaleWorker { .. })
    ));
    let other_session = WorkerId::new("session-2", current.epoch(), current.build_id().clone())
        .expect("other session");
    assert!(matches!(
        frame.validate(&other_session),
        Err(WorkerError::StaleWorker { .. })
    ));
}

#[test]
fn epoch_registry_persists_monotonic_epochs_and_excludes_concurrent_allocators() {
    let temp = tempfile::tempdir().expect("tempdir");
    let registry = EpochRegistry::new("session-1", temp.path()).expect("registry");
    let first = registry.allocate(build(1)).expect("first epoch");
    let second = registry.allocate(build(1)).expect("second epoch");
    assert_eq!(first.epoch().value(), 1);
    assert_eq!(second.epoch().value(), 2);

    let reopened = EpochRegistry::new("session-1", temp.path()).expect("reopened registry");
    assert_eq!(reopened.current().expect("current"), Some(second));

    let worker_dir = temp.path().join("worker");
    let _lock = try_exclusive_lock(&worker_dir.join("epoch.lock")).expect("hold epoch lock");
    assert!(matches!(
        registry.allocate(build(1)),
        Err(WorkerError::Busy)
    ));
}

#[test]
fn epoch_exhaustion_fails_closed() {
    let temp = tempfile::tempdir().expect("tempdir");
    let registry = EpochRegistry::new("session-1", temp.path()).expect("registry");
    let max = worker(u64::MAX, 1);
    fs::create_dir_all(temp.path().join("worker")).expect("worker dir");
    fs::write(
        temp.path().join("worker/epoch.json"),
        serde_json::to_vec(&json!({ "version": 1, "worker": max })).expect("epoch record"),
    )
    .expect("write epoch");
    assert!(matches!(
        registry.allocate(build(2)),
        Err(WorkerError::Invalid(message)) if message.contains("耗尽")
    ));
}

#[test]
fn durable_inbox_preserves_fifo_and_deduplicates_request_ids() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inbox = DurableInbox::new("session-1", temp.path()).expect("inbox");
    let worker = allocate(temp.path(), 1);
    let first = inbox
        .enqueue(request("request-a", "first"))
        .expect("enqueue a");
    let second = inbox
        .enqueue(request("request-b", "second"))
        .expect("enqueue b");
    assert!(first.accept_seq() < second.accept_seq());
    assert_eq!(
        inbox
            .enqueue(request("request-a", "first"))
            .expect("retry a"),
        first
    );
    assert!(matches!(
        inbox.enqueue(request("request-a", "different")),
        Err(WorkerError::RequestConflict(_))
    ));
    assert_eq!(inbox.claim_next(&worker).expect("claim a"), Some(first));
    assert_eq!(inbox.claim_next(&worker).expect("claim b"), Some(second));
    assert_eq!(inbox.claim_next(&worker).expect("empty"), None);
}

#[test]
fn inbox_rejects_stale_epoch_and_build_before_claiming() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inbox = DurableInbox::new("session-1", temp.path()).expect("inbox");
    let first = allocate(temp.path(), 1);
    let current = allocate(temp.path(), 1);
    inbox
        .enqueue(request("request-a", "first"))
        .expect("enqueue");
    assert!(matches!(
        inbox.claim_next(&first),
        Err(WorkerError::StaleWorker { .. })
    ));
    assert!(matches!(
        inbox.claim_next(&worker(current.epoch().value(), 2)),
        Err(WorkerError::StaleWorker { .. })
    ));
    assert_eq!(
        inbox
            .claim_next(&current)
            .expect("current claim")
            .map(|item| item.id().clone()),
        Some(RequestId::parse("request-a").expect("id"))
    );
}

#[test]
fn concurrent_enqueue_is_idempotent_and_uses_one_acceptance_sequence() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inbox = Arc::new(DurableInbox::new("session-1", temp.path()).expect("inbox"));
    let mut tasks = Vec::new();
    for _ in 0..8 {
        let temp = temp.path().to_path_buf();
        tasks.push(std::thread::spawn(move || {
            let inbox = DurableInbox::new("session-1", temp).expect("inbox");
            retry_busy(|| inbox.enqueue(request("request-a", "same")))
        }));
    }
    let items: Vec<_> = tasks
        .into_iter()
        .map(|task| task.join().expect("enqueue task"))
        .collect();
    assert!(items.iter().all(|item| item == &items[0]));
    assert_eq!(
        inbox.recover().expect("recover").pending,
        vec![items[0].clone()]
    );
}

#[test]
fn acceptance_sequence_survives_reopen() {
    let temp = tempfile::tempdir().expect("tempdir");
    let first = DurableInbox::new("session-1", temp.path())
        .expect("inbox")
        .enqueue(request("request-a", "first"))
        .expect("enqueue");
    let second = DurableInbox::new("session-1", temp.path())
        .expect("inbox")
        .enqueue(request("request-b", "second"))
        .expect("enqueue");
    assert!(first.accept_seq() < second.accept_seq());
}

#[test]
fn crash_recovery_separates_pending_requests_from_interrupted_claims() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inbox = DurableInbox::new("session-1", temp.path()).expect("inbox");
    let worker = allocate(temp.path(), 1);
    let first = inbox
        .enqueue(request("request-a", "first"))
        .expect("enqueue");
    let second = inbox
        .enqueue(request("request-b", "second"))
        .expect("enqueue");
    assert_eq!(
        inbox.claim_next(&worker).expect("claim a"),
        Some(first.clone())
    );
    std::fs::write(
        temp.path().join("worker/inbox/pending/.request-c.1.staged"),
        b"{partial",
    )
    .expect("staged crash remnant");

    let reopened = DurableInbox::new("session-1", temp.path()).expect("reopened inbox");
    let snapshot = reopened.recover().expect("recover");
    assert_eq!(snapshot.pending, vec![second]);
    assert_eq!(snapshot.interrupted.len(), 1);
    assert_eq!(snapshot.interrupted[0].worker(), &worker);
    assert_eq!(snapshot.interrupted[0].item(), &first);
}

#[tokio::test]
async fn startup_refuses_ready_until_interrupted_claims_are_reconciled() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inbox = DurableInbox::new("session-1", temp.path()).expect("inbox");
    let worker = allocate(temp.path(), 1);
    inbox
        .enqueue(request("request-a", "first"))
        .expect("enqueue");
    inbox.claim_next(&worker).expect("claim");
    let recovery = Arc::new(StaticRecovery {
        report: RecoveryReport {
            snapshot: inbox.recover().expect("recover"),
        },
    });
    let result = WorkerStartup::new(
        recovery,
        EpochRegistry::new("session-1", temp.path()).expect("registry"),
    )
    .ready(worker.clone())
    .await;
    assert!(matches!(
        result,
        Err(WorkerError::RecoveryRequired { interrupted, .. }) if interrupted.len() == 1
    ));
}

#[test]
fn completion_is_worker_fenced_and_idempotent_for_the_owner() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inbox = DurableInbox::new("session-1", temp.path()).expect("inbox");
    let stale = allocate(temp.path(), 1);
    let current = allocate(temp.path(), 1);
    let item = inbox
        .enqueue(request("request-a", "first"))
        .expect("enqueue");
    assert_eq!(
        inbox.claim_next(&current).expect("claim"),
        Some(item.clone())
    );
    let id = RequestId::parse("request-a").expect("id");
    assert!(matches!(
        inbox.complete(&stale, &id),
        Err(WorkerError::StaleWorker { .. })
    ));
    assert!(matches!(
        inbox.complete(&worker(current.epoch().value(), 2), &id),
        Err(WorkerError::StaleWorker { .. })
    ));
    inbox.complete(&current, &id).expect("complete");
    inbox.complete(&current, &id).expect("idempotent complete");
    assert!(inbox.recover().expect("recover").interrupted.is_empty());
    assert_eq!(
        inbox
            .enqueue(request("request-a", "first"))
            .expect("dedupe receipt"),
        item
    );
}

#[test]
fn reconciliation_records_an_explicit_terminal_outcome_without_replay() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inbox = DurableInbox::new("session-1", temp.path()).expect("inbox");
    let interrupted = allocate(temp.path(), 1);
    let item = inbox
        .enqueue(request("request-a", "first"))
        .expect("enqueue");
    inbox.claim_next(&interrupted).expect("claim");
    let current = allocate(temp.path(), 1);
    let id = item.id().clone();
    inbox
        .reconcile(
            &current,
            &interrupted,
            &id,
            ReconciliationOutcome::Confirmed,
            Some("external result verified".to_string()),
        )
        .expect("reconcile");
    inbox
        .reconcile(
            &current,
            &interrupted,
            &id,
            ReconciliationOutcome::Confirmed,
            None,
        )
        .expect("idempotent reconcile");
    let snapshot = inbox.recover().expect("recover");
    assert!(snapshot.pending.is_empty());
    assert!(snapshot.interrupted.is_empty());
    assert_eq!(
        inbox
            .enqueue(request("request-a", "first"))
            .expect("dedupe"),
        item
    );
}

#[test]
fn recovery_finishes_an_interrupted_receipt_publication() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inbox = DurableInbox::new("session-1", temp.path()).expect("inbox");
    let worker = allocate(temp.path(), 1);
    let item = inbox
        .enqueue(request("request-a", "first"))
        .expect("enqueue");
    inbox.claim_next(&worker).expect("claim");
    inbox.complete(&worker, item.id()).expect("complete");
    let target = temp
        .path()
        .join("worker/inbox/completed")
        .join(worker.key())
        .join("request-a.json");
    let source = temp
        .path()
        .join("worker/inbox/claimed")
        .join(worker.key())
        .join("request-a.json");
    let mut receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(&target).expect("receipt")).expect("receipt json");
    receipt["completion"] = serde_json::Value::Null;
    fs::create_dir_all(source.parent().expect("source parent")).expect("claimed dir");
    fs::write(&source, serde_json::to_vec(&receipt).expect("claim record"))
        .expect("simulate interrupted finish");
    let snapshot = inbox.recover().expect("recover");
    assert!(snapshot.interrupted.is_empty());
    assert!(!source.exists());
}

#[test]
fn corrupt_pending_record_is_not_silently_dropped() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inbox = DurableInbox::new("session-1", temp.path()).expect("inbox");
    std::fs::write(
        temp.path().join("worker/inbox/pending/broken.json"),
        b"{not-json",
    )
    .expect("write corrupt record");
    assert!(matches!(inbox.recover(), Err(WorkerError::Corrupt { .. })));
}

#[test]
fn duplicate_acceptance_sequence_blocks_recovery() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inbox = DurableInbox::new("session-1", temp.path()).expect("inbox");
    inbox
        .enqueue(request("request-a", "first"))
        .expect("enqueue a");
    let pending = temp.path().join("worker/inbox/pending");
    let path = pending.join("request-a.json");
    let mut record: serde_json::Value =
        serde_json::from_slice(&fs::read(&path).expect("record")).expect("record json");
    record["request"]["id"] = json!("request-b");
    fs::write(
        pending.join("request-b.json"),
        serde_json::to_vec(&record).expect("duplicate record"),
    )
    .expect("write duplicate sequence");
    assert!(matches!(inbox.recover(), Err(WorkerError::Corrupt { .. })));
}

#[test]
fn corrupt_completed_receipt_blocks_recovery() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inbox = DurableInbox::new("session-1", temp.path()).expect("inbox");
    let worker = allocate(temp.path(), 1);
    let item = inbox
        .enqueue(request("request-a", "first"))
        .expect("enqueue");
    inbox.claim_next(&worker).expect("claim");
    inbox.complete(&worker, item.id()).expect("complete");
    std::fs::write(
        temp.path()
            .join("worker/inbox/completed")
            .join(worker.key())
            .join("request-a.json"),
        b"{not-json",
    )
    .expect("corrupt receipt");
    assert!(matches!(inbox.recover(), Err(WorkerError::Corrupt { .. })));
}

#[tokio::test]
async fn ready_holds_the_recovery_lease_until_it_is_dropped() {
    let temp = tempfile::tempdir().expect("tempdir");
    let lock_path = temp.path().join("v2-events.wal.lock");
    let registry = EpochRegistry::new("session-1", temp.path()).expect("registry");
    let first_worker = registry.allocate(build(1)).expect("first epoch");
    let recovery: Arc<dyn WorkerRecovery> = Arc::new(WalLockRecovery {
        lock_path: lock_path.clone(),
        held: Mutex::new(None),
    });
    let first = WorkerStartup::new(
        recovery,
        EpochRegistry::new("session-1", temp.path()).expect("registry"),
    )
    .ready(first_worker.clone())
    .await
    .expect("first worker ready");
    assert_eq!(first.worker(), &first_worker);
    let second_worker = registry.allocate(build(1)).expect("second epoch");
    let competing: Arc<dyn WorkerRecovery> = Arc::new(WalLockRecovery {
        lock_path: lock_path.clone(),
        held: Mutex::new(None),
    });
    assert!(matches!(
        WorkerStartup::new(
            competing,
            EpochRegistry::new("session-1", temp.path()).expect("registry"),
        )
        .ready(second_worker.clone())
        .await,
        Err(WorkerError::Busy)
    ));
    drop(first);
    let recovery: Arc<dyn WorkerRecovery> = Arc::new(WalLockRecovery {
        lock_path,
        held: Mutex::new(None),
    });
    assert!(WorkerStartup::new(
        recovery,
        EpochRegistry::new("session-1", temp.path()).expect("registry"),
    )
    .ready(second_worker)
    .await
    .is_ok());
}

#[tokio::test]
async fn startup_rejects_a_worker_that_is_not_the_current_epoch() {
    let temp = tempfile::tempdir().expect("tempdir");
    allocate(temp.path(), 1);
    let recovery: Arc<dyn WorkerRecovery> = Arc::new(StaticRecovery {
        report: RecoveryReport::default(),
    });
    assert!(matches!(
        WorkerStartup::new(
            recovery,
            EpochRegistry::new("session-1", temp.path()).expect("registry"),
        )
        .ready(worker(2, 1))
        .await,
        Err(WorkerError::StaleWorker { .. })
    ));
}

#[test]
fn subagent_routes_fence_both_parent_and_child_workers() {
    let parent = worker(3, 1);
    let child =
        WorkerId::new("child-1", parent.epoch(), parent.build_id().clone()).expect("child worker");
    let route = SubagentRoute::new(parent.clone(), "child-1").expect("route");
    let frame = SubagentFrame::new(route, child.clone(), json!({ "event": "message.delta" }))
        .expect("frame");
    assert_eq!(
        frame.validate(&parent, &child).unwrap(),
        &json!({ "event": "message.delta" })
    );
    assert!(frame.validate(&worker(2, 1), &child).is_err());
    let stale_child =
        WorkerId::new("child-1", Epoch::new(2).expect("epoch"), build(1)).expect("stale child");
    assert!(frame.validate(&parent, &stale_child).is_err());
    assert!(SubagentFrame::new(
        SubagentRoute::new(parent, "child-1").expect("route"),
        worker(3, 1),
        json!({}),
    )
    .is_err());
}

#[test]
fn supervisor_routes_only_existing_session_scope() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session_id = "session-1";
    fs::create_dir_all(temp.path().join("session").join(session_id)).expect("session dir");
    let supervisor = WorkerSupervisor::new(temp.path().to_path_buf()).expect("supervisor");

    let session_call = PluginMessageWire {
        metadata: json!({ "path": "session/consume", "session_id": session_id }),
        payload: json!({}),
    };
    assert_eq!(
        supervisor.session_for(&session_call).expect("route"),
        Some(session_id.to_string())
    );

    let vdfs_call = PluginMessageWire {
        metadata: json!({ "path": "vdfs/write" }),
        payload: json!({ "path": format!("/session/{session_id}/inbox") }),
    };
    assert_eq!(
        supervisor.session_for(&vdfs_call).expect("route"),
        Some(session_id.to_string())
    );

    let control_call = PluginMessageWire {
        metadata: json!({ "path": "home/reload", "session_id": session_id }),
        payload: json!({}),
    };
    assert_eq!(supervisor.session_for(&control_call).expect("route"), None);
}

#[tokio::test]
async fn worker_lines_round_trip_over_stdio() {
    let current = worker(2, 3);
    let (mut writer, reader) = tokio::io::duplex(4096);
    let mut reader = tokio::io::BufReader::new(reader);
    let line = WorkerLine::Ready {
        worker: current.clone(),
    };
    write_line(&mut writer, &line).await.expect("write line");
    let raw: String = read_line(&mut reader)
        .await
        .expect("read line")
        .expect("line");
    let decoded: WorkerLine = serde_json::from_str(&raw).expect("decode line");
    assert_eq!(
        serde_json::to_value(&decoded).expect("encode decoded"),
        serde_json::to_value(&line).expect("encode original")
    );
}

struct StaticRecovery {
    report: RecoveryReport,
}

#[async_trait]
impl WorkerRecovery for StaticRecovery {
    async fn recover(&self, _worker: &WorkerId) -> Result<RecoveryReport, WorkerError> {
        Ok(self.report.clone())
    }
}

struct WalLockRecovery {
    lock_path: std::path::PathBuf,
    held: Mutex<Option<File>>,
}

#[async_trait]
impl WorkerRecovery for WalLockRecovery {
    async fn recover(&self, _worker: &WorkerId) -> Result<RecoveryReport, WorkerError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&self.lock_path)
            .map_err(|error| io_error("打开测试 WAL 锁", &self.lock_path, error))?;
        file.try_lock().map_err(|_| WorkerError::Busy)?;
        *self.held.lock().expect("held lock") = Some(file);
        Ok(RecoveryReport::default())
    }
}
