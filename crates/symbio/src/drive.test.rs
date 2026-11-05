//! `settled` 的语义：等空闲，不是立即返回。

use super::*;

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
