use super::*;
use tokio::task::JoinSet;

fn write_session(dir: &Path, id: &str, parent: Option<&str>) {
    std::fs::create_dir_all(dir).expect("session dir");
    std::fs::write(
        dir.join("session.json"),
        json!({
            "id": id,
            "metadata": { "parent_session_id": parent }
        })
        .to_string(),
    )
    .expect("session metadata");
}

fn message(route: &str) -> PluginMessageWire {
    PluginMessageWire {
        metadata: json!({ "path": route, "session_id": "child" }),
        payload: json!({ "path": "session/child/inbox", "text": "hello" }),
    }
}

#[tokio::test]
async fn subagent_rejects_a_child_owned_by_another_parent() {
    let temp = tempfile::tempdir().expect("tempdir");
    let sessions = temp.path().join("session");
    let parent_dir = sessions.join("parent");
    write_session(&parent_dir, "parent", None);
    write_session(
        &parent_dir.join("sessions").join("child"),
        "child",
        Some("other"),
    );
    let supervisor = WorkerSupervisor::new(temp.path().to_path_buf()).expect("supervisor");
    let parent = EpochRegistry::new("parent", parent_dir)
        .expect("registry")
        .allocate(supervisor.inner.build_id.clone())
        .expect("epoch");

    let error = start_subagent(
        &supervisor.inner,
        &parent,
        "child",
        message(ROUTE_VDFS_WRITE),
        &CancellationToken::new(),
        &AtomicBool::new(false),
    )
    .await
    .expect_err("must reject another parent's child");
    assert!(error.contains("不属于父会话"));
}

