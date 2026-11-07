//! `orchestrator::broadcast` 的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `broadcast.rs` 只保留生产代码，测试全部放本文件。
//!
//! 运行态结局的广播出口（`emit_session_state`）。

use super::*;
use crate::config::SessionConfig;
use crate::plugin::{SessionPlugin, OUTCOME_ABORTED, OUTCOME_COMPLETED};
use crate::types::Session;
use symbio_sdk::plugin::PluginDir;
use symbio_sdk::vdfs::host::VdfsChangeSubscriptions;
/// 造一条已入库的事件（只给 `waits_for_user` 用：kind / payload / seq）。
fn at(seq: u64, kind: &str, payload: serde_json::Value) -> Event {
    let mut event = Event::pending(
        format!("e-{seq}"),
        kind,
        Entity::Scope,
        Verb::Asserted,
        "session/main",
    );
    event.payload = payload;
    event.seq = Some(symbio_kernel::event::Seq::new(seq));
    event
}

fn card(seq: u64, call_id: &str) -> Event {
    at(
        seq,
        EVENT_ARTIFACT_ADDED,
        serde_json::json!({ "needs_user_action": true, "tool_call_id": call_id }),
    )
}

fn answer(seq: u64, call_id: &str) -> Event {
    at(
        seq,
        EVENT_USER_ANSWER,
        serde_json::json!({ "call_id": call_id, "kind": "approve" }),
    )
}

fn granted(seq: u64, call_id: &str) -> Event {
    at(
        seq,
        EVENT_AUTHZ_GRANTED,
        serde_json::json!({ "id": call_id, "basis": "approved" }),
    )
}

/// 卡在、答未到 ⇒ 仍在等。
#[test]
fn card_without_decision_keeps_waiting() {
    assert!(waits_for_user(&[card(1, "c1")]));
}

/// 答复在卡**之后** ⇒ 已答；答复在卡之前（上一轮）⇒ 仍在等。
#[test]
fn answer_resolves_only_when_it_follows_the_card() {
    assert!(!waits_for_user(&[card(1, "c1"), answer(2, "c1")]));
    assert!(waits_for_user(&[answer(1, "c1"), card(2, "c1")]));
}

/// 批准结论与答复同权（D-368）：批准之后不再算「停在卡片上」。
#[test]
fn approved_card_is_resolved() {
    assert!(!waits_for_user(&[card(1, "c1"), granted(2, "c1")]));
}

/// 别的调用的答复不能替这张卡结账。
#[test]
fn answer_for_another_call_does_not_resolve() {
    assert!(waits_for_user(&[card(1, "c1"), answer(2, "c2")]));
}

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
/// 中止与正常结束必须是**不同**的结局，且都经同一个出口产出。
///
/// 回归：两处置不同的结局会让 UI 在窗口期内显示"已完成"，提示音也可能选错音色。
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

    p.convergence
        .emit_session_state(
            &state,
            SessionStateChange::Finished {
                outcome: OUTCOME_ABORTED,
                error: None,
            },
        )
        .await;
    {
        let inner = state.inner.read().await;
        assert_eq!(inner.last_outcome.as_deref(), Some(OUTCOME_ABORTED));
        assert!(!inner.is_working(), "中止后必须收敛为空闲");
    }

    p.convergence
        .emit_session_state(
            &state,
            SessionStateChange::Finished {
                outcome: OUTCOME_COMPLETED,
                error: None,
            },
        )
        .await;
    assert_eq!(
        state.inner.read().await.last_outcome.as_deref(),
        Some(OUTCOME_COMPLETED),
        "两个结局不得坍缩成同一个值——坍缩后中止会被当成正常完成"
    );
}
