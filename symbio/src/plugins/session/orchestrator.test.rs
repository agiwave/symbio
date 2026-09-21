//! `orchestrator` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `orchestrator.rs` 只保留生产代码，测试全部放本文件。

use super::*;
use crate::symbio_core::schemas::session::session_chat_response::NodeOp;
use crate::symbio_core::PluginChannel;

/// 向在途转写图注入一个节点（测试辅助：等价于旧的 live_messages.push）。
async fn push_inflight(state: &Arc<ActiveSessionState>, message: cm::ChatMessage) {
    state.transcript.lock().await.apply(NodeOp::Upsert {
        message: Box::new(message),
    });
}

/// 造一个已登记 `ai_control_tx` 的会话状态（模拟消费循环入口的登记）。
async fn armed_state() -> Arc<ActiveSessionState> {
    let state = Arc::new(ActiveSessionState::with_session_id("s1".into()));
    let (host_chan, _ai_chan) = PluginChannel::pair(4);
    state.inner.write().await.ai_control_tx = Some(host_chan.tx.clone());
    state
}

async fn is_registered(state: &Arc<ActiveSessionState>) -> bool {
    state.inner.read().await.ai_control_tx.is_some()
}

/// 回归（watchdog 与 `stop_session` 竞态）：消费循环**提前 return** 时，
/// 控制通道登记必须随之注销。
///
/// 历史缺陷：业务 Error 帧与 1800s 消费超时两处出口写作 `return`，跳过了
/// 循环之后的 `ai_control_tx = None` 清理，留下指向已关闭通道的陈旧 sender。
/// `handle_abort` 以「登记是否为 `None`」作为子任务是否仍在运行的唯一判据，
/// 判据从此永久为假 → abort 必然空等 3s 兜底，且 Abort 帧投进死通道被丢弃。
#[tokio::test]
async fn early_return_still_unregisters_ai_control_tx() {
    let state = armed_state().await;
    assert!(is_registered(&state).await, "前置：入口已登记控制通道");

    // 模拟提前出口：守卫存活期间函数直接 return（正常路径先显式 disarm）
    async fn early_return(mut guard: AiControlGuard) {
        guard.disarm().await;
    }
    early_return(AiControlGuard {
        state: state.clone(),
        armed: true,
    })
    .await;

    assert!(
        !is_registered(&state).await,
        "提前 return 后 ai_control_tx 应已注销，否则 handle_abort 会空等 3s 兜底"
    );
}

/// 回归（同上，panic / 裸 return 路径）：即使出口既没 `disarm` 也没走到
/// 清理块，`Drop` 也必须兜住清理——这是"新增出口忘记清理"不再静默通过的保证。
#[tokio::test]
async fn guard_drop_unregisters_even_without_explicit_disarm() {
    let state = armed_state().await;
    {
        let _guard = AiControlGuard {
            state: state.clone(),
            armed: true,
        };
        // 故意不调用 disarm：离开作用域应由 Drop 清理
    }
    assert!(
        !is_registered(&state).await,
        "Drop 兜底失效：登记会永久残留，abort 判据再次退化为假"
    );
}

/// 幂等性：`disarm` 之后 Drop 不得再做二次清理——否则会误伤后续轮次
/// 新登记的控制通道（消费循环与 resume 复用同一 `ActiveSessionState`）。
#[tokio::test]
async fn disarmed_guard_does_not_clobber_next_turn_registration() {
    let state = armed_state().await;
    let mut guard = AiControlGuard {
        state: state.clone(),
        armed: true,
    };
    guard.disarm().await;

    // 模拟下一轮：新的控制通道登记进来
    let (host_chan, _ai_chan) = PluginChannel::pair(4);
    state.inner.write().await.ai_control_tx = Some(host_chan.tx.clone());

    drop(guard); // 已 disarm 的旧守卫离开作用域

    assert!(
        is_registered(&state).await,
        "旧守卫的 Drop 误清了新一轮的登记：新一轮 abort 将失去中止能力"
    );
}

// ==================== 中止收口（converge_inflight） ====================

use crate::plugins::session::config::SessionConfig;
use crate::plugins::session::plugin::SessionPlugin;
use crate::plugins::session::types::Session;
use crate::symbio_core::PluginDir;

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
    use super::failure::is_inflight;
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
    use super::failure::abort_terminal_of;
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
    let state = Arc::new(ActiveSessionState::with_session_id(sid.into()));
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
        state.transcript.lock().await.is_empty(),
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
    let state = Arc::new(ActiveSessionState::with_session_id(sid.into()));
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

    let state = Arc::new(ActiveSessionState::with_session_id(sid.into()));
    push_inflight(&state, node("turn", None, cm::MessageStatus::Streaming)).await;
    assert_eq!(p.converge_inflight(&state, sid, "用户中止").await, 1);
    assert_eq!(
        p.converge_inflight(&state, sid, "用户中止").await,
        0,
        "第二次必须是空操作：否则每次中止都会对同一批节点重复广播"
    );
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
    let state = Arc::new(ActiveSessionState::with_session_id(sid.into()));

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
