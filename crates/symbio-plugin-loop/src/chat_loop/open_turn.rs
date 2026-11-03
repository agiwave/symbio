//! 幕锚的**事实行**（P2-1）：`open-turn` / `resume-turn` 两行产出本幕的锚。
//!
//! ## 为什么幕锚是一行
//!
//! 幕锚（用户格 / 恢复格 / `round` 格）不是环的七项职务，而是**事实**：
//! 它决定 `turn_head` 看得见哪一段、租约从哪一刻算起、续写幕复用哪个根。
//! 既然它只是「读请求 → 写事实」，按 D-313 它就归行——与 `open-tools` /
//! `authorize-tool` 同一条判据（I-2 + 行1），不在驱动里按分支写。
//!
//! ## 两行，一条顺序
//!
//! | 行 | 写什么 | 什么时候 |
//! |---|---|---|
//! | `resume-turn` | 用户动作 → `request(port=tool)`（带 `resume` 标记） | 续写幕，**先于**幕锚 |
//! | `open-turn` | `user.message` / `session.config` / `turn.stage{resume\|round}` | 新轮 / 续写幕 / 再接一幕 |
//!
//! `resume-turn` 必须在前：`turn_head` 的请求面按 seq 判「有没有终局」，不看幕锚，
//! 但锚的 `now` 要等唤醒格落下才有得算（判据 12）。
//!
//! ## 驱动只读结果
//!
//! [`Opened`] 是驱动拿到的那一份：`user_text`、复用的根、是否新开。驱动不再自己
//! 数用户格、不再自己决定「要不要新根」——数事实的活跟着写事实的那一行走。
//!
//! 事实行仍**不在债表上**：它们是外部输入变成债之前的交换点（D-222 裁决 1），
//! 由驱动在开一幕时按 [`select`] 选中并调用；债表 `loop_rows::ACTORS` 只收
//! 「这一轮欠什么」。两处都不写第二份身份。

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use symbio_kernel::event::{Entity, Event, Seq, Verb, EVENT_REQUEST};
use symbio_kernel::plugin::PluginError;
use symbio_plugin_api::events::EVENT_USER_MESSAGE;
use symbio_sdk::chat::chat_node::{NodeRole, ResumeAction, ResumeRequest};
use symbio_sdk::projection::turns::count_user_messages;

use crate::control_decl::{CHAT_VOCAB, EVENT_SESSION_CONFIG, EVENT_TURN_STAGE, MESSAGE_VOCAB};

use super::super::stage_events::{self, StageFact};
use super::super::session_port::SessionPort;
use super::state::{SessionContext, TurnExit, TurnRequest, TurnState};

/// 一行幕锚：名字、写格、动作。
pub(crate) struct OpeningRow {
    pub(crate) name: &'static str,
    pub(crate) writes: &'static [&'static str],
    pub(crate) action: OpeningAction,
}

pub(crate) type OpeningAction =
    for<'a, 'b> fn(
        &'a mut OpeningCtx<'b>,
    ) -> Pin<Box<dyn Future<Output = Result<(), TurnExit>> + Send + 'a>>;

/// 开一幕的入参与出参。
pub(crate) struct OpeningCtx<'a> {
    pub(crate) context: &'a SessionContext,
    pub(crate) turn: &'a mut TurnState,
    pub(crate) req: &'a TurnRequest,
    pub(crate) session: &'a Arc<dyn SessionPort>,
    /// 本轮的调用面：分支事实的 actor 取自它的授权主体。
    pub(crate) request_ctx: &'a std::sync::Arc<dyn symbio_kernel::plugin::PluginInvokeRequest>,
    /// 本次请求的恢复动作（`None` = 新轮 / 再接一幕）。
    pub(crate) resume: Option<&'a ResumeRequest>,
    pub(crate) out: &'a mut Opened,
}

/// 驱动从幕锚行拿到的全部东西。
#[derive(Default)]
pub(crate) struct Opened {
    /// 本轮用户发言（来自最近一条用户消息节点；无用户消息 ⇒ 空串）。
    pub(crate) user_text: String,
    /// 本幕的 Turn 根：续写幕复用被续写轮的根；新轮由 `open-turn` 现开。
    pub(crate) root_id: Option<String>,
    /// `true` = 本幕新开一个根（流式骨架要发）；`false` = 复用旧根。
    pub(crate) fresh_root: bool,
    /// 续写幕要替换的旧结果子节点（由 `resume-turn` 交给 `open-turn` 写进恢复格）。
    pub(crate) replaces: Option<String>,
}

