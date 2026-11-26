//! `orchestrator::broadcast` 的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `broadcast.rs` 只保留生产代码，测试全部放本文件。
//!
//! 运行态结局的广播出口（`emit_session_state`）。

use super::*;
use crate::config::SessionConfig;
use crate::plugin::{SessionPlugin, OUTCOME_ABORTED};
use crate::plugin::{OUTCOME_COMPLETED, OUTCOME_FAILED};
use crate::types::Session;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex as StdMutex};
use symbio_platform::sdk::vdfs::host::VdfsChangeSubscriptions;
use symbio_plugin_api::PluginDir;
use tokio::sync::{oneshot, Notify};
use tokio::task::JoinSet;
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
    event.seq = Some(symbio_platform::kernel::event::Seq::new(seq));
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

/// 批准结论与答复同权：批准之后不再算「停在卡片上」。
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn finished_frame_precedes_any_reserved_round() {
    let (_dir, p) = fixture();
    let sid = "s-order";
    p.get_store()
        .await
        .unwrap()
        .save_session(&Session::new(sid))
        .await
        .unwrap();
    let changes = VdfsChangeSubscriptions::default();
    let state = Arc::new(ActiveSessionState::with_session_id(
        sid.into(),
        changes.clone(),
    ));
    p.convergence
        .active
        .sessions
        .write()
        .await
        .insert(sid.into(), Arc::clone(&state));
    state.begin_turn().await;
    let observed = Arc::new(StdMutex::new(Vec::new()));
    let sink_observed = Arc::clone(&observed);
    let entered = Arc::new(Notify::new());
    let sink_entered = Arc::clone(&entered);
    let release = Arc::new(Notify::new());
    let sink_release = Arc::clone(&release);
    let blocker = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let sink_block = Arc::clone(&blocker);
    changes.watch(
        sid,
        Arc::new(move |change| {
            sink_observed
                .lock()
                .expect("observed")
                .push(change.data.expect("node"));
            sink_entered.notify_one();
            if sink_block.swap(false, Ordering::SeqCst) {
                futures::executor::block_on(sink_release.notified());
            }
        }),
    );

    let mut tasks = JoinSet::new();
    let finish_state = Arc::clone(&state);
    let convergence = p.convergence.clone();
    tasks.spawn(async move {
        convergence
            .emit_session_state(
                &finish_state,
                SessionStateChange::Finished {
                    outcome: OUTCOME_FAILED,
                    error: Some("失败".into()),
                },
            )
            .await;
        "finished"
    });
    tokio::time::timeout(std::time::Duration::from_secs(1), entered.notified())
        .await
        .expect("finish must reach the blocking VDFS sink");
    assert_eq!(observed.lock().expect("observed").len(), 1);
    assert!(
        state.inner.read().await.is_working(),
        "终态帧投递完成前不得释放轮次"
    );

    let (next_tx, next_rx) = oneshot::channel();
    let next_state = Arc::clone(&state);
    tasks.spawn(async move {
        next_state.begin_when_idle().await;
        let _ = next_tx.send(());
        "next"
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(20), next_rx)
            .await
            .is_err(),
        "下一轮必须等待终态投递"
    );

    release.notify_one();
    let mut labels = Vec::new();
    while let Some(result) =
        tokio::time::timeout(std::time::Duration::from_secs(1), tasks.join_next())
            .await
            .expect("round transition timeout")
    {
        labels.push(result.expect("transition task"));
    }
    assert_eq!(labels, ["finished", "next"]);

    let convergence = p.convergence.clone();
    let working_state = Arc::clone(&state);
    tasks.spawn(async move {
        convergence
            .emit_session_state(&working_state, SessionStateChange::Working)
            .await;
        "working"
    });
    while let Some(result) =
        tokio::time::timeout(std::time::Duration::from_secs(1), tasks.join_next())
            .await
            .expect("working transition timeout")
    {
        labels.push(result.expect("working task"));
    }
    assert_eq!(labels, ["finished", "next", "working"]);
    state.finish_turn(OUTCOME_COMPLETED, None).await;
    let frames = observed.lock().expect("observed");
    let statuses: Vec<_> = frames
        .iter()
        .map(|frame| frame["status"].as_str().expect("status"))
        .collect();
    assert_eq!(statuses, ["failed", "working"]);
}
