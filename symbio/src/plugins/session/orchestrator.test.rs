//! `orchestrator` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `orchestrator.rs` 只保留生产代码，测试全部放本文件。

use super::*;
use crate::symbio_core::VdfsChangeSubscriptions;
/// 造一个已登记中止信号的会话状态（模拟消费循环入口的登记）。
async fn armed_state() -> (Arc<ActiveSessionState>, ExecAbortSignal) {
    let state = Arc::new(ActiveSessionState::with_session_id(
        "s1".into(),
        VdfsChangeSubscriptions::default(),
    ));
    let signal = ExecAbortSignal::new();
    state.inner.write().await.abort_signal = Some(signal.clone());
    (state, signal)
}

async fn is_registered(state: &Arc<ActiveSessionState>) -> bool {
    state.inner.read().await.abort_signal.is_some()
}

/// 回归（watchdog 与 `stop_session` 竞态）：Turn 任务**提前 return** 时，
/// 中止信号登记必须随之注销。
///
/// 历史缺陷：业务错误与 1800s 消费超时两处出口写作 `return`，跳过了循环之后的
/// 清理块，留下陈旧登记。`handle_abort` 以「登记是否为 `None`」作为子任务是否
/// 仍在运行的唯一判据，判据从此永久为假 → abort 必然空等 3s 兜底。
#[tokio::test]
async fn early_return_still_unregisters_abort_signal() {
    let (state, signal) = armed_state().await;
    assert!(is_registered(&state).await, "前置：入口已登记中止信号");

    // 模拟提前出口：守卫存活期间函数直接 return（正常路径先显式 disarm）
    async fn early_return(mut guard: AbortGuard) {
        guard.disarm().await;
    }
    early_return(AbortGuard::register(state.clone(), signal).await).await;

    assert!(
        !is_registered(&state).await,
        "提前 return 后中止信号应已注销，否则 handle_abort 会空等 3s 兜底"
    );
}

/// 回归（同上，panic / 裸 return 路径）：即使出口既没 `disarm` 也没走到
/// 清理块，`Drop` 也必须兜住清理——这是"新增出口忘记清理"不再静默通过的保证。
#[tokio::test]
async fn guard_drop_unregisters_even_without_explicit_disarm() {
    let (state, signal) = armed_state().await;
    {
        let _guard = AbortGuard::register(state.clone(), signal).await;
        // 故意不调用 disarm：离开作用域应由 Drop 清理
    }
    assert!(
        !is_registered(&state).await,
        "Drop 兜底失效：登记会永久残留，abort 判据再次退化为假"
    );
}

/// 幂等性：`disarm` 之后 Drop 不得再做二次清理——否则会误伤后续轮次
/// 新登记的中止信号（消费循环与 resume 复用同一 `ActiveSessionState`）。
#[tokio::test]
async fn disarmed_guard_does_not_clobber_next_turn_registration() {
    let (state, signal) = armed_state().await;
    let mut guard = AbortGuard::register(state.clone(), signal).await;
    guard.disarm().await;

    // 模拟下一轮：新的中止信号登记进来
    let next = ExecAbortSignal::new();
    state.inner.write().await.abort_signal = Some(next.clone());

    drop(guard); // 已 disarm 的旧守卫离开作用域

    assert!(
        is_registered(&state).await,
        "旧守卫的 Drop 误清了新一轮的登记：新一轮 abort 将失去中止能力"
    );
}

/// 注销即置位：守卫离开作用域后，本 Turn 的中止信号必须已置位。
///
/// 这是收口前「消费循环 drop 掉通道 ⇒ 执行方在 `wait_for_abort_signal` 里读到
/// 通道关闭而中止」那条隐式语义的显式化。丢掉它会静默退化：看门狗/让位出口
/// 之后，卡在 await 里的 chat_loop 再也没人叫停。
#[tokio::test]
async fn disarm_aborts_the_signal() {
    let (state, signal) = armed_state().await;
    let mut guard = AbortGuard::register(state.clone(), signal.clone()).await;
    assert!(!signal.is_aborted(), "前置：尚未中止");

    guard.disarm().await;

    assert!(
        signal.is_aborted(),
        "注销必须同时置位：否则执行方永远不会感知「发起方已不再等待」"
    );
}