#[tokio::test]
async fn shutdown_rejects_new_requests_and_subagent_starts() {
    let temp = tempfile::tempdir().expect("tempdir");
    let sessions = temp.path().join("session");
    let parent_dir = sessions.join("parent");
    write_session(&parent_dir, "parent", None);
    write_session(
        &parent_dir.join("sessions").join("child"),
        "child",
        Some("parent"),
    );
    let supervisor = WorkerSupervisor::new(temp.path().to_path_buf()).expect("supervisor");
    let parent = EpochRegistry::new("parent", parent_dir)
        .expect("registry")
        .allocate(supervisor.inner.build_id.clone())
        .expect("epoch");

    supervisor.shutdown().await;

    assert!(supervisor.invoke(message(ROUTE_VDFS_WRITE)).await.is_err());
    assert!(start_subagent(
        &supervisor.inner,
        &parent,
        "child",
        message(ROUTE_VDFS_WRITE),
        &CancellationToken::new(),
        &AtomicBool::new(false),
    )
    .await
    .is_err());
    assert!(
        get_or_spawn_at(&supervisor.inner, "parent", sessions.join("parent"),)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn lifecycle_cancellation_unblocks_subagent_waiting_for_routing() {
    let temp = tempfile::tempdir().expect("tempdir");
    let sessions = temp.path().join("session");
    let parent_dir = sessions.join("parent");
    write_session(&parent_dir, "parent", None);
    write_session(
        &parent_dir.join("sessions").join("child"),
        "child",
        Some("parent"),
    );
    let supervisor = WorkerSupervisor::new(temp.path().to_path_buf()).expect("supervisor");
    let parent = EpochRegistry::new("parent", parent_dir)
        .expect("registry")
        .allocate(supervisor.inner.build_id.clone())
        .expect("epoch");
    let routing = supervisor.inner.routing.write().await;
    let inner = Arc::clone(&supervisor.inner);
    let dispatched = Arc::new(AtomicBool::new(false));
    let start_dispatched = Arc::clone(&dispatched);
    let start = tokio::spawn(async move {
        start_subagent(
            &inner,
            &parent,
            "child",
            message(ROUTE_VDFS_WRITE),
            &CancellationToken::new(),
            &start_dispatched,
        )
        .await
    });
    tokio::task::yield_now().await;
    let mut lifecycle = supervisor.inner.subagent_lifecycle.lock().await;
    std::mem::replace(&mut *lifecycle, CancellationToken::new()).cancel();
    drop(lifecycle);
    let error = tokio::time::timeout(std::time::Duration::from_secs(1), start)
        .await
        .expect("subagent start must not wait behind shutdown")
        .expect("start task")
        .expect_err("cancelled start");
    assert!(error.contains("已取消"));
    assert!(!supervisor.inner.workers.lock().await.contains_key("child"));
    assert!(
        DurableInbox::new("child", sessions.join("parent/sessions/child"))
            .expect("inbox")
            .recover()
            .expect("recover")
            .pending
            .is_empty()
    );
    drop(routing);
}

#[tokio::test]
async fn subagent_rejects_control_routes_before_spawning() {
    let temp = tempfile::tempdir().expect("tempdir");
    let sessions = temp.path().join("session");
    let parent_dir = sessions.join("parent");
    write_session(&parent_dir, "parent", None);
    write_session(
        &parent_dir.join("sessions").join("child"),
        "child",
        Some("parent"),
    );
    let supervisor = WorkerSupervisor::new(temp.path().to_path_buf()).expect("supervisor");
    let parent = EpochRegistry::new("parent", parent_dir)
        .expect("registry")
        .allocate(supervisor.inner.build_id.clone())
        .expect("epoch");

    let error = start_subagent(
        &supervisor.inner,
        &parent,
        "child",
        message("sandbox/update"),
        &CancellationToken::new(),
        &AtomicBool::new(false),
    )
    .await
    .expect_err("must reject control routes");
    assert!(error.contains("只允许 vdfs/write 或 vdfs/action"));
}

#[test]
fn routing_rejects_unknown_session_scope() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_session(&temp.path().join("session/parent"), "parent", None);
    write_session(&temp.path().join("session/child"), "child", None);
    let supervisor = WorkerSupervisor::new(temp.path().to_path_buf()).expect("supervisor");

    let mut unknown = message(ROUTE_VDFS_WRITE);
    unknown.payload["path"] = json!("session/missing/inbox");
    assert!(supervisor.session_for(&unknown).is_err());

    let mut mismatch = message(ROUTE_VDFS_WRITE);
    mismatch.metadata["session_id"] = json!("parent");
    mismatch.payload["path"] = json!("session/child/inbox");
    assert_eq!(
        supervisor
            .session_for(&mismatch)
            .expect("path is authoritative"),
        Some("child".to_string())
    );

    let mut unknown_action = message("session/consume");
    unknown_action.metadata["session_id"] = json!("missing");
    unknown_action.payload = json!({});
    assert!(supervisor.session_for(&unknown_action).is_err());
}

#[test]
fn routing_ignores_session_segments_outside_the_declared_root() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_session(&temp.path().join("session/child"), "child", None);
    let supervisor = WorkerSupervisor::new(temp.path().to_path_buf()).expect("supervisor");
    let declared = session_node_path("child").expect("session path");
    let root = declared_addr_root().expect("declared root");

    assert_eq!(
        session_path_in(&json!({ "path": "session/child/inbox" })),
        Some("child")
    );
    assert_eq!(
        session_path_in(&json!({ "path": format!("{declared}/inbox") })),
        Some("child")
    );
    assert_eq!(
        session_path_in(&json!({ "path": format!("{declared}/subsession/grand/messages") })),
        Some("grand")
    );

    for path in [
        "other/session/child/inbox".to_string(),
        format!("{root}/other/session/child/inbox"),
        "/session/child/inbox".to_string(),
    ] {
        let mut request = message(ROUTE_VDFS_WRITE);
        request.payload["path"] = json!(path);
        assert_eq!(
            supervisor.session_for(&request).expect("control route"),
            None
        );
    }
}

#[test]
fn routing_accepts_temporary_context_on_explicit_session_reads() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_session(&temp.path().join("session/child"), "child", None);
    let supervisor = WorkerSupervisor::new(temp.path().to_path_buf()).expect("supervisor");

    for route in [
        symbio::routes::ROUTE_VDFS_LIST,
        symbio::routes::ROUTE_VDFS_STAT,
        symbio::routes::ROUTE_VDFS_READ,
    ] {
        let mut read = message(route);
        read.metadata["session_id"] = json!("v2_sess_not_persisted");
        read.payload = json!({
            "path": join_addr(
                &session_node_path("child").expect("session path"),
                "messages"
            )
        });
        assert_eq!(
            supervisor.session_for(&read).expect("session read"),
            Some("child".to_string())
        );
    }
}

