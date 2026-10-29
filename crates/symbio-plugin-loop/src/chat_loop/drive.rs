//! 一次执行的**开幕**：用户格 / 配置 / 轮次事实先于任何判决入格。
//!
//! ## 为什么它必须在这里
//!
//! `owed` 的输入闭包只有事实 / 视图 / 触发事件时间戳。用户格与配置若仍由运行器
//! 在判决之后落，义务环就用不上它们——所以写入点前移到主循环的闸门与判决之间：
//! `gate_turn` 之前不写（中止边界不产生事实），`decide` 之前写完。
//!
//! ## 三种幕
//!
//! - **新轮**：turn 号 = 事实切片里既有用户格计数；落一条 `user.message`
//!   （`u-{turn}`），它的 seq 同时是本幕的锚。
//! - **续写幕**（审批 / 问答恢复）：复用被续写轮的号，**不新开用户格**；落一条
//!   `turn.stage{stage:"resume"}`，它的 seq 是本幕的锚。同一轮可以先后恢复两次
//!   同类动作，锚的幂等键因此带幕序数。
//! - **再接一幕**（同一轮被输出上限掐断之后，`D-222` 裁决 2）：复用轮号与用户格，
//!   落一条 `turn.stage{stage:"round", round: N+1}`——**那一格自己就是本幕的锚**
//!   （判据 12：一幕一个锚）。它的幂等段是轮次本身（同一轮里 `round` 单调增），
//!   所以不必借别的事实的 seq。
//!
//! 三种幕都落 `session.config`（当轮生效的配置）。配置读的是**这一轮**的活值，
//! 落成事实之后判决只读事实：回放看到的永远是「那一轮用的配置」，不是进程此刻的值。

use symbio_core::{count_user_messages, Entity, Event, Seq, Store, Verb};
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
/// 留在这里的只有用户发言：它不落进任何一格，而 `turn_exec::drive` 要用它判技能命中。
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