/// 本幕的锚输入：续写幕交回的旧根 / 旧结果子，或重试子支要登记的旧根。
#[derive(Default)]
pub(crate) struct Anchor {
    /// 复用的 Turn 根（`None` = 本幕现开一个）。
    pub(crate) root: Option<String>,
    /// 续写幕要替换的旧结果子节点。
    pub(crate) replaces: Option<String>,
    /// 重试子支的旧根（分支格 `of`）。
    pub(crate) branch_of: Option<String>,
}

/// 本幕要跑哪些幕锚行。命中 `retry`（子支重来）也只有一个 `open-turn`。
pub(crate) fn select(resume: Option<&ResumeRequest>, first_stage: bool) -> &'static [OpeningRow] {
    if !first_stage {
        return &[];
    }
    match resume {
        Some(req) if !matches!(req.action, ResumeAction::RetrySubtree) => &RESUME_THEN_OPEN,
        _ => &OPEN_ONLY,
    }
}

const OPEN_WRITES: &[&str] = &[
    EVENT_USER_MESSAGE,
    EVENT_SESSION_CONFIG,
    EVENT_TURN_STAGE,
    symbio_plugin_api::events::EVENT_TURN_BRANCH,
];
const RESUME_WRITES: &[&str] = &[EVENT_REQUEST];

const OPEN_ONLY: [OpeningRow; 1] = [OpeningRow {
    name: "open-turn",
    writes: OPEN_WRITES,
    action: run_open_turn,
}];
const RESUME_THEN_OPEN: [OpeningRow; 2] = [
    OpeningRow {
        name: "resume-turn",
        writes: RESUME_WRITES,
        action: run_resume_turn,
    },
    OpeningRow {
        name: "open-turn",
        writes: OPEN_WRITES,
        action: run_open_turn,
    },
];

/// `resume-turn`：把用户动作翻译成一笔唤醒请求（`request(port=tool)`）。
fn run_resume_turn<'a, 'b>(
    ctx: &'a mut OpeningCtx<'b>,
) -> Pin<Box<dyn Future<Output = Result<(), TurnExit>> + Send + 'a>> {
    Box::pin(async move {
        let Some(req) = ctx.resume.cloned() else {
            return Ok(());
        };
        let facts = ctx.context.facts();
        let woken = super::wake::land_tool_wake(ctx.session, facts, req)
            .await
            .map_err(TurnExit::Failed)?;
        ctx.out.root_id = Some(woken.root_id);
        ctx.out.fresh_root = false;
        ctx.out.replaces = woken.old_child;
        Ok(())
    })
}

/// `open-turn`：落本幕的锚（用户格 / 恢复格 / `round` 格）与配置。
fn run_open_turn<'a, 'b>(
    ctx: &'a mut OpeningCtx<'b>,
) -> Pin<Box<dyn Future<Output = Result<(), TurnExit>> + Send + 'a>> {
    Box::pin(async move {
        let (kind, branch_of) = match ctx.resume.as_ref() {
            // `retry` 的子支重来：靶子必须是轮骨架，但仍按新轮落用户格。
            Some(req) if matches!(req.action, ResumeAction::RetrySubtree) => {
                super::wake::retry_subtree(ctx.session, &req.target_id)
                    .await
                    .map_err(TurnExit::Failed)?;
                (OpeningKind::NewTurn, Some(req.target_id.clone()))
            }
            // 工具恢复：唤醒请求已由 `resume-turn` 落好，这里只落恢复格。
            Some(req) => (
                OpeningKind::Resume(super::drive::resume_kind(&req.action)),
                None,
            ),
            None => (OpeningKind::NewTurn, None),
        };
        let root = ctx.out.root_id.take();
        let replaces = ctx.out.replaces.take();
        let actor = symbio_sdk::authz::request_principal(ctx.request_ctx.as_ref());
        let anchor = Anchor {
            root,
            replaces,
            branch_of,
        };
        *ctx.out = open(ctx.context, ctx.turn, ctx.req, kind, anchor, &actor)?;
        Ok(())
    })
}

