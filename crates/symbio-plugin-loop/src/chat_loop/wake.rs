//! 唤醒（d）：用户动作在开幕之前落成一条**工具调用请求**。
//!
//! ## 只把动作翻译成债
//!
//! 旧路径在这里**代替行表**走完了授权与派发：它自己调执行层、自己写结果节点、
//! 自己关 `request` 的账——一条绕开义务环的旁路。本模块只做一件事：把用户的
//! 动作翻译成一笔**唤醒请求**（`request(port=tool)`，载荷带 `resume` 标记），
//! 之后由行表自己的 `authorize-tool` / `dispatch-tool` / `deny-tools` 各行其责。
//!
//! 请求落在幕锚**之前**：`resume-turn` 这一行先落请求，`open-turn` 再落恢复格
//! （D-313 P2-1）。`turn_head` 的请求面按 seq 判「有没有终局」，不看幕锚，
//! 所以它照样看得见这一笔。
//!
//! ## 三种出口（现在是数据，不是分派）
//!
//! - [`Woken`]：唤醒请求已入格，本次请求是**续写幕**（复用原轮号、复用原 Turn
//!   根节点）；
//! - [`retry_subtree`]：`RetrySubtree`——按普通新轮重来，被重试的那一支**不删**，
//!   它成为同一问题的另一个候选（[D-173](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）；
//! - `RetryCompaction` 不是对话轮次，直接由编排层收尾（不经本模块）。

use std::sync::Arc;

use serde_json::{json, Value};
use symbio_kernel::event::Seq;
use symbio_kernel::plugin::PluginError;
use symbio_sdk::chat::chat_node::{ChatNode, NodeStatus, ResumeAction, ResumeRequest};
use symbio_sdk::llm::turn::llm_short_id;
use symbio_sdk::turn::dispatch::{run_ops, RemoveNodesRequest};
use {symbio_plugin_api::plugin_info, symbio_plugin_api::plugin_warn};

use super::super::session_port::SessionPort;
use super::super::tool_dispatch::{ToolPort, Wake, WakeSpec};

use crate::session_port;

/// 唤醒请求已入格：续写幕要复用的一切。
pub(crate) struct Woken {
    /// 被续写轮的 Turn 根节点 id（流式帧与结果节点都挂在它下面）。
    pub(crate) root_id: String,
    /// 被替换的旧结果子节点：新结果落格后删除，避免同一笔调用出现两条结果。
    pub(crate) old_child: Option<String>,
}

/// `RetrySubtree`：按普通新轮重来，**被重试的那一支留在树里**。
///
/// 这里不做删除（`remove-nodes` + 一条 `message.removed`）。删除会把两件不同的事说成
/// 同一件：「这一次不再读了」（呈现，由选中态说）与「这一次没发生过」（事实，需要
/// `message.removed` 才成立）。而那次往返确实发生、确实产出过半截答案与真跑过的工具
/// ——删除等于毁掉一条已经写下的事实，还顺手让「同 parent 多候选」永远没有数据来源
/// （裁决见 [D-173](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）。
///
/// 本函数因此只做**校验与交身份**：靶子必须是一条轮骨架（`is_run()`，不看 `status`——
/// 中止的、失败的、甚至跑完的都可以重来），它的根 id 交给新幕去登记分支
/// （`prepare-turn` 那一行：那里才有新支的根 id）。
/// 「藏起来不读」归[分支投影](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")，
/// 一处是 [`symbio_sdk::projection::branch::branch_hide_unselected`]——只有进模型的那条路调它。
pub(crate) async fn retry_subtree(
    session: &Arc<dyn SessionPort>,
    target_id: &str,
) -> Result<(), PluginError> {
    let messages =
        session_port::call::<Vec<ChatNode>>(session.as_ref(), run_ops::MESSAGES, ()).await?;

    let turn_exists = messages.iter().any(|m| m.id == target_id && m.is_run());
    if !turn_exists {
        return Err(PluginError::NotFound(format!(
            "未找到 Failed Turn 节点: {target_id}"
        )));
    }

    plugin_info!("session", "[Wake] RetrySubtree: {target_id} 另开一支");
    Ok(())
}

