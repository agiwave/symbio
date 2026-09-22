//! 会话恢复（Resume）
//!
//! 由 `chat_loop::run_chat_loop` 在 turn 循环前调用，处理用户的恢复操作：
//! - `RetryTurn`：LLM 失败重试（删除 Failed Turn 及其所有子孙节点，重新走 LLM 请求）
//! - `RetryCompaction`：压缩失败重试（删除 Failed 压缩节点，重新执行一次压缩）
//! - `Retry` / `Approve` / `Reject` / `Supply` / `Answer`：工具调用恢复
//!   （删除旧子节点 → 重新执行工具或生成结果 → 创建新子节点）
//!
//! ## 统一设计：删除-重建模式
//!
//! 所有非普通消息场景都遵循"删除旧消息 → 重新执行 → 生成新消息"的统一模式：
//! - **RetryTurn**：删除 Failed Turn 及其所有子孙节点 → chat_loop 从 session 重新加载 → LLM 请求
//! - **RetryCompaction**：删除 Failed 压缩节点 → 重新压缩 → 新压缩节点落库
//! - **工具调用恢复**：删除旧子节点 → 重新执行工具 → 创建新结果子节点
//!
//! **删除-重建不适用于历史本身**：压缩失败从不丢消息（见 `CompressionFailure`），
//! 重试只是"再试一次 LLM 摘要"，历史一条不动。
//!
//! ## 工具调用恢复核心流程
//!
//! 1. 从会话存储加载消息，定位 ToolCall 父节点 + 待恢复子节点
//! 2. 提取 tool_name / base_args
//! 3. 根据 action 执行：approve/retry/supply 调用 `execute_tool_async`；reject/answer 直接生成结果
//! 4. 删除旧子节点（广播删除帧：`status = removed`）
//! 5. 创建新 Text 结果子节点（广播完整消息：正文首次上线）
//! 6. 更新 ToolCall 父节点状态（广播完整消息：args 正文可能已被 supply 改写）
//! 7. 持久化（`replace_messages`）
//! 8. 成功 → `ResumeOutcome::Continue`（turn 循环续写）；失败 → `ResumeOutcome::Done`（退出等下次 resume）
//!
//! ## CAPABILITY_VISITOR
//!
//! 已由 agent chat handler（`agent/handlers/chat.rs:302`）通过 `fetch_tools_with_manager` 设置到 ctx，
//! `execute_tool_async` 直接复用，无需 session 层重复 `prepare_capability_manager`。

use super::chat_loop::ChatOrchestrator;
use super::chat_session::ChatSession;
use super::tool_executor::{apply_not_executed, execute_tool_async};
use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType, ResumeAction,
    ResumeRequest,
};
use crate::symbio_core::turn::{emit_message, emit_removed, emit_state, short_id};
use crate::symbio_core::{AbortSignal, EventSink, InvokeRequest, PluginError};
use crate::{plugin_error, plugin_info};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::sync::Arc;

/// resume 执行结果：继续 turn 循环 或 退出等下次 resume。
pub enum ResumeOutcome {
    /// 恢复成功，turn 循环应继续（历史已含新工具结果，或 Failed Turn 已删除）。
    Continue,
    /// 恢复失败或 reject/answer 已终态，turn 循环应退出（留 Failed 等下次 resume）。
    Done,
}

/// 处理会话恢复。
///
/// 由 `run_chat_loop` 在 turn 循环前调用。根据 `req.action` 分发：
/// - `RetryTurn`：LLM 失败重试，调用 `process_retry_turn`
/// - `RetryCompaction`：压缩失败重试，调用 `chat_loop::compress::retry_compaction`
/// - 其他：工具调用恢复，调用 `process_tool_resume_action`
///
/// `ctx` 应已由 agent chat handler 设置好 `CAPABILITY_VISITOR`。
/// `channel` 是 run_chat_loop 的主 channel，用于广播 Delete/Update 事件。
/// `session` 用于加载/持久化会话消息。
pub async fn process_resume(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn InvokeRequest>,
    sink: &EventSink,
    abort: &AbortSignal,
    session: &Arc<dyn ChatSession>,
    req: ResumeRequest,
) -> Result<ResumeOutcome, PluginError> {
    match req.action {
        ResumeAction::RetryTurn => process_retry_turn(session, sink, &req).await,
        // 压缩失败重试：执行体在压缩流水线所在模块（那里才有 `SessionContext`
        // 与切分/内核的私有面）。完成后一律 `Done`——压缩不是对话轮次，没有
        // "续写"可言；与其它 resume 出口一样，失败节点（若有）留下等下次 resume。
        ResumeAction::RetryCompaction => {
            super::chat_loop::compress::retry_compaction(
                orchestrator,
                ctx,
                sink,
                abort,
                session,
                &req.target_id,
            )
            .await?;
            Ok(ResumeOutcome::Done)
        }
        _ => process_tool_resume_action(orchestrator, ctx, sink, abort, session, req).await,
    }
}

