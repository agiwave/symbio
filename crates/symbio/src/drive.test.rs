//! `settled` 的语义：等空闲，不是立即返回。

use super::*;
use symbio_plugin_api::drive::DriveKey;

/// `settled` 是「等」不是「立即返回」：跑着时挂起，空闲后由通知唤醒。
#[tokio::test]
async fn settled_waits_for_idle() {
    let state = Arc::new(KickState {
        running: AtomicBool::new(true),
        dirty: AtomicBool::new(false),
        in_flight: AtomicBool::new(false),
        latest: Mutex::new(None),
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

/// 在飞 ≠ 空闲：驱动循环已退出（`running=false`），但还有一笔已派发、应答未落格的债。
/// 此刻 `settled` 必须继续等——否则调用方会在空档里投影，记忆巩固只看到本轮一半的
/// 编码（t29 的早投影）。
#[tokio::test]
async fn settled_does_not_wake_while_a_port_is_in_flight() {
    let state = Arc::new(KickState {
        running: AtomicBool::new(false),
        dirty: AtomicBool::new(false),
        in_flight: AtomicBool::new(true),
        latest: Mutex::new(None),
        idle: tokio::sync::Notify::new(),
    });

    let waiting = tokio::spawn({
        let state = Arc::clone(&state);
        async move { wait_idle(state).await }
    });
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(
        !waiting.is_finished(),
        "还有在飞的债时 settled 不该返回（会话未收口）"
    );

    // 应答落格：驱动跑完这一轮，出口不再是 `Waiting`，这才会叫醒。
    state.in_flight.store(false, Ordering::Release);
    state.idle.notify_waiters();
    tokio::time::timeout(std::time::Duration::from_secs(1), waiting)
        .await
        .expect("收口后应被唤醒")
        .unwrap();
}

/// 等外部事实（`NeedsHuman`）是交还时刻：没有在飞的债，`settled` 立即返回。
#[tokio::test]
async fn settled_returns_when_only_human_can_advance() {
    let state = Arc::new(KickState {
        running: AtomicBool::new(false),
        dirty: AtomicBool::new(false),
        in_flight: AtomicBool::new(false),
        latest: Mutex::new(None),
        idle: tokio::sync::Notify::new(),
    });

    tokio::time::timeout(std::time::Duration::from_secs(1), wait_idle(state))
        .await
        .expect("NeedsHuman 出口必须叫醒调用方");
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

    let stalled = drive_notice("s", &DriveOutcome::Stalled(vec![key()])).expect("零进展必须出声");
    assert!(stalled.contains("一圈没有新增事实"), "{stalled}");

    assert!(drive_notice("s", &DriveOutcome::NoDebt).is_none());
    assert!(drive_notice("s", &DriveOutcome::Waiting(vec![])).is_none());
    // 交还人工是正常出口，不是停摆：不出声。
    assert!(drive_notice("s", &DriveOutcome::NeedsHuman(vec![key()])).is_none());
}
