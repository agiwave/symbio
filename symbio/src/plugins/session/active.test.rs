//! 会话运行态的两组纯逻辑测试。
//!
//! - 自动压缩熔断状态机（`compression_should_skip` / `record_success` /
//!   `record_failure`）：刻意收在三个方法里，不依赖 LLM / 存储；
//! - 运行态合并规则（`begin_turn` / `finish_turn` / `set_warning` 与
//!   `runtime_of_state`）：钉住 ADR-083 §5.2 的「工作态 > 进程内缓存 > 事实投影」。
//!
//! 回归动机：实测会话 `09d74431` 在运行过程中**反复**自动压缩失败，但旧代码既不记录
//! 原因、也不停止重试——每轮用户消息都白等一次数分钟的注定失败请求。熔断要解决的
//! 就是「反复失败不收敛」。其中「历史超限」（`input_over_limit`）是**永久失败**：
//! 历史只增不减，半开重试注定复现，故单独用永久标志跳过（见 `record_failure`）。

use super::{runtime_of_state, ActiveSessionManager, ActiveSessionState};
use crate::plugins::session::plugin::{OUTCOME_ABORTED, OUTCOME_COMPLETED, OUTCOME_FAILED};
use crate::plugins::session::runtime::{RuntimeFacts, RuntimeOutcome};
use crate::symbio_core::VdfsChangeSubscriptions;

fn facts(outcome: Option<RuntimeOutcome>) -> RuntimeFacts {
    RuntimeFacts { outcome }
}

fn failed(why: &str) -> RuntimeFacts {
    facts(Some(RuntimeOutcome::Failed {
        error: Some(why.to_string()),
    }))
}

#[tokio::test]
async fn skip_is_false_until_threshold_reached() {
    let st = ActiveSessionState::with_session_id("s".into(), VdfsChangeSubscriptions::default());
    // 阈值前：不跳过，仍尝试
    assert!(!st.compression_should_skip().await);
    st.compression_record_failure(false).await;
    assert!(!st.compression_should_skip().await);
    st.compression_record_failure(false).await;
    assert!(!st.compression_should_skip().await);
    // 第 3 次（达到阈值）：开闸，开始跳过
    st.compression_record_failure(false).await;
    assert!(st.compression_should_skip().await);
}

#[tokio::test]
async fn success_resets_the_counter() {
    let st = ActiveSessionState::with_session_id("s".into(), VdfsChangeSubscriptions::default());
    st.compression_record_failure(false).await;
    st.compression_record_failure(false).await;
    st.compression_record_failure(false).await;
    assert!(st.compression_should_skip().await, "已达阈值应跳过");
    // 任一次压缩成功清零计数
    st.compression_record_success().await;
    assert!(
        !st.compression_should_skip().await,
        "成功后必须恢复到「不跳过」"
    );
}

#[tokio::test]
async fn cooldown_is_observed_after_open() {
    let st = ActiveSessionState::with_session_id("s".into(), VdfsChangeSubscriptions::default());
    // 推到开闸
    st.compression_record_failure(false).await;
    st.compression_record_failure(false).await;
    st.compression_record_failure(false).await;
    assert!(st.compression_should_skip().await);

    // 冷却期内再次失败：不得刷新开闸时刻（否则每次失败都重置冷却 → 永不重试）
    st.compression_record_failure(false).await;
    // 冷却未到：仍跳过
    assert!(st.compression_should_skip().await);

    // 模拟冷却结束：直接把开闸时刻推到过去（用 record_success 清零再重开不可行，
    // 这里改为探测「冷却后放行一次」——通过把当前时刻与开闸时刻拉开）。
    // 由于 opened_at 是 Instant，无法在测试里快进时间，故只验证「计数达标即跳过」
    // 这一不变量，冷却时长由常量保证、由集成路径覆盖。
    let inner = st.inner.read().await;
    assert!(inner.auto_compress_circuit_opened_at.is_some());
}

/// 永久失败（历史超限）第一次就跳过，且不等阈值/冷却——重试注定复现。
#[tokio::test]
async fn permanent_failure_skips_immediately() {
    let st = ActiveSessionState::with_session_id("s".into(), VdfsChangeSubscriptions::default());
    st.compression_record_failure(true).await;
    assert!(
        st.compression_should_skip().await,
        "历史超限是永久失败，第一次就应跳过"
    );
}

/// 永久失败只能被「成功」清除——模拟用户清理历史后压缩重新可行。
#[tokio::test]
async fn permanent_failure_is_cleared_by_success_only() {
    let st = ActiveSessionState::with_session_id("s".into(), VdfsChangeSubscriptions::default());
    st.compression_record_failure(true).await;
    // 再记一次瞬时失败：不得解除永久跳过
    st.compression_record_failure(false).await;
    assert!(st.compression_should_skip().await);

    st.compression_record_success().await;
    assert!(
        !st.compression_should_skip().await,
        "成功后永久标志必须清除"
    );
    assert!(
        !st.inner.read().await.auto_compress_over_limit,
        "标志本体也应复位"
    );
}

