//! 自动压缩熔断状态机的单元测试。
//!
//! 这些是不依赖 LLM / 存储的纯状态机测试——熔断逻辑被刻意收在 `ActiveSessionState`
//! 的三个方法里（`compression_should_skip` / `record_success` / `record_failure`），
//! 这样它的行为可以孤立验证，而不必驱动一整条压缩流水线。
//!
//! 回归动机：实测会话 `09d74431` 在运行过程中**反复**自动压缩失败，但旧代码既不记录
//! 原因、也不停止重试——每轮用户消息都白等一次数分钟的注定失败请求。熔断要解决的
//! 就是「反复失败不收敛」。其中「历史超限」（`input_over_limit`）是**永久失败**：
//! 历史只增不减，半开重试注定复现，故单独用永久标志跳过（见 `record_failure`）。

use super::ActiveSessionState;
use crate::symbio_core::ChangeSubscriptions;

#[tokio::test]
async fn skip_is_false_until_threshold_reached() {
    let st = ActiveSessionState::with_session_id("s".into(), ChangeSubscriptions::default());
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
    let st = ActiveSessionState::with_session_id("s".into(), ChangeSubscriptions::default());
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
    let st = ActiveSessionState::with_session_id("s".into(), ChangeSubscriptions::default());
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
    let st = ActiveSessionState::with_session_id("s".into(), ChangeSubscriptions::default());
    st.compression_record_failure(true).await;
    assert!(
        st.compression_should_skip().await,
        "历史超限是永久失败，第一次就应跳过"
    );
}

/// 永久失败只能被「成功」清除——模拟用户清理历史后压缩重新可行。
#[tokio::test]
async fn permanent_failure_is_cleared_by_success_only() {
    let st = ActiveSessionState::with_session_id("s".into(), ChangeSubscriptions::default());
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