/// 工具调用的唤醒：把用户动作落成一笔 `request(port=tool)`。
///
/// 不在这里执行工具、不写授权 / 结果——那些归行表的授权行与派发行（d）。
pub(crate) async fn land_tool_wake(
    session: &Arc<dyn SessionPort>,
    facts: Option<Arc<dyn symbio_kernel::store::Store<Event = symbio_kernel::event::Event>>>,
    req: ResumeRequest,
) -> Result<Woken, PluginError> {
    let messages =
        session_port::call::<Vec<ChatNode>>(session.as_ref(), run_ops::MESSAGES, ()).await?;

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
    // Turn 根：`flat` 档记在 `meta.turn_root`，`grouped` 档是 `parent_id`。
    // 两处都取不到（老数据 / 手工节点）时现开一个——续写幕必须有一个根可挂。
    let turn_root = symbio_sdk::turn::node_build::turn_root_of(tc)
        .map(str::to_string)
        .or_else(|| tc.parent_id.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(llm_short_id);

    let kind = super::drive::resume_kind(&req.action);
    let (args, wake) = wake_intent(&req, base_args);

    let facts = facts.ok_or_else(|| PluginError::InternalError("工具恢复缺少事实轨".into()))?;
    let snapshot = facts.range(Seq::new(0));
    let (turn_no, user_seq) = super::super::turn_exec::last_open_turn(&snapshot)
        .ok_or_else(|| PluginError::NotFound("恢复找不到未收束的轮次".into()))?;
    // 幕锚此时还没落（`open-turn` 在本请求里随后才跑）⇒ 占位 0。三笔工具债不读
    // 带时间的视图，`now` 参与不了判据。
    let port = ToolPort::new(facts, turn_no, user_seq, 0);
    port.land_wake(&WakeSpec {
        kind,
        target: &req.target_id,
        tool: &tool_name,
        args: &args,
        turn_root: &turn_root,
        wake: &wake,
    })
    .map_err(|e| PluginError::InternalError(format!("唤醒请求落格失败：{e:?}")))?;

    plugin_info!(
        "session",
        "[Wake] {kind}: tool `{tool_name}` queued as request(port=tool) for turn {turn_no}"
    );
    Ok(Woken {
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
    session: &Arc<dyn SessionPort>,
    context: &mut super::state::SessionContext,
    turn: &mut super::state::TurnState,
    messages: Vec<ChatNode>,
) -> Result<(), PluginError> {
    if let Some(old) = super::open_turn::resume_replaces(turn) {
        // 旧交互卡与新结果卡是**同一个节点身份**（结果 id 由调用 id 派生，D-312）：
        // 新结果覆盖它时不能再落 `remove-nodes`，否则被删的是刚写下的答复，
        // 重建面按 `message.removed` 把它当成永久消失（T27 的「有请求无响应」）。
        // 仅当新结果没有接管这个 id（确实没有替换节点）时才删旧卡。
        let superseded = messages.iter().any(|m| m.id == old);
        if !superseded && context.messages.iter().any(|m| m.id == old) {
            context.messages.retain(|m| m.id != old);
            session_port::call_unit(
                session.as_ref(),
                run_ops::REMOVE_NODES,
                RemoveNodesRequest { ids: vec![old] },
            )
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
        session_port::call::<Vec<ChatNode>>(
            session.as_ref(),
            run_ops::EDIT_NODES,
            symbio_sdk::turn::dispatch::EditNodesRequest {
                nodes: edited,
                frame: symbio_sdk::turn::dispatch::EditFrame::Message,
            },
        )
        .await?;
    }
    turn.last_saved = tail_start;
    Ok(())
}

/// 用户动作 ⇒ 派发参数与语义（执行 / 直接产结果）。
fn wake_intent(req: &ResumeRequest, base_args: Value) -> (Value, Wake) {
    match req.action {
        // 这三个动作真正重跑工具；参数按动作改写，执行层只知道「有一次调用要跑」。
        ResumeAction::Approve => {
            let mut args = base_args;
            if let Some(obj) = args.as_object_mut() {
                obj.insert("approved".into(), json!(true));
            }
            (args, Wake::Execute)
        }
        ResumeAction::Retry => (base_args, Wake::Execute),
        ResumeAction::Supply => {
            let mut args = base_args;
            if let (Some(obj), Some(supplied)) = (args.as_object_mut(), req.args.clone()) {
                if let Some(sup_obj) = supplied.as_object() {
                    for (k, v) in sup_obj {
                        obj.insert(k.clone(), v.clone());
                    }
                }
            }
            (args, Wake::Execute)
        }
        // 拒绝 / 回答不执行工具，就地合成结果。
        ResumeAction::Reject => {
            let text = format!("用户拒绝执行: {}", req.reason.clone().unwrap_or_default());
            (
                base_args,
                Wake::Result {
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
                Wake::Result {
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
            (base_args, Wake::Execute)
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
