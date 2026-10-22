//! 一次执行的**开幕**（T-225）：用户格 / 配置 / 轮次事实先于任何判决入格。
//!
//! ## 为什么它必须在这里
//!
//! `owed` 的输入闭包只有事实 / 视图 / 触发事件时间戳。用户格与配置若仍由运行器
//! 在判决之后落，义务环就用不上它们——所以写入点前移到主循环的闸门与判决之间：
//! `gate_turn` 之前不写（中止边界不产生事实），`decide` 之前写完。
//!
//! ## 两种幕
//!
//! - **新轮**：turn 号 = 事实切片里既有用户格计数；落一条 `user.message`
//!   （`u-{turn}`），它的 seq 同时是本幕的锚。
//! - **续写幕**（审批 / 问答恢复）：复用被续写轮的号，**不新开用户格**；落一条
//!   `turn.stage{stage:"resume"}`，它的 seq 是本幕的锚。同一轮可以先后恢复两次
//!   同类动作，锚的幂等键因此带幕序数。
//!
//! 两种幕都落 `session.config`（当轮生效的配置）与 `turn.stage{stage:"round"}`。
//! 配置读的是**这一轮**的活值，落成事实之后判决只读事实：回放看到的永远是
//! 「那一轮用的配置」，不是进程此刻的值。

use crate::symbio_core::{
    count_user_messages, Entity, Seq, Verb, EVENT_SESSION_CONFIG, EVENT_TURN_STAGE,
    EVENT_USER_MESSAGE,
};

use super::super::stage_events::{self, StageFact};

/// 租约看护的扫描窗口（条）——与视图声明的 `cost` 上界同值：超过它视图
/// 就会拒绝求值，所以调用方自己先切（分层而不是让视图静默算不出来）。
const LEASE_WINDOW: usize = 4096;

/// 本幕的身份与输入。
pub(crate) struct Drive {
    pub(crate) turn_no: u64,
    pub(crate) user_seq: u64,
    pub(crate) drive_seq: u64,
    /// 本轮用户发言（来自最近一条用户消息节点；无用户消息 ⇒ 空串）。
    pub(crate) user_text: String,
}

/// 最近一条**用户消息节点**的 `(id, 正文, meta)`。
///
/// 与 `chat_loop::inputs::last_user_text` 同一条判据（后往前第一条 user 角色）；
/// 这里多取 id 与 meta，因为用户格的载荷要复刻节点形状。
fn last_user_message(
    context: &super::state::SessionContext,
) -> (Option<String>, String, Option<serde_json::Value>) {
    context
        .messages
        .iter()
        .rev()
        .find(|m| m.node_role() == Some(crate::symbio_core::chat_node::NodeRole::User))
        .map(|m| {
            let text = match &m.content {
                Some(crate::symbio_core::chat_node::MessageContent::Text(t)) => t.clone(),
                _ => String::new(),
            };
            (Some(m.id.clone()), text, m.meta.clone())
        })
        .unwrap_or((None, String::new(), None))
}

/// 恢复动作的事实名（`turn.stage{stage:"resume"}` 的 `result`）。
///
/// 与 `ResumeAction` 的 serde 线格式同值；`RetrySubtree` / `RetryCompaction`
/// 不落本格（前者按新轮处理，后者在进循环之前就返回了）。
pub(crate) fn resume_kind(action: &crate::symbio_core::chat_node::ResumeAction) -> &'static str {
    use crate::symbio_core::chat_node::ResumeAction;
    match action {
        ResumeAction::Retry => "retry",
        ResumeAction::Approve => "approve",
        ResumeAction::Reject => "reject",
        ResumeAction::Supply => "supply",
        ResumeAction::Answer => "answer",
        ResumeAction::RetrySubtree => "retry_subtree",
        ResumeAction::RetryCompaction => "retry_compaction",
    }
}

