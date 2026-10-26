//! 唤醒（d）：用户动作在开幕之前落成一条**工具调用请求**。
//!
//! ## 与旧 `resume.rs` 的区别
//!
//! 旧路径在这里**代替行表**走完了授权与派发：它自己调执行层、自己写结果节点、
//! 自己关 `request` 的账——一条绕开义务环的旁路。本模块只做一件事：把用户的
//! 动作翻译成一笔**唤醒请求**（`request(port=tool)`，载荷带 `resume` 标记），
//! 之后由行表自己的 `authorize-tool` / `dispatch-tools` / `deny-tools` 各行其责。
//!
//! 请求落在幕锚**之前**（开幕在 `begin_drive`，本模块跑在它前面）：`turn_head`
//! 的请求面按 seq 判「有没有终局」，不看幕锚，所以它照样看得见这一笔。
//!
//! ## 三种出口
//!
//! - [`WakeOutcome::Woken`]：唤醒请求已入格，本次请求是**续写幕**（复用原轮号、
//!   复用原 Turn 根节点）；
//! - [`WakeOutcome::NewTurn`]：`RetrySubtree`——按普通新轮重来，被重试的那一支
//!   **不删**，它成为同一问题的另一个候选（[D-173](../../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）；
//! - [`WakeOutcome::Done`]：`RetryCompaction` 等不产生对话轮次的动作，直接收尾。

use super::ChatOrchestrator;
use crate::symbio_core::chat_node::{ChatNode, NodeStatus, ResumeAction, ResumeRequest};
use crate::symbio_core::{llm_short_id, run_ops, RemoveNodesRequest, Seq};
use crate::symbio_core::{ExecAbortSignal, PluginError, PluginInvokeRequest};
use crate::{plugin_info, plugin_warn};
use serde_json::{json, Value};
use std::sync::Arc;

use super::super::tool_dispatch::{ToolPort, WakeIntent, WakeSpec};
use super::super::turn_session::LoopSession;

/// 恢复请求的处理结果。
pub(crate) enum WakeOutcome {
    /// 唤醒请求已入格：续写同一轮，复用原 Turn 根节点。
    Woken {
        /// 续写幕的事实名（`turn.stage{stage:"resume"}` 的 `result`）。
        kind: &'static str,
        /// 被续写轮的 Turn 根节点 id（流式帧与结果节点都挂在它下面）。
        root_id: String,
        /// 被替换的旧结果子节点：新结果落格后删除，避免同一笔调用出现两条结果。
        old_child: Option<String>,
    },
    /// 按普通新轮重来（`RetrySubtree`）：被重试的那一支**不删**，它成为同一问题的
    /// 另一个候选（[D-173](../../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
    NewTurn {
        /// 被重试那一支的 Turn 根节点 id——新支开出来时按它登记「是哪一支的另一版本」
        /// （`turn.branch` 的 `of`）。
        retried_root: String,
    },
    /// 本请求不产生对话轮次（`RetryCompaction`），直接收尾。
    Done,
}

/// 处理会话恢复请求。
pub(crate) async fn process_wake(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn PluginInvokeRequest>,
    abort: &ExecAbortSignal,
    session: &Arc<LoopSession>,
    req: ResumeRequest,
) -> Result<WakeOutcome, PluginError> {
    match req.action {
        ResumeAction::RetrySubtree => retry_subtree(session, &req.target_id).await,
        // 压缩不是对话轮次：执行体在压缩流水线所在模块（那里才有切分 / 内核的私有面），
        // 完成后一律 `Done`。
        ResumeAction::RetryCompaction => {
            super::super::context::retry_compaction(
                orchestrator,
                ctx,
                abort,
                session,
                &req.target_id,
            )
            .await?;
            Ok(WakeOutcome::Done)
        }
        _ => land_tool_wake(session, req).await,
    }
}

/// `RetrySubtree`：按普通新轮重来，**被重试的那一支留在树里**。
///
/// 这里不做删除（`remove-nodes` + 一条 `message.removed`）。删除会把两件不同的事说成
/// 同一件：「这一次不再读了」（呈现，由选中态说）与「这一次没发生过」（事实，需要
/// `message.removed` 才成立）。而那次往返确实发生、确实产出过半截答案与真跑过的工具
/// ——删除等于毁掉一条已经写下的事实，还顺手让「同 parent 多候选」永远没有数据来源
/// （裁决见 [D-173](../../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
///
/// 本函数因此只做**校验与交身份**：靶子必须是一条轮骨架（`is_run()`，不看 `status`——
/// 中止的、失败的、甚至跑完的都可以重来），它的根 id 交给新幕去登记分支
/// （[`super::super::stage_exec`] 的 `prepare-turn`：那里才有新支的根 id）。
/// 「藏起来不读」归[分支投影](../../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")，
/// 一处是 [`crate::symbio_core::branch_hide_unselected`]——只有进模型的那条路调它。
async fn retry_subtree(session: &LoopSession, target_id: &str) -> Result<WakeOutcome, PluginError> {
    let messages = session
        .run_op::<Vec<ChatNode>>(run_ops::MESSAGES, ())
        .await?;

    let turn_exists = messages.iter().any(|m| m.id == target_id && m.is_run());
    if !turn_exists {
        return Err(PluginError::NotFound(format!(
            "未找到 Failed Turn 节点: {target_id}"
        )));
    }

    plugin_info!("session", "[Wake] RetrySubtree: {target_id} 另开一支");
    Ok(WakeOutcome::NewTurn {
        retried_root: target_id.to_string(),
    })
}

