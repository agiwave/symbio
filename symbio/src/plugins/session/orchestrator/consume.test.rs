//! `orchestrator::consume` 的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `consume.rs` 只保留生产代码，测试全部放本文件。
//!
//! 中止入口 `handle_abort` 的端到端契约。

use super::*;
use crate::plugins::session::config::SessionConfig;
use crate::plugins::session::plugin::SessionPlugin;
use crate::plugins::session::types::Session;
use crate::symbio_core::PluginDir;
use crate::symbio_core::VdfsChangeSubscriptions;
/// 向在途转写图注入一条消息（测试辅助：等价于旧的 live_messages.push）。
async fn push_inflight(state: &Arc<ActiveSessionState>, message: cm::ChatMessage) {
    state.transcript.lock().await.apply(message);
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

fn node(id: &str, parent: Option<&str>, status: cm::MessageStatus) -> cm::ChatMessage {
    cm::ChatMessage {
        id: id.into(),
        parent_id: parent.map(|p| p.into()),
        role: Some(cm::MessageRole::Assistant),
        msg_type: Some(cm::MessageType::Text),
        status: Some(status),
        ..Default::default()
    }
}

/// 根级 **Turn** 节点（区别于上面的普通文本节点）。
///
/// 中止的终态按「是不是根级 Turn」二分，所以测试里必须有**真的** Turn 节点——
/// 拿 `node()` 顶替会让断言测了个空（`msg_type` 是 `Text`）。
fn turn_node(id: &str, status: cm::MessageStatus) -> cm::ChatMessage {
    cm::ChatMessage {
        msg_type: Some(cm::MessageType::Turn),
        ..node(id, None, status)
    }
}
/// 中止入口的**端到端**契约（本次「通道 → 信号」改造的唯一对外行为面）。
///
/// 收口前 `handle_abort` 往执行期通道投一帧 `ControlSignal::Abort`，执行方在
/// `select!` 里收帧后自行置位标志；现在它直接置位**同一个** [`ExecAbortSignal`] 对象。
/// 外部可观察的结果必须逐条一致，本用例逐条锁定：
/// 置位信号 → 注销登记 → 复位工作态 → 结局为 `aborted` → 在途根 Turn 定稿 `Aborted`。
#[tokio::test]
async fn handle_abort_signals_registered_turn_and_converges() {
    let (_dir, p) = fixture();
    let sid = "s-abort-entry";
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
    let signal = ExecAbortSignal::new();
    {
        let mut inner = state.inner.write().await;
        inner.abort_signal = Some(signal.clone());
        inner.is_working = true;
    }
    push_inflight(&state, turn_node("turn-live", cm::MessageStatus::Streaming)).await;

    // 模拟执行方（chat_loop）收敛后由 AbortGuard 注销登记——否则 handle_abort
    // 会走满 3s 兜底（那也是合法路径，只是慢）。
    let state_for_guard = state.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        state_for_guard.inner.write().await.abort_signal = None;
    });

    p.handle_abort(&state).await;

    assert!(
        signal.is_aborted(),
        "中止必须置位在途 Turn 的信号——执行方唯一的中止感知来源"
    );
    {
        let inner = state.inner.read().await;
        assert!(
            inner.abort_signal.is_none(),
            "登记必须注销：handle_abort 凭它判定 chat_loop 已收敛"
        );
        assert!(!inner.is_working, "中止后必须收敛为空闲");
        assert_eq!(
            inner.last_outcome.as_deref(),
            Some(OUTCOME_ABORTED),
            "结局必须是 aborted：坍缩成 completed 会让前端把中止当成正常完成"
        );
    }

    let chat = p.open_chat_session(sid).await.unwrap();
    let msgs = chat.get_messages().await.unwrap();
    assert_eq!(
        msgs.iter()
            .find(|m| m.id == "turn-live")
            .and_then(|m| m.status.clone()),
        Some(cm::MessageStatus::Aborted),
        "在途根 Turn 必须定稿为 Aborted——前端的重试入口挂在它上面"
    );
}