/// 处理 LLM 失败重试（RetryTurn）。
///
/// 流程：
/// 1. 加载会话消息
/// 2. 定位 Failed Turn（`target_id` 指向，msg_type=Turn）
/// 3. BFS 收集所有子孙节点 id（含 target_id 自身）
/// 4. 从 messages 中删除这些节点
/// 5. `replace_messages` 持久化
/// 6. 广播 Delete 事件（每个被删除的消息）
/// 7. 返回 `Continue`：chat_loop 从 session 重新加载消息（已删除 Failed Turn），重新走 LLM 请求
///
/// 注意：本函数不重新执行 LLM，仅清理 Failed Turn。LLM 请求由 chat_loop 的 turn 循环
/// 在加载 session 历史后自动发起（用户原消息仍在历史中）。
async fn process_retry_turn(
    session: &Arc<dyn ChatSession>,
    sink: &EventSink,
    req: &ResumeRequest,
) -> Result<ResumeOutcome, PluginError> {
    // 1. 加载会话消息
    let mut messages = session.get_messages().await?;

    // 2. 定位 Failed Turn（target_id 指向，msg_type=Turn）
    let turn_exists = messages
        .iter()
        .any(|m| m.id == req.target_id && m.msg_type == Some(MessageType::Turn));
    if !turn_exists {
        return Err(PluginError::NotFound(format!(
            "未找到 Failed Turn 节点: {}",
            req.target_id
        )));
    }

    // 3. BFS 收集所有子孙节点 id（含 target_id 自身）
    let mut to_delete: HashSet<String> = HashSet::new();
    to_delete.insert(req.target_id.clone());
    let mut queue: Vec<String> = vec![req.target_id.clone()];
    while let Some(parent_id) = queue.pop() {
        for m in &messages {
            if m.parent_id.as_deref() == Some(&parent_id) && to_delete.insert(m.id.clone()) {
                queue.push(m.id.clone());
            }
        }
    }

    // 4. 从 messages 中删除这些节点（保留删除前副本用于广播）
    let deleted_messages: Vec<ChatMessage> = messages
        .iter()
        .filter(|m| to_delete.contains(&m.id))
        .cloned()
        .collect();
    messages.retain(|m| !to_delete.contains(&m.id));

    // 5. 持久化
    session.replace_messages(messages).await?;

    // 6. 广播删除（每个被删除的消息一条 `status = removed` 的删除帧）
    for msg in &deleted_messages {
        emit_removed(sink, &msg.id).await;
    }

    plugin_info!(
        "session",
        "[Resume] RetryTurn: deleted {} messages (turn {} and descendants), continuing chat_loop",
        deleted_messages.len(),
        req.target_id
    );

    // 7. Continue：chat_loop 从 session 重新加载消息（已删除 Failed Turn），重新走 LLM 请求
    Ok(ResumeOutcome::Continue)
}