/// 落一幕的锚。同步：只有事实写入与纯读，没有跨边界往返。
pub(crate) fn open(
    context: &SessionContext,
    turn: &mut TurnState,
    req: &TurnRequest,
    kind: OpeningKind,
    anchor: Anchor,
    actor: &str,
) -> Result<Opened, TurnExit> {
    let Anchor {
        root,
        replaces,
        branch_of,
    } = anchor;
    let store = context.facts().ok_or_else(|| {
        TurnExit::Failed(PluginError::InternalError(
            "执行需要持久会话（临时会话无事实源）".into(),
        ))
    })?;
    // ── 行表是开幕的前提（DE-015 §3.4：装载失败即拒绝启动，不许部分成功）──────
    let frozen = super::super::loop_rows::table().map_err(TurnExit::Failed)?;
    let snapshot = store.range(Seq::new(0));
    let (turn_no, user_seq, user_text, round, anchor) = match kind {
        OpeningKind::Resume(kind) => {
            let (turn_no, user_seq) = super::super::turn_exec::last_open_turn(&snapshot)
                .ok_or_else(|| {
                    TurnExit::Failed(PluginError::InternalError(
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
            let stage = StageFact::resume(turn.round(), kind, &ordinal.to_string())
                .with_replaces(replaces);
            let seqs =
                stage_events::land_with_seqs(store.as_ref(), turn_no, user_seq, "system", &[stage])
                    .map_err(|e| {
                        TurnExit::Failed(PluginError::InternalError(format!(
                            "恢复阶段事实落格失败：{e:?}"
                        )))
                    })?;
            let drive_seq = seqs.first().copied().unwrap_or(0);
            (
                turn_no,
                user_seq,
                last_user_message(context).1,
                turn.round(),
                Some(drive_seq),
            )
        }
        OpeningKind::NewTurn => {
            let turn_no = count_user_messages(&snapshot, &CHAT_VOCAB, &MESSAGE_VOCAB);
            let (node_id, text, meta) = last_user_message(context);
            let mut payload = serde_json::json!({ "text": text, "id": node_id });
            if let Some(meta) = meta.filter(|m| !m.is_null()) {
                payload["meta"] = meta;
            }
            let seq = store
                .append(
                    Event::pending(
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
                    TurnExit::Failed(PluginError::InternalError(format!("用户格落格失败：{e:?}")))
                })?;
            (
                turn_no,
                seq.value(),
                text,
                turn.round(),
                Some(seq.value()),
            )
        }
        OpeningKind::Continue => {
            // 轮次由**那一格**回答（`turn_next_round`）：`continue-turn` 的前件与
            // 这里读的是同一个视图，不在这里按「当前轮次 + 1」再数一遍。
            let round =
                super::super::loop_rows::next_stage_round(&snapshot, turn.turn_no, turn.now_ms)
                    .ok_or_else(|| {
                        TurnExit::Failed(PluginError::InternalError(
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
    // ── 本幕的根 ─────────────────────────────────────────────────────────
    //
    // 续写幕复用被续写轮的根（`root` 由 `resume-turn` 交回）；新轮现开一个。
    // `fresh_root` 同时回答「要不要重发流式骨架」——续写幕那个骨架早已发过。
    let (root_id, fresh_root) = match root {
        Some(root) => (root, false),
        None => (symbio_sdk::llm::turn::llm_short_id(), true),
    };
    if let Some(of) = branch_of {
        store
            .append(
                Event::pending(
                    format!("branch-{root_id}"),
                    symbio_plugin_api::events::EVENT_TURN_BRANCH,
                    Entity::Turn,
                    Verb::Asserted,
                    turn_no,
                    actor,
                )
                .with_produced_by(user_seq)
                .with_payload(serde_json::json!({ "of": of, "into": root_id })),
            )
            .map_err(|e| {
                TurnExit::Failed(PluginError::InternalError(format!(
                    "分支事实落格失败：{e:?}"
                )))
            })?;
    }
    // ── 本幕的锚 ─────────────────────────────────────────────────────────
    let (drive_seq, round_landed) = match anchor {
        Some(seq) => (seq, false),
        None => {
            let seqs = stage_events::land_with_seqs(
                store.as_ref(),
                turn_no,
                user_seq,
                "system",
                &[StageFact::request_round(round, &round.to_string())],
            )
            .map_err(|e| {
                TurnExit::Failed(PluginError::InternalError(format!(
                    "轮次阶段事实落格失败：{e:?}"
                )))
            })?;
            (seqs.first().copied().unwrap_or(0), true)
        }
    };
    // ── 本次 drive 的 `now` = 刚落下这条唤醒事实的 `ts`（判据 12）─────────────
    let now_ms = super::super::loop_rows::wake_now_ms(&store.range(Seq::new(drive_seq)), drive_seq);
    // ── 收租：上一段生命里派出去、没回来的调用 ─────────────────
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
            frozen.ports(),
            tail,
            stale_turn,
            anchor,
            now_ms,
        ) {
            symbio_plugin_api::plugin_warn!("session", "[Loop] 租约看护失败（继续开新幕）: {e:?}");
        }
    }
    let settings = &context.view.settings;
    store
        .append(
            Event::pending(
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
                "readyset_cap": settings.task_cap,
                "skill_compile_enabled": settings.skill_compile_enabled,
                "skill_fast_path": settings.skill_fast_path,
                "is_persistent": context.view.persistent,
            })),
        )
        .map_err(|e| {
            TurnExit::Failed(PluginError::InternalError(format!(
                "配置事实落格失败：{e:?}"
            )))
        })?;
    if !round_landed {
        stage_events::land(
            store.as_ref(),
            turn_no,
            user_seq,
            "system",
            &[StageFact::request_round(round, &drive_seq.to_string())],
        )
        .map_err(|e| {
            TurnExit::Failed(PluginError::InternalError(format!(
                "轮次阶段事实落格失败：{e:?}"
            )))
        })?;
    }
    turn.turn_no = turn_no;
    turn.user_seq = user_seq;
    turn.drive_seq = drive_seq;
    turn.now_ms = now_ms;
    turn.facts = store.range(Seq::new(0));
    Ok(Opened {
        user_text,
        root_id: Some(root_id),
        fresh_root,
        replaces: None,
    })
}

/// 幕的种类（测试与驱动只认三种：新轮 / 恢复 / 再接一幕）。
#[derive(Clone, Copy)]
pub(crate) enum OpeningKind {
    NewTurn,
    Resume(&'static str),
    Continue,
}

/// 再接一幕（`continue-turn` 行的动作）：只落 `round` 格与配置，不碰用户格。
pub(crate) fn continue_stage(
    context: &SessionContext,
    turn: &mut TurnState,
    req: &TurnRequest,
) -> Result<(), TurnExit> {
    open(
        context,
        turn,
        req,
        OpeningKind::Continue,
        Anchor::default(),
        "system",
    )
    .map(|_| ())
}

/// 最近一条**用户消息节点**的 `(id, 正文, meta)`。
fn last_user_message(
    context: &SessionContext,
) -> (Option<String>, String, Option<serde_json::Value>) {
    context
        .messages
        .iter()
        .rev()
        .find(|m| m.node_role() == Some(NodeRole::User))
        .map(|m| {
            let text = match &m.content {
                Some(symbio_sdk::chat::chat_node::MessageContent::Text(t)) => t.clone(),
                _ => String::new(),
            };
            (Some(m.id.clone()), text, m.meta.clone())
        })
        .unwrap_or((None, String::new(), None))
}

/// 本次请求的用户发言（驱动在非首幕也要它；不落进任何一格）。
pub(crate) fn current_user_text(context: &SessionContext) -> String {
    last_user_message(context).1
}

/// 续写幕要替换的旧结果子节点——**从恢复格读回**（不再在 `TurnState` 上存一份）。
pub(crate) fn resume_replaces(turn: &TurnState) -> Option<String> {
    let anchor = Seq::new(turn.drive_seq);
    turn.facts
        .iter()
        .rev()
        .find(|e| {
            e.seq.map(|s| s >= anchor).unwrap_or(false)
                && e.kind == EVENT_TURN_STAGE
                && e.payload.get("stage").and_then(|v| v.as_str()) == Some("resume")
        })
        .and_then(|e| e.payload.get("replaces").and_then(|v| v.as_str()))
        .map(str::to_string)
}

/// 租约看护的扫描窗口（条）——**由声明回答**。
fn lease_window() -> usize {
    super::super::loop_rows::scan_bound().unwrap_or(0)
}

/// 收租窗口的切片：快照的**尾部** [`lease_window`] 条。
pub(crate) fn lease_tail(snapshot: &[Event]) -> &[Event] {
    &snapshot[snapshot.len().saturating_sub(lease_window())..]
}

#[cfg(test)]
#[path = "open_turn.test.rs"]
mod tests;
