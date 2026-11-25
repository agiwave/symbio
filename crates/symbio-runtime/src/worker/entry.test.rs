use super::*;

fn worker_args() -> Vec<String> {
    [
        "__worker",
        "--session",
        "session-1",
        "--session-dir",
        "runtime/session-1",
        "--epoch",
        "2",
        "--build",
        &"a".repeat(64),
        "--homedir",
        "runtime",
    ]
    .into_iter()
    .map(str::to_string)
    .collect()
}

#[test]
fn worker_command_validates_shared_bootstrap() {
    let argv = worker_args();
    let bootstrap = parse_worker(argv.iter().skip(1).cloned()).expect("bootstrap");
    assert_eq!(bootstrap.worker.session_id(), "session-1");
    assert_eq!(bootstrap.worker.epoch(), Epoch::new(2).unwrap());
    assert_eq!(
        bootstrap.homedir,
        std::env::current_dir().unwrap().join("runtime")
    );
    assert_eq!(
        bootstrap.session_dir,
        std::env::current_dir().unwrap().join("runtime/session-1")
    );
}

#[test]
fn worker_command_rejects_invalid_identity_and_unknown_flags() {
    let mut argv = worker_args();
    argv[2] = "../bad".to_string();
    assert!(parse_worker(argv.iter().skip(1).cloned()).is_err());
    let mut argv = worker_args();
    argv.push("--unexpected".to_string());
    assert!(parse_worker(argv.iter().skip(1).cloned()).is_err());
    argv.pop();
    argv[6] = "0".to_string();
    assert!(parse_worker(argv.iter().skip(1).cloned()).is_err());
}

#[tokio::test]
async fn public_commands_do_not_dispatch_worker() {
    assert!(
        run_worker_command(["serve"], crate::root_factory::tests::test_root_factory())
            .await
            .is_none()
    );
}