/// 处理工具调用恢复（Retry/Approve/Reject/Supply/Answer）。
///
/// 删除-重建模式：删除旧子节点 → 重新执行工具或生成结果 → 创建新子节点 → 更新父节点状态。
async fn process_tool_resume_action(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn InvokeRequest>,
    sink: &EventSink,
    abort: &AbortSignal,
    session: &Arc<dyn ChatSession>,
    req: ResumeRequest,
) -> Result<ResumeOutcome, PluginError> {
    // 1. 加载会话消息
    let mut messages = session.get_messages().await?;

    // 2. 定位 ToolCall 父节点
    let tc_idx = messages
        .iter()
        .position(|m| m.id == req.target_id && m.msg_type == Some(MessageType::ToolCall));
    let tc_idx = match tc_idx {
        Some(i) => i,
        None => {
            return Err(PluginError::NotFound(format!(
                "未找到工具调用节点: {}",
                req.target_id
            )));
        }
    };

    // 3. 定位待恢复子节点（user_prompt 或 Failed/WaitingUserAction Text）
    let child_idx = messages.iter().position(|m| {
        m.parent_id.as_deref() == Some(&req.target_id)
            && (m.msg_type == Some(MessageType::UserPrompt)
                || m.status == Some(MessageStatus::Failed)
                || m.status == Some(MessageStatus::WaitingUserAction))
    });
    let child_idx = match child_idx {
        Some(i) => i,
        None => {
            return Err(PluginError::NotFound(format!(
                "工具调用 {} 无可恢复的子节点",
                req.target_id
            )));
        }
    };

    // 4. 提取 tool_name / base_args
    let (tool_name, base_args, _failure_kind) = extract_tool_context(&messages, tc_idx, child_idx);
    let old_child_id = messages[child_idx].id.clone();

    // 5. 根据 action 执行。
    //
    // 会**真正重跑工具**的三个 action（approve / retry / supply）先把父 ToolCall
    // 置「运行中」再执行——与 `process_tool_calls_async` 同一条规则：ToolCall 节点
    // 覆盖整段执行窗口，而不是"参数齐了就结束"（`docs/node-state-streaming.md` §2.2）。
    // 否则用户点下「批准执行」后画面毫无变化，直到结果突然出现，无法判断是否在跑。
    //
    // `meta.started_at` 与 `emit_tool_running` 同源：前端据此显示"已运行 47s"。
    //
    // reject / answer 不执行工具（就地生成结果），下发"运行中"只会闪一帧，故排除。
    //
    // 完整快照：从存储转写取父 ToolCall 副本应用「运行中」，整条广播。
    // `messages` 本身**不动**（`Streaming` 是瞬态状态，持久层拒绝落盘——见
    // `chat_session.rs::ensure_durable_states`）；存储里的父节点保持恢复前的
    // 终态，直到步骤 9 以真正的终态整条替换。
    if matches!(
        &req.action,
        ResumeAction::Approve | ResumeAction::Retry | ResumeAction::Supply
    ) {
        match messages.iter().find(|m| m.id == req.target_id) {
            Some(tc) => {
                let mut running = tc.clone();
                running.status = Some(MessageStatus::Streaming);
                let mut meta = running.meta.clone().unwrap_or_else(|| json!({}));
                if let Some(obj) = meta.as_object_mut() {
                    obj.insert("started_at".into(), json!(crate::symbio_core::now_ms()));
                }
                running.meta = Some(meta);
                emit_state(sink, running).await;
            }
            None => {
                plugin_error!(
                    "session",
                    "[Resume] 运行态广播: 转写中不存在工具调用 {}（协议违例），跳过",
                    req.target_id
                );
            }
        }
    }

    let (final_result_text, final_success, new_tc_args) = match req.action {
        ResumeAction::RetryTurn => unreachable!("RetryTurn 已在顶层分发，此处不可达"),
        ResumeAction::RetryCompaction => {
            unreachable!("RetryCompaction 已在顶层分发，此处不可达")
        }
        ResumeAction::Reject => {
            let msg = format!("用户拒绝执行: {}", req.reason.unwrap_or_default());
            (msg, false, None)
        }
        ResumeAction::Answer => {
            let answer_str = serde_json::to_string_pretty(&req.answer.unwrap_or(json!(null)))
                .unwrap_or_default();
            (answer_str, true, None)
        }
        ResumeAction::Approve => {
            let mut a = base_args.clone();
            if let Some(obj) = a.as_object_mut() {
                obj.insert("approved".into(), json!(true));
            }
            let (res, success) =
                reexecute_tool(orchestrator, ctx, abort, &tool_name, a, &req.target_id).await;
            (res, success, None)
        }
        ResumeAction::Retry => {
            let (res, success) = reexecute_tool(
                orchestrator,
                ctx,
                abort,
                &tool_name,
                base_args.clone(),
                &req.target_id,
            )
            .await;
            (res, success, None)
        }
        ResumeAction::Supply => {
            let mut a = base_args.clone();
            if let (Some(obj), Some(supplied)) = (a.as_object_mut(), req.args.clone()) {
                if let Some(sup_obj) = supplied.as_object() {
                    for (k, v) in sup_obj {
                        obj.insert(k.clone(), v.clone());
                    }
                }
            }
            let (res, success) = reexecute_tool(
                orchestrator,
                ctx,
                abort,
                &tool_name,
                a.clone(),
                &req.target_id,
            )
            .await;
            (res, success, Some(a))
        }
    };

    if abort.is_aborted() {
        // 父 ToolCall 在步骤 5 被广播成 `Streaming`（"运行中"）——那是**广播帧**
        // （`Streaming` 是瞬态状态，持久层拒绝落盘，见 `chat_session.rs::
        // ensure_durable_states`），前端此刻显示"运行中"。若在这里直接返回，
        // 广播层没有任何后续机制会再碰它，前端就永远显示"运行中"
        // （违反 `docs/node-state-streaming.md` §8.11）。存储里的父节点仍是
        // 恢复前的终态（Failed），故此处落库的收口补丁同样是终态。
        plugin_info!("session", "[Resume] aborted during tool re-execution");
        let parent_update = finalize_aborted_parent(&mut messages, tc_idx, &req.target_id);
        if let Err(e) = session.replace_messages(messages).await {
            plugin_error!(
                "session",
                "[Resume] 中止收口落库失败（前端仍会收到定稿补丁）: {}",
                e
            );
        }
        emit_state(sink, parent_update).await;
        return Ok(ResumeOutcome::Done);
    }

    // 6. 删除旧子节点
    messages.remove(child_idx);

    // 7. 创建新子节点（新 id，Text 类型，工具结果）
    let new_child_id = short_id();
    let now_ts = crate::symbio_core::now_ms();
    let new_child = ChatMessage {
        id: new_child_id.clone(),
        parent_id: Some(req.target_id.clone()),
        role: Some(MessageRole::Tool),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text(final_result_text.clone())),
        status: Some(if final_success {
            MessageStatus::Completed
        } else {
            MessageStatus::Failed
        }),
        meta: Some(json!({"success": final_success})),
        timestamp: Some(now_ts),
        ..Default::default()
    };
    messages.push(new_child.clone());

    // 8. 更新 ToolCall 父节点状态（child_idx 删除后调整 tc_idx）
    let tc_idx_adjusted = if tc_idx > child_idx {
        tc_idx - 1
    } else {
        tc_idx
    };
    let final_parent_status = if final_success {
        MessageStatus::Completed
    } else {
        MessageStatus::Failed
    };
    let tc = &mut messages[tc_idx_adjusted];
    tc.status = Some(final_parent_status);
    tc.meta = Some(if final_success {
        json!({"success": true})
    } else {
        json!({
            "success": false,
            "failure_kind": "error",
            "tool_name": tool_name,
            "args": base_args
        })
    });
    // 失败时不把 error 文本挂到父 ToolCall —— 子 Text 节点的 content 已包含错误详情，
    // 若父节点也挂 error，前端 MessageNode 会在父和子两处都渲染同一份错误文本，造成重复。
    // 父节点仅靠 status=Failed + meta.failure_kind 表达失败状态（供 resume 定位）。
    tc.error = None;
    // supply 时更新 ToolCall 的 content（args JSON）
    if let Some(new_args) = new_tc_args {
        tc.content = Some(MessageContent::Text(
            serde_json::to_string(&new_args).unwrap_or_default(),
        ));
    }
    let updated_parent = tc.clone();

    // 9. 持久化（replace_messages：删除旧子 + 新增新子 + 更新父节点）
    session.replace_messages(messages).await?;

    // 10. 广播：旧子删除（`status = removed`）+ 新子完整消息（正文首次上线）
    //     + 父节点完整消息（supply 可能改写了它的 args 正文 ⇒ 整条替换）
    emit_removed(sink, &old_child_id).await;
    emit_message(sink, new_child).await;
    emit_message(sink, updated_parent).await;

    // 11. 成功 → Continue；失败 → Done
    if final_success {
        plugin_info!(
            "session",
            "[Resume] tool {} resumed successfully, continuing chat_loop",
            tool_name
        );
        Ok(ResumeOutcome::Continue)
    } else {
        plugin_info!(
            "session",
            "[Resume] tool {} resumed with failure, waiting for next resume",
            tool_name
        );
        Ok(ResumeOutcome::Done)
    }
}