#[tokio::test]
async fn subagent_cancellation_is_fenced_to_its_owner() {
    let temp = tempfile::tempdir().expect("tempdir");
    let supervisor = WorkerSupervisor::new(temp.path().to_path_buf()).expect("supervisor");
    let build = BuildId::parse(format!("{:064x}", 1)).expect("build");
    let owner = EpochRegistry::new("parent", temp.path().join("parent"))
        .expect("registry")
        .allocate(build.clone())
        .expect("owner");
    let stranger = EpochRegistry::new("other", temp.path().join("other"))
        .expect("registry")
        .allocate(build)
        .expect("stranger");
    let id = RequestId::parse("subagent-1").expect("id");
    let cancel = CancellationToken::new();
    let (done_tx, done_rx) = oneshot::channel();
    supervisor.inner.subagents.lock().await.insert(
        id.clone(),
        SubagentStart {
            owner: owner.clone(),
            cancel: cancel.clone(),
            done: done_rx,
        },
    );
    drop(done_tx);

    assert!(cancel_subagent(&supervisor.inner, &stranger, &id)
        .await
        .is_err());
    assert!(!cancel.is_cancelled());
    cancel_subagent(&supervisor.inner, &owner, &id)
        .await
        .expect("owner cancellation");
    assert!(cancel.is_cancelled());
}

#[test]
fn routing_keeps_global_vdfs_calls_on_the_control_plane() {
    let temp = tempfile::tempdir().expect("tempdir");
    write_session(&temp.path().join("session/parent"), "parent", None);
    let supervisor = WorkerSupervisor::new(temp.path().to_path_buf()).expect("supervisor");

    for route in [
        symbio::routes::ROUTE_VDFS_ROOT,
        symbio::routes::ROUTE_VDFS_LIST,
    ] {
        let mut global = message(route);
        global.metadata["session_id"] = json!("v2_sess_not_persisted");
        global.payload = json!({});
        assert_eq!(supervisor.session_for(&global).expect("global route"), None);
    }

    let mut create = message(ROUTE_VDFS_WRITE);
    create
        .metadata
        .as_object_mut()
        .expect("metadata")
        .remove("session_id");
    create.payload = json!({
        "path": session_node_path("new-session").expect("session path"),
        "create": true,
        "text": "{}",
    });
    assert_eq!(supervisor.session_for(&create).expect("create route"), None);

    create.payload["path"] = json!(session_node_path("parent").expect("session path"));
    assert_eq!(
        supervisor.session_for(&create).expect("existing upsert"),
        Some("parent".to_string())
    );
}

#[test]
fn nested_session_paths_route_to_the_leaf_worker() {
    let parent = session_node_path("parent").expect("session path");
    let child = join_addr(&parent, "subsession/child");
    let mut nested = json!({ "path": join_addr(&child, "inbox/item") });
    assert_eq!(session_path_in(&nested), Some("child"));
    assert!(!session_node_in(&nested));
    assert_eq!(session_prefix_in(&nested).as_deref(), Some(child.as_str()));
    nested
        .as_object_mut()
        .expect("object")
        .insert("path".to_string(), json!(child));
    assert!(session_node_in(&nested));
}

#[tokio::test]
async fn deleting_a_session_stops_its_worker_subtree_only() {
    let temp = tempfile::tempdir().expect("tempdir");
    let parent = temp.path().join("session/parent");
    write_session(&parent, "parent", None);
    write_session(&parent.join("sessions/child"), "child", Some("parent"));
    write_session(&temp.path().join("session/other"), "other", None);
    let supervisor = WorkerSupervisor::new(temp.path().to_path_buf()).expect("supervisor");
    let build = supervisor.inner.build_id.clone();
    let mut receivers = Vec::new();
    for id in ["parent", "child", "other"] {
        let worker = EpochRegistry::new(id, temp.path().join(id))
            .expect("registry")
            .allocate(build.clone())
            .expect("worker");
        let (sender, receiver) = mpsc::channel(1);
        let done = CancellationToken::new();
        done.cancel();
        receivers.push(receiver);
        supervisor.inner.workers.lock().await.insert(
            id.to_string(),
            Arc::new(WorkerState {
                worker,
                sender,
                pending: Mutex::new(HashMap::new()),
                resync_pending: AtomicBool::new(false),
                stop: CancellationToken::new(),
                done,
            }),
        );
    }

    supervisor.stop_session_tree("parent").await;

    let workers = supervisor.inner.workers.lock().await;
    assert!(!workers.contains_key("parent"));
    assert!(!workers.contains_key("child"));
    assert!(workers.contains_key("other"));
    drop(workers);
    receivers.pop();
    for receiver in &mut receivers {
        let line = receiver.recv().await.expect("shutdown line");
        assert!(matches!(line, Outgoing::Line(WorkerLine::Shutdown { .. })));
    }
}

