//! 一次执行的**开幕**：用户格 / 配置 / 轮次事实先于任何判决入格。
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

use symbio_core::{count_user_messages, Entity, Seq, Verb};
use symbio_plugin_api::events::EVENT_USER_MESSAGE;

use crate::control_decl::{CHAT_VOCAB, EVENT_SESSION_CONFIG, EVENT_TURN_STAGE, MESSAGE_VOCAB};

use super::super::stage_events::{self, StageFact};

/// 租约看护的扫描窗口（条）——**由声明回答**，不在这里抄一个数。
///
/// 此前这里写着 `const LEASE_WINDOW: usize = 4096`，与 `control_decl` 的七处
/// `cost: 4096` 手抄同值。手抄同值不是同步：两侧任一单独改动，另一侧就静默失效，
/// 而失效的形状是 `Frozen::eval` 回 `None` ⇒ `ring::owed` 折成空集 ⇒ 看护扫不到任何
/// 过期尝试、`waiting` 恒假。现在窗口就是切片上界本身
/// （[`Frozen::scan_bound`]），「同值」由构造保证，不是约定。
///
/// 表还没装好时回 `0`：那一轮的打开会先被 `loop_rows::table` 拦下，这里不替它判。
fn lease_window() -> usize {
    super::super::loop_rows::scan_bound().unwrap_or(0)
}

/// 收租窗口的切片：快照的**尾部** [`lease_window`] 条。
///
/// 单独成函数是为了让「窗口边界」成为**被断言的行为**而不是散在 `begin_drive` 里
/// 的一段切片表达式（`drive.test.rs` 直接喂一份超窗夹具断言切点）。窗口外的未回
/// `sent` 不会被看护看到，只能由 `waiting` 行挂着——这是窗口选值的边界，不是安全
/// 保证，失效条件写在 [DE-015 §2.6](../../../../docs/003-design/015-义务环.md "DE-015-义务环")
/// 的第 7 条边界段里。
fn lease_tail(snapshot: &[symbio_core::Event]) -> &[symbio_core::Event] {
    &snapshot[snapshot.len().saturating_sub(lease_window())..]
}

/// 本幕落成事实之后，编排层**还需要**的那一样东西。
///
/// 身份三格（`turn_no` / `user_seq` / `drive_seq`）**不在这里**：它们已经写进
/// `TurnState`，是同一批值的第二份拷贝。「现在是哪一幕」只能有一个真源（`D-222`
/// §二：幕是身份），而它已经在 `TurnState` 里（终态还要改成由事实折出）。
/// 留在这里的只有用户发言：它不落进任何一格，而 `execute_turn` 要用它判技能命中。
pub(crate) struct Drive {
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
        .find(|m| m.node_role() == Some(symbio_core::chat_node::NodeRole::User))
        .map(|m| {
            let text = match &m.content {
                Some(symbio_core::chat_node::MessageContent::Text(t)) => t.clone(),
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
pub(crate) fn resume_kind(action: &symbio_core::chat_node::ResumeAction) -> &'static str {
    use symbio_core::chat_node::ResumeAction;
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

/// 开一幕：唤醒格（用户格 / 恢复格）→ 收租 → 配置 → 本幕锚 → 刷新本轮事实切片。
///
/// 唤醒格排在收租**之前**：`now` 只能来自那条事实的 `ts`（判据 12），所以本次
/// `drive` 的时间要先进日志，租约才有得算。
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
        super::state::TurnExit::Failed(symbio_core::PluginError::InternalError(
            "v2 执行需要持久会话（临时会话无事实源）".into(),
        ))
    })?;
    // ── 行表是开幕的前提（DE-015 §3.4：装载失败即拒绝启动，不许部分成功）──────
    //
    // 装载期任何缺口都让 `table` 恒 `Err`。这里不
    // 「跳过收租继续开新幕」——没有表就没有 `owed`，那一幕只会静默停在原处。
    let frozen = super::super::loop_rows::table().map_err(super::state::TurnExit::Failed)?;
    let snapshot = store.range(Seq::new(0));
    let (turn_no, user_seq, drive_seq, user_text) = match resume_kind {
        Some(kind) => {
            let (turn_no, user_seq) = super::super::turn_exec::last_open_turn(&snapshot)
                .ok_or_else(|| {
                    super::state::TurnExit::Failed(symbio_core::PluginError::InternalError(
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
                super::state::TurnExit::Failed(symbio_core::PluginError::InternalError(format!(
                    "恢复阶段事实落格失败：{e:?}"
                )))
            })?;
            let drive_seq = seqs.first().copied().unwrap_or(0);
            (turn_no, user_seq, drive_seq, last_user_message(context).1)
        }
        None => {
            let turn_no = count_user_messages(&snapshot, &CHAT_VOCAB, &MESSAGE_VOCAB);
            let (node_id, text, meta) = last_user_message(context);
            let mut payload = serde_json::json!({ "text": text, "id": node_id });
            if let Some(meta) = meta.filter(|m| !m.is_null()) {
                payload["meta"] = meta;
            }
            let seq = store
                .append(
                    symbio_core::Event::pending(
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
                    super::state::TurnExit::Failed(symbio_core::PluginError::InternalError(
                        format!("用户格落格失败：{e:?}"),
                    ))
                })?;
            (turn_no, seq.value(), seq.value(), text)
        }
    };
    // ── 本次 drive 的 `now` = 刚落下这条唤醒事实的 `ts`（判据 12）─────────────
    //
    // 幕内每一次债求值都用这一个值。持久事实源在入库处给缺时刻的事件补落格时刻
    // （`EventEnvelope::fallback_timestamp`），于是这个 `ts` 就是唤醒发生的那一
    // 毫秒——停机跨了多久由这条事实带进日志，而不是由判决现读墙钟。
    let now_ms = super::super::loop_rows::wake_now_ms(&store.range(Seq::new(drive_seq)), drive_seq);
    // ── 收租：上一段生命里派出去、没回来的调用 ─────────────────
    //
    // 看护点就是**恢复点**：唤醒事实的 `ts` 比上一世最后一条 `sent` 晚了一整个
    // 停机区间，所以跨过进程重启的那笔一定已经超时。重试还是判死由
    // `PortDecl.max_attempts` 定（`turn` 是 1 ⇒ 判死，因为等它的 future 已经没了）。
    //
    // 扫描窗口取**开幕前**那份快照的尾部（按视图声明的 4096 上界切）：被收的那几笔
    // 属于它们自己那一轮，与刚落的唤醒格无关；切在开幕前也让 `anchor` 仍是上一世
    // 的最后一条。
    {
        let tail = lease_tail(&snapshot);
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
            now_ms,
        ) {
            // 收租失败不阻断新局面：那一笔欠账仍然在日志里开着（不静默当它死了）。
            symbio_core::plugin_warn!("session", "[Loop] 租约看护失败（继续开新幕）: {e:?}");
        }
    }
    let settings = context.session.settings();
    store
        .append(
            symbio_core::Event::pending(
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
            super::state::TurnExit::Failed(symbio_core::PluginError::InternalError(format!(
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
        super::state::TurnExit::Failed(symbio_core::PluginError::InternalError(format!(
            "轮次阶段事实落格失败：{e:?}"
        )))
    })?;
    turn.turn_no = turn_no;
    turn.user_seq = user_seq;
    turn.drive_seq = drive_seq;
    turn.now_ms = now_ms;
    turn.facts = store.range(Seq::new(0));
    Ok(Drive { user_text })
}

#[cfg(test)]
#[path = "drive.test.rs"]
mod tests;