/// 中止收口：把父 ToolCall 从 `Streaming` 就地定稿，返回**完整快照**供广播。
///
/// ## 为什么必须做
///
/// 步骤 5 在真正重跑工具**之前**就把父节点广播成 `Streaming`（这是有意的：
/// 否则用户点下「批准执行」后画面毫无变化）。`Streaming` 是瞬态状态——只经
/// 广播通道下发，存储里仍是恢复前的 `Failed` 终态（持久层写入不变量见
/// `chat_session.rs::ensure_durable_states`）。但它同时把「收口」的责任交给了
/// 后续流程——一旦中途返回，**广播层**没有任何机制会再碰它，前端就永久停在
/// "运行中"。中止正是这样一条中途返回的路径
/// （`docs/node-state-streaming.md` §8.11：不得有节点停在 `Streaming`）。
///
/// ## 为什么复用 `apply_not_executed`
///
/// 中止是「没执行完」，不是「执行失败」，因此与未执行工具同一口径：
/// `Completed` + `meta.failure_kind`，**不挂 error**。挂 error 会让前端在父节点上
/// 多渲染一份错误文本，而中止本来就不是错误；用独立写法则会让同一语义长出第二
/// 份定义，日后改一处漏一处。
fn finalize_aborted_parent(
    messages: &mut [ChatMessage],
    tc_idx: usize,
    _tool_call_id: &str,
) -> ChatMessage {
    let tc = messages
        .get_mut(tc_idx)
        .expect("步骤 2 已校验过 tc_idx 有效");
    apply_not_executed(tc, "aborted");
    tc.clone()
}

