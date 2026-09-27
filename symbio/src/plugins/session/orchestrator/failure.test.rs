//! `orchestrator::failure` 的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `failure.rs` 只保留生产代码，测试全部放本文件。
//!
//! 中止/失败收口的判据与收敛（`is_inflight` / `abort_terminal_of` / `converge_inflight`）。

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
// ==================== 中止收口（converge_inflight） ====================
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

/// 「在途」集合的**唯一**定义，直接锁定。
///
/// 少一个状态 = 前端一个永远转下去的「运行中」——它不报错、只是一直转，
/// 因此必须有测试盯着这个集合，而不是靠读代码。
#[test]
fn inflight_set_covers_pending_and_streaming_but_not_waiting_user_action() {
    use super::is_inflight;
    assert!(
        is_inflight(&None),
        "未标注状态视为在途（防御：在途缓冲不应有无状态节点）"
    );
    assert!(is_inflight(&Some(cm::MessageStatus::Pending)));
    assert!(is_inflight(&Some(cm::MessageStatus::Streaming)));
    assert!(
        !is_inflight(&Some(cm::MessageStatus::WaitingUserAction)),
        "等待用户回答不是「正在跑」：中止不该把审批入口一并抹掉"
    );
    assert!(!is_inflight(&Some(cm::MessageStatus::Completed)));
    assert!(!is_inflight(&Some(cm::MessageStatus::Failed)));
    assert!(
        !is_inflight(&Some(cm::MessageStatus::Aborted)),
        "已中止是终态：否则中止收口会把它又当在途节点收敛一遍"
    );
}

/// 中止的终态映射：根级 Turn → `Aborted`，其余 → `Completed`。
///
/// `Completed` 在这里是**错的**：它宣布这一轮正常结束，而这一轮是半截的；
/// 更直接的后果是前端的重试入口随之消失——用户明明可以按「重试」重跑。
#[test]
fn abort_terminal_of_marks_only_root_turn_aborted() {
    use super::abort_terminal_of;
    assert_eq!(
        abort_terminal_of(&turn_node("t", cm::MessageStatus::Streaming)),
        cm::MessageStatus::Aborted
    );
    // 子节点：正文 / 思考 / 工具调用都没有独立的重试语义，定稿结束动画即可
    assert_eq!(
        abort_terminal_of(&node("c", Some("t"), cm::MessageStatus::Streaming)),
        cm::MessageStatus::Completed
    );
    // 有父的 Turn 是子会话节点，重试归父会话的根 Turn
    assert_eq!(
        abort_terminal_of(&cm::ChatMessage {
            msg_type: Some(cm::MessageType::Turn),
            ..node("sub", Some("t"), cm::MessageStatus::Streaming)
        }),
        cm::MessageStatus::Completed
    );
}

/// 中止收口收敛**在途缓冲**，且不得惊扰存储里的终态消息。
///
/// 存储由写入不变量保证只有终态（`ensure_durable_states` 拒绝 `Streaming`
/// 落盘），因此收口对象只有尚未落库的在途节点。存储侧同时在场，验证的是
/// 「补写进存储时已有消息原样保留 + 父节点存在性判据」。
#[tokio::test]
async fn converge_inflight_finalizes_live_buffer_nodes() {
    let (_dir, p) = fixture();
    let sid = "s-abort";
    let store = p.get_store().await.expect("存储不可用");
    store.save_session(&Session::new(sid)).await.unwrap();

    // 存储侧：全部终态（上一轮已收口的世界）
    let chat = p.open_chat_session(sid).await.unwrap();
    chat.replace_messages(vec![node("turn", None, cm::MessageStatus::Completed)])
        .await
        .unwrap();

    // 在途侧：Turn 下未落库的 reasoning(Streaming) 与一个父不存在的孤儿
    let state = Arc::new(ActiveSessionState::with_session_id(
        sid.into(),
        VdfsChangeSubscriptions::default(),
    ));
    push_inflight(
        &state,
        node("reason", Some("turn"), cm::MessageStatus::Streaming),
    )
    .await;
    push_inflight(
        &state,
        node("orphan", Some("ghost-parent"), cm::MessageStatus::Streaming),
    )
    .await;

    let converged = p.converge_inflight(&state, sid, "用户中止").await;
    assert_eq!(converged, 1, "在途侧 1 条（孤儿被丢弃；存储侧无在途节点）");

    let after = chat.get_messages().await.unwrap();
    let status_of = |id: &str| {
        after
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.status.clone())
            .unwrap_or(None)
    };
    assert_eq!(
        status_of("turn"),
        Some(cm::MessageStatus::Completed),
        "存储里的终态消息不得被收口改写"
    );
    assert_eq!(
        status_of("reason"),
        Some(cm::MessageStatus::Completed),
        "在途缓冲里的节点必须被补写为终态"
    );
    assert_eq!(status_of("orphan"), None, "父节点不存在的孤儿不得写入存储");

    assert!(
        state.transcript.lock().await.snapshot().is_empty(),
        "权威副本已回到存储，在途缓冲必须作废——否则陈旧副本会继续参与叠加"
    );
}

