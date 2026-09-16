//! `orchestrator` 模块的单元测试。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`，见 `CONTRIBUTING.md`）：
//! `orchestrator.rs` 只保留生产代码，测试全部放本文件。

use super::*;
use crate::symbio_core::PluginChannel;

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

// ==================== 合并语义 ↔ 变更语义（不得漂移） ====================

fn text_msg(id: &str, content: &str, status: cm::MessageStatus) -> cm::ChatMessage {
    cm::ChatMessage {
        id: id.into(),
        role: Some(cm::MessageRole::Assistant),
        msg_type: Some(cm::MessageType::Text),
        content: Some(cm::MessageContent::Text(content.into())),
        status: Some(status),
        ..Default::default()
    }
}

/// 流式文本：合并**回报**被追加的那段，VDFS 变更才能据此产出 `appended`。
///
/// 这是「变更语义与合并语义不漂移」的结构保证——判据不是另算一遍，
/// 而是合并函数自己说出来的。若有人日后把 `appended` 的判据改成独立实现，
/// 这条测试仍会通过（它测的是回报值），所以另有一条测试直接断言两者一致
/// （见 `plugin.rs::message_change_maps_patch_to_change_kind`）。
#[test]
fn merge_reports_appended_delta_for_streamed_text() {
    let mut existing = text_msg("m1", "你好", cm::MessageStatus::Streaming);
    let delta = merge_message_patch(
        &mut existing,
        &text_msg("m1", "，世界", cm::MessageStatus::Streaming),
    );
    assert_eq!(delta.as_deref(), Some("，世界"));
    assert_eq!(
        existing.content.as_ref().map(|c| c.to_text()),
        Some("你好，世界".to_string()),
        "回报的 delta 必须正是被拼进去的那一段"
    );

    // 首次写入同样是追加：节点此前无正文，尾部多出来的就是全部
    let mut empty = text_msg("m2", "", cm::MessageStatus::Streaming);
    empty.content = None;
    let delta = merge_message_patch(
        &mut empty,
        &text_msg("m2", "开头", cm::MessageStatus::Streaming),
    );
    assert_eq!(delta.as_deref(), Some("开头"));
}

/// 全量替换类补丁**不得**回报增量——否则消费者会按 delta 拼接，
/// 把「每帧都是完整参数」的工具调用拼成垃圾。
#[test]
fn merge_reports_no_delta_for_full_replacement() {
    // ToolCall：每帧全量 JSON
    let mut tc = cm::ChatMessage {
        id: "t1".into(),
        role: Some(cm::MessageRole::Assistant),
        msg_type: Some(cm::MessageType::ToolCall),
        content: Some(cm::MessageContent::Text("{\"a\":".into())),
        ..Default::default()
    };
    let patch = cm::ChatMessage {
        id: "t1".into(),
        msg_type: Some(cm::MessageType::ToolCall),
        content: Some(cm::MessageContent::Text("{\"a\":1}".into())),
        ..Default::default()
    };
    assert!(merge_message_patch(&mut tc, &patch).is_none());
    assert_eq!(
        tc.content.as_ref().map(|c| c.to_text()),
        Some("{\"a\":1}".to_string()),
        "ToolCall 是全量替换，不是追加"
    );

    // role=Tool 的工具流式帧：全量替换（与前端 sessions.ts 同语义）
    let mut tool = text_msg("r1", "第一行\n", cm::MessageStatus::Streaming);
    tool.role = Some(cm::MessageRole::Tool);
    let patch = cm::ChatMessage {
        id: "r1".into(),
        role: Some(cm::MessageRole::Tool),
        content: Some(cm::MessageContent::Text("第一行\n第二行\n".into())),
        ..Default::default()
    };
    assert!(merge_message_patch(&mut tool, &patch).is_none());

    // 纯状态补丁（只带 id + status）：不触及内容
    let mut m = text_msg("m3", "正文", cm::MessageStatus::Streaming);
    let status_only = cm::ChatMessage {
        id: "m3".into(),
        status: Some(cm::MessageStatus::Completed),
        ..Default::default()
    };
    assert!(merge_message_patch(&mut m, &status_only).is_none());
    assert_eq!(m.status, Some(cm::MessageStatus::Completed));
    assert_eq!(
        m.content.as_ref().map(|c| c.to_text()),
        Some("正文".to_string())
    );
}