/// 开一幕：用户格（或恢复格）→ 配置 → 本幕锚 → 刷新本轮事实切片。
///
/// 返回的 [`Drive`] 是本幕的**唯一身份来源**——后续的派发格、阶段格、收束格
/// 都从它取 `(turn_no, user_seq, drive_seq)`，不再各自数一遍事实。
pub(crate) fn begin_drive(
    context: &super::state::SessionContext,
    turn: &mut super::state::TurnState,
    req: &super::state::TurnRequest,
    resume_kind: Option<&str>,
) -> Result<Drive, super::state::TurnExit> {
    let store = context.session.facts().ok_or_else(|| {
        super::state::TurnExit::Failed(crate::symbio_core::PluginError::InternalError(
            "v2 执行需要持久会话（临时会话无事实源）".into(),
        ))
    })?;
    let snapshot = store.range(Seq::new(0));
    // ── 先收租（T-223）：上一段生命里派出去、没回来的调用 ─────────────────
    //
    // 看护点就是**恢复点**：时间只在事件上推进，所以一条 `sent` 只要跨过进程
    // 重启就一定已经超时。重试还是判死由 `PortDecl.max_attempts` 定（`turn`
    // 是 1 ⇒ 判死，因为等它的 future 已经没了）。
    //
    // 切片按声明的扫描上界（4096）取尾部：租约要看的只是最近那几笔在途，
    // 超过上界就交给调用方分层而不是让视图静默算不出来。
    if let Some(frozen) = super::super::loop_rows::table() {
        let tail = &snapshot[snapshot.len().saturating_sub(LEASE_WINDOW)..];
        // 被收的那几笔属于**它们自己那一轮**（往往不是即将开的这一幕）。
        let stale_turn = tail.last().map(|e| e.turn).unwrap_or(0);
        let anchor = tail
            .last()
            .and_then(|e| e.seq)
            .map(|s| s.value())
            .unwrap_or(0);
        if let Err(e) = super::super::port::reap(
            store.as_ref(),
            &frozen,
            super::super::loop_rows::PORTS,
            tail,
            stale_turn,
            anchor,
            crate::symbio_core::clock_now_ms(),
        ) {
            // 收租失败不阻断新局面：那一笔欠账仍然在日志里开着（不静默当它死了）。
            crate::plugin_warn!("session", "[Loop] 租约看护失败（继续开新幕）: {e:?}");
        }
    }
    let (turn_no, user_seq, drive_seq, user_text) = match resume_kind {
        Some(kind) => {
            let (turn_no, user_seq) = super::super::turn_exec::last_open_turn(&snapshot)
                .ok_or_else(|| {
                    super::state::TurnExit::Failed(crate::symbio_core::PluginError::InternalError(
                        "续写轮找不到未收束的轮次（事实源与恢复路径不一致）".into(),
                    ))
                })?;
            let ordinal = snapshot
                .iter()
                .filter(|e| {
                    e.turn == turn_no
                        && e.kind == EVENT_TURN_STAGE
                        && e.payload.get("stage").and_then(|v| v.as_str()) == Some("resume")
                })
                .count()
                + 1;
            let seqs = stage_events::land_with_seqs(
                store.as_ref(),
                turn_no,
                user_seq,
                "system",
                &[StageFact::resume(
                    turn.request_round,
                    kind,
                    &ordinal.to_string(),
                )],
            )
            .map_err(|e| {
                super::state::TurnExit::Failed(crate::symbio_core::PluginError::InternalError(
                    format!("恢复阶段事实落格失败：{e:?}"),
                ))
            })?;
            let drive_seq = seqs.first().copied().unwrap_or(0);
            (turn_no, user_seq, drive_seq, last_user_message(context).1)
        }
        None => {
            let turn_no = count_user_messages(&snapshot);
            let (node_id, text, meta) = last_user_message(context);
            let mut payload = serde_json::json!({ "text": text, "id": node_id });
            if let Some(meta) = meta.filter(|m| !m.is_null()) {
                payload["meta"] = meta;
            }
            let seq = store
                .append(
                    crate::symbio_core::Event::pending(
                        format!("u-{turn_no}"),
                        EVENT_USER_MESSAGE,
                        Entity::Turn,
                        Verb::Opened,
                        turn_no,
                        "user",
                    )
                    .with_payload(payload),
                )
                .map_err(|e| {
                    super::state::TurnExit::Failed(crate::symbio_core::PluginError::InternalError(
                        format!("用户格落格失败：{e:?}"),
                    ))
                })?;
            (turn_no, seq.value(), seq.value(), text)
        }
    };
    let settings = context.session.settings();
    store
        .append(
            crate::symbio_core::Event::pending(
                format!("cfg-{turn_no}-{drive_seq}"),
                EVENT_SESSION_CONFIG,
                Entity::Turn,
                Verb::Asserted,
                turn_no,
                "system",
            )
            .with_produced_by(user_seq)
            .with_payload(serde_json::json!({
                "classify_enabled": settings.classify_enabled,
                // 生效值（请求级覆盖已经在 `TurnRequest::new` 里折过）。
                "auto_compress": req.auto_compress,
                "progress_enabled": settings.progress_enabled,
                "progress_interval_ms": settings.progress_interval_ms,
                "progress_min_rounds": settings.progress_min_rounds,
                "progress_max_per_turn": settings.progress_max_per_turn,
                "is_persistent": context.session.is_persistent(),
            })),
        )
        .map_err(|e| {
            super::state::TurnExit::Failed(crate::symbio_core::PluginError::InternalError(format!(
                "配置事实落格失败：{e:?}"
            )))
        })?;
    stage_events::land(
        store.as_ref(),
        turn_no,
        user_seq,
        "system",
        &[StageFact::request_round(
            turn.request_round,
            &drive_seq.to_string(),
        )],
    )
    .map_err(|e| {
        super::state::TurnExit::Failed(crate::symbio_core::PluginError::InternalError(format!(
            "轮次阶段事实落格失败：{e:?}"
        )))
    })?;
    turn.turn_no = turn_no;
    turn.user_seq = user_seq;
    turn.drive_seq = drive_seq;
    turn.facts = store.range(Seq::new(0));
    Ok(Drive {
        turn_no,
        user_seq,
        drive_seq,
        user_text,
    })
}

#[cfg(test)]
#[path = "drive.test.rs"]
mod tests;