/// 中止后根级 Turn 必须是 `Aborted`，**不能**是 `Completed`。
///
/// 这是「能不能重试」的分界：前端的重试入口挂在 Turn 的终态上，`Completed` 会让它
/// 彻底消失。这条测试盯的是**端到端**结果（走完整收口），与上面只测映射函数的
/// `abort_terminal_of_marks_only_root_turn_aborted` 互为补充。
/// 场景：流式首帧刚到、还没写盘——Turn 与其子节点都只在在途缓冲里。
#[tokio::test]
async fn converge_inflight_marks_root_turn_aborted_and_children_completed() {
    let (_dir, p) = fixture();
    let sid = "s-abort-turn";
    let store = p.get_store().await.expect("存储不可用");
    store.save_session(&Session::new(sid)).await.unwrap();

    // 在途缓冲：根 Turn（未落库）+ 其子节点。子节点晚于父节点入列——
    // 父存在性判据按序检查，与真实广播顺序一致。
    let state = Arc::new(ActiveSessionState::with_session_id(
        sid.into(),
        VdfsChangeSubscriptions::default(),
    ));
    push_inflight(&state, turn_node("turn-live", cm::MessageStatus::Streaming)).await;
    push_inflight(
        &state,
        node("reason", Some("turn-live"), cm::MessageStatus::Streaming),
    )
    .await;

    p.converge_inflight(&state, sid, "用户中止").await;

    let chat = p.open_chat_session(sid).await.unwrap();
    let after = chat.get_messages().await.unwrap();
    let status_of = |id: &str| {
        after
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.status.clone())
            .unwrap_or(None)
    };
    assert_eq!(
        status_of("turn-live"),
        Some(cm::MessageStatus::Aborted),
        "被中止的这一轮是半截的，不得宣布它正常结束"
    );
    assert_eq!(status_of("reason"), Some(cm::MessageStatus::Completed));
    assert_eq!(
        after
            .iter()
            .find(|m| m.id == "turn-live")
            .and_then(|m| m.error.clone()),
        None,
        "中止不挂 error 文案：它是用户自己的操作，不是故障"
    );
}

/// 幂等：已经收口过的会话再收一次不得产生任何变更（`handle_abort` 与消费循环的
/// ABORTED 分支都会调用它，两者都可能先跑）。
#[tokio::test]
async fn converge_inflight_is_idempotent() {
    let (_dir, p) = fixture();
    let sid = "s-idem";
    let store = p.get_store().await.expect("存储不可用");
    store.save_session(&Session::new(sid)).await.unwrap();

    let state = Arc::new(ActiveSessionState::with_session_id(
        sid.into(),
        VdfsChangeSubscriptions::default(),
    ));
    push_inflight(&state, node("turn", None, cm::MessageStatus::Streaming)).await;
    assert_eq!(p.converge_inflight(&state, sid, "用户中止").await, 1);
    assert_eq!(
        p.converge_inflight(&state, sid, "用户中止").await,
        0,
        "第二次必须是空操作：否则每次中止都会对同一批节点重复广播"
    );
}