/// 工具调用的唤醒：把用户动作落成一笔 `request(port=tool)`。
///
/// 不在这里执行工具、不写授权 / 结果——那些归行表的授权行与派发行（d）。
async fn land_tool_wake(
    session: &LoopSession,
    req: ResumeRequest,
) -> Result<WakeOutcome, PluginError> {
    let messages = session
        .run_op::<Vec<ChatNode>>(run_ops::MESSAGES, ())
        .await?;

    let tc_idx = messages
        .iter()
        .position(|m| m.id == req.target_id && m.is_tool_call())
        .ok_or_else(|| PluginError::NotFound(format!("未找到工具调用节点: {}", req.target_id)))?;

    // 待替换的子节点：交互卡 / 失败结果 / 等待用户的结果。
    let child_idx = messages
        .iter()
        .position(|m| {
            m.parent_id.as_deref() == Some(&req.target_id)
                && (m.is_interaction()
                    || m.status == Some(NodeStatus::Failed)
                    || m.status == Some(NodeStatus::WaitingUserAction))
        })
        .ok_or_else(|| {
            PluginError::NotFound(format!("工具调用 {} 无可恢复的子节点", req.target_id))
        })?;

    let tc = &messages[tc_idx];
    let (tool_name, base_args) = extract_tool_context(&messages, tc_idx, child_idx);
    let old_child = Some(messages[child_idx].id.clone());
    let wire_id = tc.tool_call_id.clone().unwrap_or_default();
    // Turn 根：`flat` 档记在 `meta.turn_root`，`grouped` 档是 `parent_id`。
    // 两处都取不到（老数据 / 手工节点）时现开一个——续写幕必须有一个根可挂。
    let turn_root = crate::symbio_core::node_build::turn_root_of(tc)
        .map(str::to_string)
        .or_else(|| tc.parent_id.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(llm_short_id);

    let kind = super::drive::resume_kind(&req.action);
    let (args, intent) = wake_intent(&req, base_args);

    let facts = session
        .facts()
        .ok_or_else(|| PluginError::InternalError("工具恢复缺少事实轨".into()))?;
    let snapshot = facts.range(Seq::new(0));
    let (turn_no, user_seq) = super::super::turn_exec::last_open_turn(&snapshot)
        .ok_or_else(|| PluginError::NotFound("恢复找不到未收束的轮次".into()))?;
    // 幕锚此时还没落（`begin_drive` 在本次请求里才跑）⇒ 占位 0。三笔工具债不读
    // 带时间的视图，`now` 参与不了判据。
    let port = ToolPort::new(facts, turn_no, user_seq, 0);
    port.land_wake(&WakeSpec {
        kind,
        target: &req.target_id,
        tool: &tool_name,
        args: &args,
        wire_id: &wire_id,
        turn_root: &turn_root,
        intent: &intent,
    })
    .map_err(|e| PluginError::InternalError(format!("唤醒请求落格失败：{e:?}")))?;

    plugin_info!(
        "session",
        "[Wake] {kind}: tool `{tool_name}` queued as request(port=tool) for turn {turn_no}"
    );
    Ok(WakeOutcome::Woken {
        kind,
        root_id: turn_root,
        old_child,
    })
}

/// 唤醒 ⇒ 执行器交回的消息并入落库镜像。
///
/// 与普通轮次的 `extend` 不同：续写幕的新结果要**替换**旧结果子节点，调用卡本身
/// （id = 被恢复的调用节点）也已在历史里——按 id 整条替换，再由动作面落
/// `edit-nodes` / `remove-nodes`。不这样做的话，实时面看见的是一张被覆盖的旧卡，
/// 而落库面同时留着旧卡和旧结果，重建（只按 `artifact.added` 复刻）又是第三种形状。
///
/// 返回后 `turn.last_saved` 指向**新增消息的起点**：替换过的节点已由 `edit-nodes`
/// 落过，`persist_messages` 的尾部增量只该包含真正新增的那几条。
pub(crate) async fn merge_runner_messages(
    session: &LoopSession,
    context: &mut super::state::SessionContext,
    turn: &mut super::state::TurnState,
    messages: Vec<ChatNode>,
) -> Result<(), PluginError> {
    if let Some(old) = turn.resume_old_child.take() {
        if context.messages.iter().any(|m| m.id == old) {
            context.messages.retain(|m| m.id != old);
            session
                .run_op_unit(run_ops::REMOVE_NODES, RemoveNodesRequest { ids: vec![old] })
                .await?;
        }
    }

    let tail_start = context.messages.len();
    let mut edited: Vec<ChatNode> = Vec::new();
    for msg in messages {
        match context.messages.iter().position(|m| m.id == msg.id) {
            Some(idx) => {
                context.messages[idx] = msg.clone();
                edited.push(msg);
            }
            None => context.messages.push(msg),
        }
    }
    if !edited.is_empty() {
        session
            .run_op::<Vec<ChatNode>>(
                run_ops::EDIT_NODES,
                crate::symbio_core::EditNodesRequest {
                    nodes: edited,
                    frame: crate::symbio_core::EditFrame::Message,
                },
            )
            .await?;
    }
    turn.last_saved = tail_start;
    Ok(())
}

/// 用户动作 ⇒ 派发参数与语义（执行 / 直接产结果）。
fn wake_intent(req: &ResumeRequest, base_args: Value) -> (Value, WakeIntent) {
    match req.action {
        // 这三个动作真正重跑工具；参数按动作改写，执行层只知道「有一次调用要跑」。
        ResumeAction::Approve => {
            let mut args = base_args;
            if let Some(obj) = args.as_object_mut() {
                obj.insert("approved".into(), json!(true));
            }
            (args, WakeIntent::Execute)
        }
        ResumeAction::Retry => (base_args, WakeIntent::Execute),
        ResumeAction::Supply => {
            let mut args = base_args;
            if let (Some(obj), Some(supplied)) = (args.as_object_mut(), req.args.clone()) {
                if let Some(sup_obj) = supplied.as_object() {
                    for (k, v) in sup_obj {
                        obj.insert(k.clone(), v.clone());
                    }
                }
            }
            (args, WakeIntent::Execute)
        }
        // 拒绝 / 回答不执行工具，就地合成结果。
        ResumeAction::Reject => {
            let text = format!("用户拒绝执行: {}", req.reason.clone().unwrap_or_default());
            (
                base_args,
                WakeIntent::Result {
                    text,
                    ok: false,
                    denied: true,
                },
            )
        }
        ResumeAction::Answer => {
            let text = serde_json::to_string_pretty(&req.answer.clone().unwrap_or(json!(null)))
                .unwrap_or_default();
            (
                base_args,
                WakeIntent::Result {
                    text,
                    ok: true,
                    denied: false,
                },
            )
        }
        ResumeAction::RetrySubtree | ResumeAction::RetryCompaction => {
            plugin_warn!(
                "session",
                "[Wake] 不可达：RetrySubtree / RetryCompaction 已在顶层分发"
            );
            (base_args, WakeIntent::Execute)
        }
    }
}

/// 从 ToolCall 父节点和待恢复子节点中提取 `(tool_name, args)`。
///
/// - **交互子节点**（confirm / ask_user）：优先从 `meta.prompt` 取；
/// - **失败结果**（工具执行失败重试）：从 ToolCall 的 `meta.args` 或 `content` 解析。
///
/// 取不到工具名时**不给默认值**：默认一个别的工具名会让恢复真的去执行另一个工具，
/// 比「取不到上下文」更难查。
fn extract_tool_context(messages: &[ChatNode], tc_idx: usize, child_idx: usize) -> (String, Value) {
    let tc = &messages[tc_idx];
    let child = &messages[child_idx];

    if child.is_interaction() {
        if let Some(prompt) = child.meta.as_ref().and_then(|m| m.get("prompt")) {
            if let Some(tool_name) = prompt.get("tool_name").and_then(|v| v.as_str()) {
                let args = prompt.get("args").cloned().unwrap_or_else(|| json!({}));
                return (tool_name.to_string(), args);
            }
        }
    }

    let tool_name = tc.tool_name().unwrap_or_default().to_string();
    let args = tc
        .meta
        .as_ref()
        .and_then(|m| m.get("args"))
        .cloned()
        .unwrap_or_else(|| {
            tc.content
                .as_ref()
                .and_then(|c| c.to_text().parse::<Value>().ok())
                .unwrap_or_else(|| json!({}))
        });
    (tool_name, args)
}