#[tokio::test]
async fn subagent_rejects_payload_targeting_another_session() {
    let temp = tempfile::tempdir().expect("tempdir");
    let sessions = temp.path().join("session");
    let parent_dir = sessions.join("parent");
    write_session(&parent_dir, "parent", None);
    write_session(
        &parent_dir.join("sessions").join("child"),
        "child",
        Some("parent"),
    );
    let supervisor = WorkerSupervisor::new(temp.path().to_path_buf()).expect("supervisor");
    let parent = EpochRegistry::new("parent", parent_dir)
        .expect("registry")
        .allocate(supervisor.inner.build_id.clone())
        .expect("epoch");
    let mut wrong_target = message(ROUTE_VDFS_WRITE);
    wrong_target.payload["path"] = json!("session/other/inbox");

    let error = start_subagent(
        &supervisor.inner,
        &parent,
        "child",
        wrong_target,
        &CancellationToken::new(),
        &AtomicBool::new(false),
    )
    .await
    .expect_err("must reject another payload target");
    assert!(error.contains("载荷的会话归属不一致"));
    assert!(!supervisor.inner.workers.lock().await.contains_key("child"));
}

#[tokio::test]
async fn full_event_destination_does_not_block_relay_and_receives_one_resync() {
    let temp = tempfile::tempdir().expect("tempdir");
    let supervisor = WorkerSupervisor::new(temp.path().to_path_buf()).expect("supervisor");
    let worker = EpochRegistry::new("destination", temp.path())
        .expect("registry")
        .allocate(supervisor.inner.build_id.clone())
        .expect("epoch");
    let (sender, mut receiver) = mpsc::channel(1);
    let destination = Arc::new(WorkerState {
        worker,
        sender,
        pending: Mutex::new(HashMap::new()),
        resync_pending: AtomicBool::new(false),
        stop: CancellationToken::new(),
        done: CancellationToken::new(),
    });
    destination
        .sender
        .send(Outgoing::Event {
            frame: PluginFrame::data(json!({})),
        })
        .await
        .expect("fill destination");
    let frame = PluginFrame::data(event_bus_build_envelope(
        "vdfs.changed",
        None,
        json!({ "path": "session/destination" }),
    ));

    relay_event(&destination, frame.clone());
    relay_event(&destination, frame);
    tokio::task::yield_now().await;
    let Outgoing::Event { .. } = receiver.recv().await.expect("queued event") else {
        panic!("expected queued event");
    };
    let Outgoing::Event { frame: marker } =
        tokio::time::timeout(std::time::Duration::from_secs(1), receiver.recv())
            .await
            .expect("resync timeout")
            .expect("resync")
    else {
        panic!("expected resync event");
    };
    assert_eq!(
        marker.value().expect("marker")["data"]["data"]["type"],
        EVENT_BUS_RESYNC_MARKER_TYPE
    );
    tokio::task::yield_now().await;
    assert!(!destination.resync_pending.load(Ordering::SeqCst));
    assert!(receiver.try_recv().is_err());
}

#[tokio::test]
async fn cancelled_dispatch_removes_pending_reply_and_durable_record() {
    let temp = tempfile::tempdir().expect("tempdir");
    let session_dir = temp.path().join("session");
    write_session(&session_dir, "child", None);
    let build = BuildId::parse(format!("{:064x}", 1)).expect("build");
    let worker = EpochRegistry::new("child", &session_dir)
        .expect("registry")
        .allocate(build)
        .expect("worker");
    let (sender, mut receiver) = mpsc::channel(1);
    let state = Arc::new(WorkerState {
        worker,
        sender,
        pending: Mutex::new(HashMap::new()),
        resync_pending: AtomicBool::new(false),
        stop: CancellationToken::new(),
        done: CancellationToken::new(),
    });
    state
        .sender
        .send(Outgoing::Event {
            frame: PluginFrame::data(json!({})),
        })
        .await
        .expect("fill outgoing channel");
    let id = RequestId::parse("cancelled-request").expect("id");
    let request_id = id.clone();
    let state_for_dispatch = Arc::clone(&state);
    let mut tasks = JoinSet::new();
    tasks.spawn(async move {
        deliver(
            "child",
            session_dir,
            &state_for_dispatch,
            request_id,
            message(ROUTE_VDFS_WRITE),
        )
        .await
    });
    for _ in 0..100 {
        if state.pending.lock().await.contains_key(&id) {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(state.pending.lock().await.contains_key(&id));
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}

    let inbox = DurableInbox::new("child", temp.path().join("session")).expect("inbox");
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            if state.pending.lock().await.is_empty()
                && inbox.recover().expect("recover").pending.is_empty()
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("cancelled dispatch cleanup");
    assert!(receiver.try_recv().is_ok(), "未发送的请求不得进入 worker");
}
