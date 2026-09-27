//! `orchestrator::broadcast` 的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `broadcast.rs` 只保留生产代码，测试全部放本文件。
//!
//! 运行态结局的广播出口（`emit_session_state`）。

use super::*;
use crate::plugins::session::config::SessionConfig;
use crate::plugins::session::plugin::SessionPlugin;
use crate::plugins::session::types::Session;
use crate::symbio_core::PluginDir;
use crate::symbio_core::VdfsChangeSubscriptions;
/// 每例独占存储根。
fn fixture() -> (tempfile::TempDir, SessionPlugin) {
    let dir = tempfile::tempdir().expect("临时目录创建失败");
    let plugin = SessionPlugin::new(
        None,
        SessionConfig::default(),
        PluginDir::at(dir.path(), "session"),
    );
    (dir, plugin)
}
/// 中止与正常结束必须是**不同**的结局，且都经同一个构造点产出。
///
/// 回归：消费循环的 ABORTED 出口曾经广播 `completed`，而 `handle_abort` 广播
/// `aborted`——最终显示哪个取决于「3s 轮询」与「循环收尾」谁先跑到：窗口期内
/// UI 会短暂显示"已完成"，提示音也可能选错音色。
#[tokio::test]
async fn aborted_and_completed_outcomes_are_distinct() {
    let (_dir, p) = fixture();
    let sid = "s-outcome";
    p.get_store()
        .await
        .unwrap()
        .save_session(&Session::new(sid))
        .await
        .unwrap();
    let state = Arc::new(ActiveSessionState::with_session_id(
        sid.into(),
        VdfsChangeSubscriptions::default(),
    ));

    p.emit_session_state(&state, SessionStateChange::aborted())
        .await;
    {
        let inner = state.inner.read().await;
        assert_eq!(inner.last_outcome.as_deref(), Some(OUTCOME_ABORTED));
        assert!(!inner.is_working, "中止后必须收敛为空闲");
    }

    p.emit_session_state(&state, SessionStateChange::completed())
        .await;
    assert_eq!(
        state.inner.read().await.last_outcome.as_deref(),
        Some(OUTCOME_COMPLETED),
        "两个结局不得坍缩成同一个值——坍缩后中止会被当成正常完成"
    );
}
