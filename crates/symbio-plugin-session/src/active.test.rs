//! 会话运行态的纯逻辑测试。
//!
//! 钉的是运行态合并规则（`begin_turn` / `finish_turn` 与 `runtime_of_state`）：
//! D-083 §5.2 的「工作态 > 进程内缓存 > 事实投影」，以及「收轮唤醒」——
//! 收件箱消费者据此取下一条，取代此前的 50ms 轮询。

use super::{runtime_of_state, ActiveSessionManager, ActiveSessionState};
use crate::plugin::{OUTCOME_ABORTED, OUTCOME_COMPLETED, OUTCOME_FAILED};
use crate::runtime::{RuntimeFacts, RuntimeOutcome};
use std::sync::Arc;
use symbio_platform::sdk::vdfs::host::VdfsChangeSubscriptions;
use tokio::sync::Barrier;
use tokio::task::JoinSet;

fn facts(outcome: Option<RuntimeOutcome>) -> RuntimeFacts {
    RuntimeFacts { outcome }
}

fn failed(why: &str) -> RuntimeFacts {
    facts(Some(RuntimeOutcome::Failed {
        error: Some(why.to_string()),
    }))
}

// ==================== 运行态合并规则（D-083 §5.2）====================
//
// 合并规则只有一条：**工作态压过一切；有进程内结局以缓存为准；否则由事实投影作答**。
// 缓存可以领先事实（中止先写内存、收束格随后落盘），所以不能让旧事实覆盖新缓存。

/// 工作态压过已定稿的事实：还在跑的时候不得同时显示"处理中"与"上次失败"。
#[tokio::test]
async fn working_masks_both_memory_and_facts() {
    let st = ActiveSessionState::with_session_id("s".into(), VdfsChangeSubscriptions::default());
    st.begin_turn().await;
    let facts = failed("上一轮的失败");
    let rt = runtime_of_state(&*st.inner.read().await, Some(&facts));
    assert!(rt.working);
    assert_eq!(rt.outcome, None);
    assert_eq!(rt.error, None);
    assert!(st.abort_current_turn().await, "开轮必须同时登记可中止信号");
}

/// 内存结局压过事实：中止在收束格落盘之前就写了缓存，读侧必须看到 `aborted`，
/// 而不是事实里上一轮的 `failed`。
#[tokio::test]
async fn memory_outcome_beats_the_fact_projection() {
    let st = ActiveSessionState::with_session_id("s".into(), VdfsChangeSubscriptions::default());
    st.finish_turn(OUTCOME_ABORTED, None).await;
    let facts = failed("上一轮的失败");
    let rt = runtime_of_state(&*st.inner.read().await, Some(&facts));
    assert!(!rt.working);
    assert_eq!(rt.outcome.as_deref(), Some(OUTCOME_ABORTED));
    assert_eq!(rt.error, None, "中止不带错误文案");
}

/// 本进程没有收过轮（重启 / 从未跑到终态）⇒ 事实就是唯一来源。
#[tokio::test]
async fn facts_answer_when_memory_has_no_outcome() {
    let mgr = ActiveSessionManager::new(VdfsChangeSubscriptions::default());
    let facts = failed("重启前的失败");
    let rt = mgr.runtime_of("s", Some(&facts)).await;
    assert!(!rt.working);
    assert_eq!(rt.outcome.as_deref(), Some(OUTCOME_FAILED));
    assert_eq!(rt.error.as_deref(), Some("重启前的失败"));
}

/// 事实也没有结局（空史 / 只开了轮）⇒ 空闲。
#[tokio::test]
async fn neither_memory_nor_facts_means_idle() {
    let mgr = ActiveSessionManager::new(VdfsChangeSubscriptions::default());
    assert!(mgr.runtime_of("s", None).await.outcome.is_none());
    let empty = facts(None);
    let rt = mgr.runtime_of("s", Some(&empty)).await;
    assert!(!rt.working);
    assert_eq!(rt.outcome, None);
}

/// 开轮清空上一轮的结局：新一轮开始后它作废。
#[tokio::test]
async fn begin_turn_clears_the_previous_round() {
    let st = ActiveSessionState::with_session_id("s".into(), VdfsChangeSubscriptions::default());
    st.finish_turn(OUTCOME_FAILED, Some("上一轮失败".into()))
        .await;
    st.begin_turn().await;

    let rt = runtime_of_state(&*st.inner.read().await, Some(&failed("事实里的失败")));
    assert!(rt.working);
    assert_eq!(rt.outcome, None);
    assert_eq!(rt.error, None);
}

// ==================== ：收轮唤醒（取代 50ms 轮询）====================
//
// 收件箱消费者此前靠 50ms 轮询兜底，因为它无法知道「哪条收尾路径把工作态复位了」。
// 现在工作态迁移是闭集：任何一次「由忙变闲」都在 `WorkingState::apply` 里被判定，
// 并由 `round_done` 通知一次。这条用例钉的正是那个契约。

/// 收轮必须叫醒等待方（收件箱消费者据此取下一条）。
#[tokio::test]
async fn finish_turn_wakes_the_waiter() {
    let st = ActiveSessionState::with_session_id("s".into(), VdfsChangeSubscriptions::default());
    st.begin_turn().await;

    let waiter = st.round_done.notified();
    st.finish_turn(OUTCOME_COMPLETED, None).await;

    tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
        .await
        .expect("收轮必须唤醒等待方——否则消费者只能靠轮询兜底");
}

#[tokio::test]
async fn concurrent_reservation_and_abort_share_one_critical_section() {
    let st = Arc::new(ActiveSessionState::with_session_id(
        "s".into(),
        VdfsChangeSubscriptions::default(),
    ));
    st.begin_turn().await;
    let barrier = Arc::new(Barrier::new(32));
    let mut tasks = JoinSet::new();
    for index in 0..32 {
        let state = Arc::clone(&st);
        let begin = async move {
            let mut inner = state.inner.write().await;
            let reserved = state.try_begin_locked(&mut inner);
            (index, reserved)
        };
        let abort_state = Arc::clone(&st);
        let abort = async move { (index, abort_state.abort_current_turn().await) };
        let barrier = Arc::clone(&barrier);
        tasks.spawn(async move {
            barrier.wait().await;
            if index % 2 == 0 {
                begin.await
            } else {
                abort.await
            }
        });
    }
    let mut results = Vec::new();
    while let Some(result) = tasks.join_next().await {
        results.push(result);
    }
    let successful_aborts = results
        .iter()
        .filter(|result| result.as_ref().expect("task").1)
        .count();
    assert!(successful_aborts >= 1, "占轮后必须能中止这一轮");
    assert_eq!(
        results
            .iter()
            .filter(|result| {
                let result = result.as_ref().expect("task");
                result.0 % 2 == 0 && result.1
            })
            .count(),
        0,
        "已在途时不得重复占轮或替换中止信号"
    );
    assert!(st
        .abort_signal
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        .expect("本轮信号")
        .is_aborted());
    st.finish_turn(OUTCOME_ABORTED, None).await;
    assert!(!st.abort_current_turn().await, "收轮后不得中止上一轮");
}
