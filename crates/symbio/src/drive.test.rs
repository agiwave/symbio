//! `settled` 的语义：等空闲，不是立即返回。

use super::*;
use symbio_plugin_api::drive::DriveKey;

/// `settled` 是「等」不是「立即返回」：跑着时挂起，空闲后由通知唤醒。
#[tokio::test]
async fn settled_waits_for_idle() {
    let state = Arc::new(KickState {
        running: AtomicBool::new(true),
        dirty: AtomicBool::new(false),
        idle: tokio::sync::Notify::new(),
    });

    let waiting = tokio::spawn({
        let state = Arc::clone(&state);
        async move { wait_idle(state).await }
    });
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    assert!(!waiting.is_finished(), "驱动还在跑时 settled 不该返回");

    state.running.store(false, Ordering::Release);
    state.idle.notify_waiters();
    tokio::time::timeout(std::time::Duration::from_secs(1), waiting)
        .await
        .expect("空闲后应被唤醒")
        .unwrap();
}

fn key() -> DriveKey {
    DriveKey {
        port: "p".into(),
        id: "x".into(),
        attempt: 1,
    }
}

#[test]
fn exhausted_and_stalled_are_both_visible() {
    let exhausted = drive_notice(
        "s",
        &DriveOutcome::Exhausted {
            steps: 128,
            outstanding: vec![key()],
        },
    )
    .expect("超预算必须出声");
    assert!(exhausted.contains("步数预算用尽"), "{exhausted}");

    let stalled =
        drive_notice("s", &DriveOutcome::Stalled(vec![key()])).expect("零进展必须出声");
    assert!(stalled.contains("一圈没有新增事实"), "{stalled}");

    assert!(drive_notice("s", &DriveOutcome::NoDebt).is_none());
    assert!(drive_notice("s", &DriveOutcome::Waiting(vec![])).is_none());
}
