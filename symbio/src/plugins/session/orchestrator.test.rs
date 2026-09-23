//! `orchestrator` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `orchestrator.rs` 只保留生产代码，测试全部放本文件。

use super::*;
use crate::symbio_core::vdfs::ChangeSubscriptions;

/// 向在途转写图注入一条消息（测试辅助：等价于旧的 live_messages.push）。
async fn push_inflight(state: &Arc<ActiveSessionState>, message: cm::ChatMessage) {
    state.transcript.lock().await.apply(message);
}

/// 造一个已登记中止信号的会话状态（模拟消费循环入口的登记）。
async fn armed_state() -> (Arc<ActiveSessionState>, AbortSignal) {
    let state = Arc::new(ActiveSessionState::with_session_id(
        "s1".into(),
        ChangeSubscriptions::default(),
    ));
    let signal = AbortSignal::new();
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
    let next = AbortSignal::new();
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
    let state = Arc::new(ActiveSessionState::with_session_id(
        sid.into(),
        ChangeSubscriptions::default(),
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
        ChangeSubscriptions::default(),
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
        ChangeSubscriptions::default(),
    ));
    push_inflight(&state, node("turn", None, cm::MessageStatus::Streaming)).await;
    assert_eq!(p.converge_inflight(&state, sid, "用户中止").await, 1);
    assert_eq!(
        p.converge_inflight(&state, sid, "用户中止").await,
        0,
        "第二次必须是空操作：否则每次中止都会对同一批节点重复广播"
    );
}

/// 中止入口的**端到端**契约（本次「通道 → 信号」改造的唯一对外行为面）。
///
/// 收口前 `handle_abort` 往执行期通道投一帧 `ControlSignal::Abort`，执行方在
/// `select!` 里收帧后自行置位标志；现在它直接置位**同一个** [`AbortSignal`] 对象。
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
        ChangeSubscriptions::default(),
    ));
    let signal = AbortSignal::new();
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
        ChangeSubscriptions::default(),
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
