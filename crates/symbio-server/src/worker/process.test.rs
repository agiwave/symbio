use super::super::{BuildId, Epoch, InboxRequest};
use super::*;
use crate::worker::event_relay::mark_relayed;

fn test_launcher() -> (
    Arc<StdioLauncher>,
    Arc<mpsc::Sender<WorkerLine>>,
    mpsc::Receiver<WorkerLine>,
    SubagentReplies,
    WorkerId,
) {
    let (tx, rx) = mpsc::channel(4);
    let tx = Arc::new(tx);
    let pending = Arc::new(StdMutex::new(HashMap::new()));
    let worker = WorkerId::new(
        "parent",
        Epoch::new(1).expect("epoch"),
        BuildId::parse(format!("{:064x}", 1)).expect("build"),
    )
    .expect("worker");
    (
        Arc::new(StdioLauncher {
            tx: Arc::downgrade(&tx),
            pending: Arc::clone(&pending),
            worker: worker.clone(),
        }),
        tx,
        rx,
        pending,
        worker,
    )
}

fn subagent_request() -> SubagentRequest {
    SubagentRequest {
        child_session_id: "child".to_string(),
        message: PluginMessageWire {
            metadata: json!({ "path": "vdfs/write", "session_id": "child" }),
            payload: json!({ "path": "session/child/inbox", "text": "hello" }),
        },
    }
}

fn started_id(line: WorkerLine) -> RequestId {
    let WorkerLine::StartSubagent { id, .. } = line else {
        panic!("expected subagent start");
    };
    id
}

fn cancelled_id(line: WorkerLine) -> (RequestId, WorkerId) {
    let WorkerLine::CancelSubagent { id, worker } = line else {
        panic!("expected subagent cancellation");
    };
    (id, worker)
}

struct LauncherRecovery {
    _launcher: Arc<StdioLauncher>,
}

#[async_trait]
impl WorkerRecovery for LauncherRecovery {
    async fn recover(&self, _worker: &WorkerId) -> Result<RecoveryReport, WorkerError> {
        Ok(RecoveryReport::default())
    }
}

#[test]
fn weak_launcher_output_sender_allows_eof_while_recovery_lease_is_held() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        let temp = tempfile::tempdir().expect("tempdir");
        let build = BuildId::parse(format!("{:064x}", 1)).expect("build");
        let worker = EpochRegistry::new("session-1", temp.path())
            .expect("registry")
            .allocate(build)
            .expect("epoch");
        let (launcher, tx, mut rx, _, _) = test_launcher();
        let writer = tokio::spawn(async move { while rx.recv().await.is_some() {} });
        let recovery: Arc<dyn WorkerRecovery> = Arc::new(LauncherRecovery {
            _launcher: Arc::clone(&launcher),
        });
        let ready = WorkerStartup::new(
            recovery,
            EpochRegistry::new("session-1", temp.path()).expect("registry"),
        )
        .ready(worker)
        .await
        .expect("ready");
        drop(launcher);
        drop(tx);
        tokio::time::timeout(std::time::Duration::from_secs(1), writer)
            .await
            .expect("writer must observe EOF")
            .expect("writer task");
        drop(ready);
    });
}

#[tokio::test]
async fn normal_subagent_reply_does_not_emit_a_redundant_cancel() {
    let (launcher, _tx, mut rx, pending, _worker) = test_launcher();
    let task = {
        let launcher = Arc::clone(&launcher);
        tokio::spawn(async move {
            launcher
                .launch(subagent_request(), ExecAbortSignal::new())
                .await
        })
    };
    let id = started_id(rx.recv().await.expect("start line"));
    let reply = pending
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&id)
        .expect("pending reply");
    reply.send(Ok(())).expect("reply");
    task.await.expect("launch task").expect("launch");
    assert!(pending.lock().expect("pending").is_empty());
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn aborted_subagent_launch_cleans_pending_and_requests_cancellation() {
    let (launcher, _tx, mut rx, pending, worker) = test_launcher();
    let abort = ExecAbortSignal::new();
    let task = {
        let launcher = Arc::clone(&launcher);
        let abort = abort.clone();
        tokio::spawn(async move { launcher.launch(subagent_request(), abort).await })
    };
    let id = started_id(rx.recv().await.expect("start line"));
    abort.abort();
    let error = task
        .await
        .expect("launch task")
        .expect_err("aborted launch");
    assert!(error.contains("已中止"));
    assert_eq!(
        cancelled_id(rx.recv().await.expect("cancel line")),
        (id, worker)
    );
    assert!(pending.lock().expect("pending").is_empty());
}