/// 本轮用户发言（最近一条用户消息节点的正文；无用户消息 ⇒ 空串）。
///
/// 同一轮的第二幕起用它：那一幕的唤醒格已由 `continue-turn` 落过，`begin_drive`
/// 不再被调用，而执行体仍要按发言判技能命中（`turn_exec::drive` 的输入）。
pub(crate) fn current_user_text(context: &super::state::SessionContext) -> String {
    last_user_message(context).1
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

/// 这一跑是从哪儿开的幕（`D-222` 裁决 1–2）。
///
/// 前两档是**交换点**：外部的人话 / 恢复动作变成一笔请求，环从那一刻起才有账，
/// 所以它们在环外。第三档不是外部输入——它是同一轮里「再跑一段」那笔**债**的兑现，
/// 前件（`closure = exhausted` ∧ 轮次未到上限）与出口（新的 `round` 格）都在表上。
enum Wake<'a> {
    NewTurn,
    Resume(&'a str),
    Continue,
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
    open_drive(
        context,
        turn,
        req,
        match resume_kind {
            Some(kind) => Wake::Resume(kind),
            None => Wake::NewTurn,
        },
    )
}

/// 同一轮里**再接一幕**：轮号与用户格都复用，本幕的锚是刚落下的 `round` 格。
///
/// 与 [`begin_drive`] 分成两个入口只为一件事：这一档没有外部输入。它由
/// `continue-turn` 那笔债兑现（`D-222` 裁决 2），而债的前件在表上；把它和新轮
/// 混成一个 `None` 会让「本幕是不是用户新开的一轮」在代码里没有答案——续写的
/// 第二幕会重落一条同文本的用户格、轮号原地膨胀。
pub(crate) fn continue_drive(
    context: &super::state::SessionContext,
    turn: &mut super::state::TurnState,
    req: &super::state::TurnRequest,
) -> Result<Drive, super::state::TurnExit> {
    open_drive(context, turn, req, Wake::Continue)
}

fn open_drive(
    context: &super::state::SessionContext,
    turn: &mut super::state::TurnState,
    req: &super::state::TurnRequest,
    wake: Wake<'_>,
) -> Result<Drive, super::state::TurnExit> {
    let store = context.session.facts().ok_or_else(|| {
        super::state::TurnExit::Failed(symbio_core::PluginError::InternalError(
            "执行需要持久会话（临时会话无事实源）".into(),
        ))
    })?;
    // ── 行表是开幕的前提（DE-015 §3.4：装载失败即拒绝启动，不许部分成功）──────
    //
    // 装载期任何缺口都让 `table` 恒 `Err`。这里不
    // 「跳过收租继续开新幕」——没有表就没有 `owed`，那一幕只会静默停在原处。
    let frozen = super::super::loop_rows::table().map_err(super::state::TurnExit::Failed)?;
    let snapshot = store.range(Seq::new(0));
    // `(轮号, 用户格, 用户发言, 本幕轮次, 幕锚)`。
    //
    // 幕锚 `Some(seq)` = 唤醒格（用户格 / 恢复格）；`None` = 续幕那条 `round` 格
    // ——它自己那一格的 seq 要落完才知道，所以锚在下面才定。
    let (turn_no, user_seq, user_text, round, anchor) = match wake {
        Wake::Resume(kind) => {
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
            (
                turn_no,
                user_seq,
                last_user_message(context).1,
                turn.request_round,
                Some(drive_seq),
            )
        }
        Wake::NewTurn => {
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
            (
                turn_no,
                seq.value(),
                text,
                turn.request_round,
                Some(seq.value()),
            )
        }
        Wake::Continue => {
            // 轮次由**那一格**回答（`turn_next_round`）：`continue-turn` 的前件与
            // 这里读的是同一个视图，不在这里按「当前轮次 + 1」再数一遍。
            let round =
                super::super::loop_rows::next_stage_round(&snapshot, turn.turn_no, turn.now_ms)
                    .ok_or_else(|| {
                        super::state::TurnExit::Failed(symbio_core::PluginError::InternalError(
                            "续幕没有下一轮的判据（continue-turn 的前件与事实不一致）".into(),
                        ))
                    })?;
            (
                turn.turn_no,
                turn.user_seq,
                last_user_message(context).1,
                usize::try_from(round).unwrap_or(usize::MAX),
                None,
            )
        }
    };
    // ── 本幕的锚 ─────────────────────────────────────────────────────────
    //
    // 续幕的锚就是它的 `round` 格本身（判据 12：一幕一个锚），所以那一格要**先**
    // 落、再从它的 seq 取 `drive_seq`。唤醒幕的锚是其唤醒格，`round` 格跟在
    // 配置之后落（与合表之前逐字同序）。
    let (drive_seq, round_landed) = match anchor {
        Some(seq) => (seq, false),
        None => {
            // 幂等段用轮次本身：同一轮里 `round` 单调增，不借别的事实。
            let seqs = stage_events::land_with_seqs(
                store.as_ref(),
                turn_no,
                user_seq,
                "system",
                &[StageFact::request_round(round, &round.to_string())],
            )
            .map_err(|e| {
                super::state::TurnExit::Failed(symbio_core::PluginError::InternalError(format!(
                    "轮次阶段事实落格失败：{e:?}"
                )))
            })?;
            (seqs.first().copied().unwrap_or(0), true)
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
    // 唤醒幕的轮次格跟在配置之后（与合表之前逐字同序）；续幕那一格已先落——它是锚。
    if !round_landed {
        stage_events::land(
            store.as_ref(),
            turn_no,
            user_seq,
            "system",
            &[StageFact::request_round(round, &drive_seq.to_string())],
        )
        .map_err(|e| {
            super::state::TurnExit::Failed(symbio_core::PluginError::InternalError(format!(
                "轮次阶段事实落格失败：{e:?}"
            )))
        })?;
    }
    turn.turn_no = turn_no;
    turn.user_seq = user_seq;
    turn.drive_seq = drive_seq;
    turn.request_round = round;
    turn.now_ms = now_ms;
    turn.facts = store.range(Seq::new(0));
    Ok(Drive { user_text })
}

// ── 接缝上的两次落格：档位与开场白 ─────────────────────────────────────────
//
// 两者都是**幕级事实**（`turn.dispatch` / `chat.assistant.opening`），但它们的内容要到
// 轮首四行跑完才存在：档位由 `policy.reflex` 的出口格折出（`D-223` 订正 1），开场白是
// 判决行 `Escalate` 首响的那句话。所以它们**不能**跟 `begin_drive` 一起落（那里太早，
// 四行一格都还没写），也不能留在 `TurnRunner::begin` 里（草稿要排到幕首，`D-222` 订正 3）
// ——落点是接缝：轮首四行之后、内环之前。
//
// 为什么是自由函数而不是 `TurnSession::set_*`：这两格草稿一格都不读（`prefix` / `reflex`
// 进草稿是因为环内要用），能落格的句柄在接缝这一侧（事实源 + 上线面）。挂到草稿上等于
// 让草稿持一份自己用不到的句柄。
//
// 合表之后这条接缝消失：`Ctx` 里有 `&mut TurnState`，需要这两格的消费者自己读。

/// 落**派发档**（`turn.dispatch`）：本幕以哪一档派发、给多大预算、以谁的身份。
///
/// 幂等键带**幕号**（`d-{turn}-{drive_seq}`）：同一轮可以被恢复幕再执行一次，两幕的档位
/// 各记各的。
///
/// `budget_ms` 取 `tier.budget_ms()`，不取主体规格里那一格（`D-223` 订正 2）：规格的
/// 预算写在幕首建主体时，那一刻档位还没折出来，它只能记主体的**声明档**；本幕的实际
/// 预算归这一格。`I3`（`invariants::declared_budget_ms`）读的永远是这一格。
pub(crate) fn land_dispatch(
    store: &dyn Store<Event = Event>,
    turn_no: u64,
    user_seq: u64,
    drive_seq: u64,
    tier: symbio_core::LatencyTier,
    actor: &symbio_core::ActorSpec,
) -> Result<(), symbio_core::AppendError> {
    store.append(
        Event::pending(
            format!("d-{turn_no}-{drive_seq}"),
            crate::control_decl::EVENT_TURN_DISPATCH,
            Entity::Turn,
            Verb::Asserted,
            turn_no,
            &actor.principal,
        )
        .with_produced_by(user_seq)
        // `budget_ms` 由**档位**折出，不由主体带（`D-223` 订正 2）：主体在幕首就建
        // （`D-222` 订正 3），那一刻档位还没定，规格里那一格只能是主体的**声明档**。
        // 本幕的预算因此只从这里出——`tier` 与 `budget_ms` 由同一个值派生，两格
        // 不可能对不上。
        .with_payload(serde_json::json!({
            "tier": tier.name(),
            "budget_ms": tier.budget_ms(),
            "principal": actor.principal,
        })),
    )?;
    Ok(())
}

/// 落**对话面开场白**（`chat.assistant.opening`，`Escalate` 首响）并让它上线。
///
/// 本轮随后**照进工具循环**，所以它落在 `turn × asserted`（不是收束格 N3「每轮至多一条」）。
/// 空正文不落格（与汇报 / 收束同一条纪律）。落格即上线：节点 id 用落格载荷里那个
/// （`dialog.id`），轮末同 id 再送一次时消费端按 id 合并成同一格。
///
/// `None` 的两样（没有首响 / 没有节点身份）都退化成「什么都不做」：两者必须同时在场，
/// 只有正文而没有节点身份会让实时面与重建面折出两个不同的节点。
pub(crate) fn land_opening(
    store: &dyn Store<Event = Event>,
    sink: &dyn symbio_core::DeltaSink,
    turn_no: u64,
    user_seq: u64,
    principal: &str,
    text: Option<&str>,
    dialog: Option<&super::super::turn_runner::DialogFacts>,
) -> Result<(), symbio_core::AppendError> {
    let (Some(opening), Some(dialog)) = (text, dialog) else {
        return Ok(());
    };
    if opening.trim().is_empty() {
        return Ok(());
    }
    store.append(
        Event::pending(
            format!("op-{turn_no}"),
            symbio_plugin_api::events::EVENT_CHAT_ASSISTANT_OPENING,
            Entity::Turn,
            Verb::Asserted,
            turn_no,
            principal,
        )
        .with_produced_by(user_seq)
        .with_payload(serde_json::json!({
            "text": opening,
            "id": dialog.id,
            "reason": dialog.reason,
            "exclude_from_context": dialog.exclude_from_context,
        })),
    )?;
    let mut node = symbio_core::node_build::dialog_node(
        opening,
        &dialog.reason,
        dialog.exclude_from_context,
        &symbio_core::principal::Principal::from_legacy(principal),
    );
    node.id = dialog.id.clone();
    sink.on_tool_frame(&node);
    Ok(())
}

#[cfg(test)]
#[path = "drive.test.rs"]
mod tests;