// ==================== 运行态合并规则（ADR-083 §5.2）====================
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

/// 告警是宿主局部量：无论结局来自缓存还是事实，都原样带上。
#[tokio::test]
async fn warning_survives_the_merge() {
    let st = ActiveSessionState::with_session_id("s".into(), VdfsChangeSubscriptions::default());
    st.set_warning(Some("持久化失败".into())).await;
    let from_memory = runtime_of_state(&*st.inner.read().await, None);
    assert_eq!(from_memory.warning.as_deref(), Some("持久化失败"));

    let from_facts = runtime_of_state(
        &*st.inner.read().await,
        Some(&facts(Some(RuntimeOutcome::Completed))),
    );
    assert_eq!(from_facts.warning.as_deref(), Some("持久化失败"));
    assert_eq!(from_facts.outcome.as_deref(), Some(OUTCOME_COMPLETED));
}

/// 开轮清空上一轮的结局与告警：新一轮开始后它们一律作废。
#[tokio::test]
async fn begin_turn_clears_the_previous_round_and_warning() {
    let st = ActiveSessionState::with_session_id("s".into(), VdfsChangeSubscriptions::default());
    st.finish_turn(OUTCOME_FAILED, Some("上一轮失败".into()))
        .await;
    st.set_warning(Some("告警".into())).await;
    st.begin_turn().await;

    let rt = runtime_of_state(&*st.inner.read().await, Some(&failed("事实里的失败")));
    assert!(rt.working);
    assert_eq!(rt.outcome, None);
    assert_eq!(rt.error, None);
    assert_eq!(rt.warning, None);
}

// ==================== T-191：收轮唤醒（取代 50ms 轮询）====================
//
// 收件箱消费者此前靠 50ms 轮询兜底，因为它无法知道「哪条收尾路径把工作态复位了」。
// 现在工作态迁移是闭集：任何一次「由忙变闲」都在 `WorkingState::apply` 里被判定，
// 并由 `turn_done` 通知一次。这三条用例钉的正是那个契约。

/// 收轮必须叫醒等待方（收件箱消费者据此取下一条）。
#[tokio::test]
async fn finish_turn_wakes_the_waiter() {
    let st = ActiveSessionState::with_session_id("s".into(), VdfsChangeSubscriptions::default());
    st.begin_turn().await;

    let waiter = st.turn_done.notified();
    st.finish_turn(OUTCOME_COMPLETED, None).await;

    tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
        .await
        .expect("收轮必须唤醒等待方——否则消费者只能靠轮询兜底");
}

/// 提前让位（中止 / 正常收尾的前置复位）同样是一次「由忙变闲」，必须叫醒。
#[tokio::test]
async fn early_reset_wakes_the_waiter() {
    let st = ActiveSessionState::with_session_id("s".into(), VdfsChangeSubscriptions::default());
    st.begin_turn().await;

    let waiter = st.turn_done.notified();
    st.reset_working().await;

    tokio::time::timeout(std::time::Duration::from_secs(1), waiter)
        .await
        .expect("提前让位必须唤醒等待方");
}

/// 一次收轮只唤醒一次：提前让位之后才到的「收尾完成」不得再留一个许可。
///
/// 伪唤醒本身不会算错，但它会让「一次收轮 = 一次唤醒」失去意义——而消费者的
/// 正确性正是挂在这条等式上（多醒一次只是空转，少醒一次就是消息永远不发）。
#[tokio::test]
async fn a_single_handover_wakes_exactly_once() {
    let st = ActiveSessionState::with_session_id("s".into(), VdfsChangeSubscriptions::default());
    st.begin_turn().await;
    st.reset_working().await;

    tokio::time::timeout(
        std::time::Duration::from_millis(200),
        st.turn_done.notified(),
    )
    .await
    .expect("第一次等待应立刻拿到提前让位的许可");

    st.finish_turn(OUTCOME_ABORTED, None).await;
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(50),
            st.turn_done.notified()
        )
        .await
        .is_err(),
        "收尾完成时已经空闲：不得再制造第二次唤醒"
    );
}

/// 空闲时的复位是空操作：不许伪造唤醒（否则消费者会被自己叫醒空转）。
#[tokio::test]
async fn idle_reset_never_fakes_a_wakeup() {
    let st = ActiveSessionState::with_session_id("s".into(), VdfsChangeSubscriptions::default());

    st.reset_working().await;
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(50),
            st.turn_done.notified()
        )
        .await
        .is_err(),
        "空闲时复位不得唤醒任何人"
    );
}