#[tokio::test]
async fn dropped_subagent_launch_cleans_pending_and_requests_cancellation() {
    let (launcher, _tx, mut rx, pending, worker) = test_launcher();
    let task = {
        let launcher = Arc::clone(&launcher);
        tokio::spawn(async move {
            launcher
                .launch(subagent_request(), ExecAbortSignal::new())
                .await
        })
    };
    let id = started_id(rx.recv().await.expect("start line"));
    task.abort();
    assert!(task.await.expect_err("dropped launch").is_cancelled());
    assert_eq!(
        cancelled_id(rx.recv().await.expect("cancel line")),
        (id, worker)
    );
    assert!(pending.lock().expect("pending").is_empty());
}

#[tokio::test]
async fn worker_shutdown_fails_pending_subagent_launches() {
    let (launcher, _tx, mut rx, pending, _worker) = test_launcher();
    let task = {
        let launcher = Arc::clone(&launcher);
        tokio::spawn(async move {
            launcher
                .launch(subagent_request(), ExecAbortSignal::new())
                .await
        })
    };
    let _id = started_id(rx.recv().await.expect("start line"));
    launcher.fail_all("worker 已停止");
    let error = task
        .await
        .expect("launch task")
        .expect_err("stopped launch");
    assert!(error.contains("worker 已停止"));
    assert!(pending.lock().expect("pending").is_empty());
    assert!(rx.try_recv().is_err());
}

#[test]
fn production_recovery_abandons_interrupted_work_and_retains_pending_requests() {
    let temp = tempfile::tempdir().expect("tempdir");
    let inbox = DurableInbox::new("session-1", temp.path()).expect("inbox");
    let interrupted = EpochRegistry::new("session-1", temp.path())
        .expect("registry")
        .allocate(BuildId::parse(format!("{:064x}", 1)).expect("build"))
        .expect("interrupted epoch");
    inbox
        .enqueue(InboxRequest::new("request-a", json!({}), json!({}), None).expect("request"))
        .expect("enqueue a");
    inbox
        .enqueue(InboxRequest::new("request-b", json!({}), json!({}), None).expect("request"))
        .expect("enqueue b");
    inbox.claim_next(&interrupted).expect("claim a");
    let current = EpochRegistry::new("session-1", temp.path())
        .expect("registry")
        .allocate(BuildId::parse(format!("{:064x}", 1)).expect("build"))
        .expect("current epoch");

    let recovered = reconcile_interrupted(&inbox, &current, &inbox.recover().expect("recover"))
        .expect("production recovery");
    assert!(recovered.interrupted.is_empty());
    assert_eq!(recovered.pending.len(), 1);
    assert_eq!(recovered.pending[0].id().as_str(), "request-b");
    let receipt: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            temp.path()
                .join("worker/inbox/completed")
                .join(interrupted.key())
                .join("request-a.json"),
        )
        .expect("receipt"),
    )
    .expect("receipt json");
    assert_eq!(receipt["completion"]["outcome"]["reconciled"], "abandoned");
    assert!(inbox.recover().expect("final recovery").pending.len() == 1);
}

#[test]
fn nested_session_directory_keeps_identity_and_address() {
    let home = tempfile::tempdir().expect("home");
    let homedir = std::fs::canonicalize(home.path()).expect("canonical home");
    let nested = homedir.join("session/parent/sessions/child");
    std::fs::create_dir_all(&nested).expect("nested dir");
    std::fs::write(nested.join("session.json"), br#"{"id":"child"}"#).expect("session metadata");

    resolve_session_dir(&homedir, &nested, "child").expect("nested");
    assert_eq!(
        session_address("/vdfs", "child").expect("address"),
        "/vdfs/session/child"
    );
    assert!(resolve_session_dir(&homedir, &nested, "parent").is_err());
    assert!(resolve_session_dir(&homedir, &homedir, "child").is_err());
}

#[tokio::test]
async fn relayed_frames_do_not_stop_local_forwarding() {
    let (input_tx, input_rx) = mpsc::channel(3);
    let (output_tx, mut output_rx) = mpsc::channel(3);
    let stop = CancellationToken::new();
    let task = tokio::spawn(forward_events(input_rx, output_tx, stop.clone()));

    input_tx
        .send(PluginFrame::data(
            json!({ "data": { "kind": "vdfs.changed", "data": { "path": "first" } } }),
        ))
        .await
        .expect("send first");
    input_tx
        .send(mark_relayed(PluginFrame::data(json!({
            "data": { "kind": "vdfs.changed", "data": { "path": "relayed" } }
        }))))
        .await
        .expect("send relayed");
    input_tx
        .send(PluginFrame::data(
            json!({ "data": { "kind": "vdfs.changed", "data": { "path": "last" } } }),
        ))
        .await
        .expect("send last");

    for expected in ["first", "last"] {
        let line = tokio::time::timeout(std::time::Duration::from_secs(1), output_rx.recv())
            .await
            .expect("forward timeout")
            .expect("forward frame");
        let WorkerLine::Event { frame } = line else {
            panic!("expected event");
        };
        assert_eq!(frame.value().unwrap()["data"]["data"]["path"], expected);
    }
    assert!(output_rx.try_recv().is_err());
    stop.cancel();
    task.await.expect("forward task");
}