/// 重新执行工具（approve/retry/supply 共用）。
///
/// 返回 `(result_text, success)`。
///
/// 使用**静默出口**（[`EventSink::silent`]）而非主出口，因为 `execute_tool_async` 会以
/// `result_msg_id` 发送流式更新和完成事件，若走主出口会与 `process_tool_resume_action`
/// 自行构造的 `new_child`（不同 id）产生重复节点。丢弃中间更新，由调用方统一
/// 构造最终结果子节点。
///
/// 收口前这里靠「新建一个临时 `PluginChannel` 并起一个 drain 任务排空对端」实现
/// 静默——那是「出口只能是通道」的产物；现在静默是出口的一种取值，两行代码即达意。
async fn reexecute_tool(
    orchestrator: &ChatOrchestrator,
    ctx: &Arc<dyn InvokeRequest>,
    abort: &AbortSignal,
    tool_name: &str,
    args: Value,
    tool_call_id: &str,
) -> (String, bool) {
    let result_msg_id = short_id();
    let silent = EventSink::silent();

    let (res, success, _captured) = execute_tool_async(
        &orchestrator.parent,
        tool_name,
        args,
        tool_call_id,
        &silent,
        abort,
        result_msg_id,
        ctx.clone(),
    )
    .await;

    (res, success)
}

/// 从 ToolCall 父节点和待恢复子节点中提取工具执行上下文。
///
/// 返回 `(tool_name, base_args, failure_kind)`：
/// - **user_prompt 子节点**（confirm/ask_user 场景）：优先从 `meta.prompt` 提取
///   tool_name 和 args
/// - **Failed Text 子节点**（工具执行失败重试场景）：从 ToolCall 父节点的 `meta`
///   提取 tool_name/args，或从 `content`（args JSON 字符串）解析
fn extract_tool_context(
    messages: &[ChatMessage],
    tc_idx: usize,
    child_idx: usize,
) -> (String, Value, String) {
    let tc = &messages[tc_idx];
    let child = &messages[child_idx];

    // 优先从子节点（user_prompt）的 meta.prompt 提取（confirm/ask_user 场景）
    if child.msg_type == Some(MessageType::UserPrompt) {
        if let Some(prompt) = child.meta.as_ref().and_then(|m| m.get("prompt")) {
            let tool_name = prompt
                .get("tool_name")
                .and_then(|v| v.as_str())
                .unwrap_or("ask_user")
                .to_string();
            let args = prompt.get("args").cloned().unwrap_or(json!({}));
            let failure_kind = child
                .meta
                .as_ref()
                .and_then(|m| m.get("failure_kind"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            return (tool_name, args, failure_kind);
        }
    }

    // 回退：从 ToolCall 父节点的 meta（失败重试场景）提取
    let tool_name = tc.name.clone().unwrap_or_else(|| {
        tc.meta
            .as_ref()
            .and_then(|m| m.get("tool_name"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    });
    let args = tc
        .meta
        .as_ref()
        .and_then(|m| m.get("args"))
        .cloned()
        .unwrap_or_else(|| {
            // 尝试从 ToolCall content（args JSON 字符串）解析
            tc.content
                .as_ref()
                .and_then(|c| c.to_text().parse::<Value>().ok())
                .unwrap_or(json!({}))
        });
    let failure_kind = tc
        .meta
        .as_ref()
        .and_then(|m| m.get("failure_kind"))
        .and_then(|v| v.as_str())
        .unwrap_or("error")
        .to_string();
    (tool_name, args, failure_kind)
}

#[cfg(test)]
#[path = "resume.test.rs"]
mod tests;
