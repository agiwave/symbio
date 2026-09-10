//! 工具执行层
//!
//! 负责：
//! - 单个工具调用的路由与执行（`execute_tool_async`）
//! - 批量工具调用处理与结果广播（`process_tool_calls_async`）
//!
//! 设计说明（会话激活/恢复状态机）：
//! - 本层**不阻塞**等待任何用户输入。需要用户确认（confirm）或主动询问（ask_user）
//!   的工具会自行产出 `user_prompt` 消息节点并标记 `WaitingUserAction`，由编排层
//!   （chat_loop）在本轮结束时将会话置于 `AwaitingInput(user)`；用户答案以一条普通
//!   `user` 消息回填后，新一轮会重跑该工具。详见 USER_INPUT_MECHANISM 设计文档。

use crate::symbio_core::turn::{build_tool_message, short_id, ToolCallInfo};
use crate::symbio_core::{
    schemas::{
        session::chat_message::{
            ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
        },
        session::session_chat_response,
        system::hook::{HookEvent, HookOutput},
    },
    InvokeRequestExt,
};
use crate::symbio_core::{InvokeRequest, Plugin, PluginChannel, PluginFrame, PluginPayload, HOOK_FIRE};
use crate::{plugin_error, plugin_info, plugin_warn};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::tool_result_guard::{guard_tool_result, DEFAULT_TOOL_RESULT_TOKEN_CAP};

// 工具结果提取

/// 辅助：发送工具执行过程中的增量更新
async fn emit_tool_update(
    channel: &PluginChannel,
    msg_id: &str,
    tool_call_id: &str,
    delta: String,
    status: MessageStatus,
) {
    let _ = channel
        .tx
        .send(PluginFrame::Data(
            serde_json::to_value(session_chat_response::StreamEvent::Update {
                message: ChatMessage {
                    id: msg_id.to_string(),
                    parent_id: Some(tool_call_id.to_string()),
                    role: Some(MessageRole::Tool),
                    msg_type: Some(MessageType::Text),
                    content: Some(MessageContent::Text(delta)),
                    status: Some(status),
                    ..Default::default()
                },
            })
            .unwrap_or_default(),
        ))
        .await;
}

/// 从工具返回的 JSON 数据中提取可读的文本结果。
pub fn extract_result(data: &Value) -> String {
    if let Some(content) = data.get("content").and_then(|v| v.as_str()) {
        return content.to_string();
    }
    if let Some(output) = data.get("output").and_then(|v| v.as_str()) {
        return output.to_string();
    }
    if let Some(success) = data.get("success").and_then(|s| s.as_bool()) {
        if success {
            data.to_string()
        } else {
            format!(
                "Error: {}",
                data.get("error")
                    .and_then(|e| e.as_str())
                    .unwrap_or("unknown error")
            )
        }
    } else {
        data.as_str()
            .map(|s| s.to_string())
            .unwrap_or_else(|| data.to_string())
    }
}

/// Hook 事件触发工具函数
pub async fn fire_hook(
    parent: &Option<Arc<dyn Plugin>>,
    event: HookEvent,
    ctx: Arc<dyn InvokeRequest>,
) -> HookOutput {
    let p = match parent {
        Some(p) => p,
        None => return HookOutput::default(),
    };

    let session_id = ctx.get(crate::symbio_core::SESSION_ID).unwrap_or_default();

    let hook_ctx = ctx.fork();
    hook_ctx.set(crate::symbio_core::PATH, HOOK_FIRE.to_string());
    let _ = hook_ctx.set_payload(json!({
        "session_id": session_id,
        "event": serde_json::to_value(&event).unwrap_or(json!({})),
    }));

    match p.clone().route(hook_ctx).await {
        Ok(resp) => resp
            .get::<HookOutput>()
            .unwrap_or_else(|_| HookOutput::default()),
        Err(_) => HookOutput::default(),
    }
}

// 单工具执行（无阻塞）

/// 执行单个工具调用（不阻塞等待用户）。
///
/// 返回 `(result_text, success)`。
///
/// 若工具因需要用户确认而产出 `user_prompt`（WaitingUserAction）节点，
/// 该节点已作为普通工具结果通过 `channel` 广播，本函数返回其提示文本，
/// `success=true`（让编排层知道本轮已正常结束于 pending，而非失败）。
#[allow(clippy::too_many_arguments)]
pub async fn execute_tool_async(
    parent: &Option<Arc<dyn Plugin>>,
    tool_name: &str,
    args: Value,
    tool_call_id: &str,
    channel: &mut PluginChannel,
    is_aborted: &AtomicBool,
    result_msg_id: String,
    ctx: Arc<dyn InvokeRequest>,
) -> (String, bool, Option<ChatMessage>) {
    plugin_info!(
        "session",
        "[Tool] Execution started: {} ({})",
        tool_name,
        tool_call_id
    );

    let invoke_name = tool_name.replace("__", "/");
    let p = match parent {
        Some(p) => p,
        None => return ("Error: No parent plugin".into(), false, None),
    };

    let session_id = ctx.get(crate::symbio_core::SESSION_ID).unwrap_or_default();
    let agent_id = ctx.get(crate::symbio_core::AGENT_ID).unwrap_or_default();
    let workdir = ctx.get(crate::symbio_core::WORKDIR).unwrap_or_default();

    let tool_ctx = ctx.fork();
    tool_ctx.set(crate::symbio_core::WORKDIR, workdir);
    tool_ctx.set(crate::symbio_core::AGENT_ID, agent_id);
    tool_ctx.set(crate::symbio_core::SESSION_ID, session_id);
    tool_ctx.set(crate::symbio_core::TOOL_CALL_ID, tool_call_id.to_string());

    let route_result = if let Some(tool_manager) = ctx.get(crate::symbio_core::CAPABILITY_MANAGER) {
        if tool_manager.has_capability(tool_name).await {
            plugin_info!("session", "[Tool] Using ToolManager for: {}", tool_name);
            let _ = tool_ctx.set_payload(args.clone());
            tool_manager.invoke(tool_name, tool_ctx.clone()).await
        } else {
            plugin_info!(
                "session",
                "[Tool] ToolManager does not have tool: {}, falling back to route",
                tool_name
            );
            tool_ctx.set(crate::symbio_core::PATH, invoke_name.clone());
            let _ = tool_ctx.set_payload(args.clone());
            p.clone().route(tool_ctx).await
        }
    } else {
        tool_ctx.set(crate::symbio_core::PATH, invoke_name.clone());
        let _ = tool_ctx.set_payload(args.clone());
        p.clone().route(tool_ctx).await
    };

    match route_result {
        Ok(resp) => match resp {
            // ── 即时响应 ──────────────────────────────────────────────────────
            PluginPayload::Data(_) => {
                let data = match resp.get::<serde_json::Value>() {
                    Ok(d) => d,
                    Err(_) => {
                        return ("Error: Failed to deserialize payload".into(), false, None);
                    }
                };
                // plugin_debug!(
                //     "session",
                //     "Tool immediate response for {}: {}",
                //     tool_name,
                //     data
                // );

                // 直接返回结果（需要确认/询问的工具已自行产出 user_prompt 节点）
                let res = extract_result(&data);
                plugin_info!(
                    "session",
                    "[Tool] FINISHED: {} (Len: {})",
                    tool_name,
                    res.len()
                );
                (res, true, None)
            }

            // ── 流式响应 ──────────────────────────────────────────────────────
            PluginPayload::Session(mut tool_chan) => {
                plugin_info!(
                    "session",
                    "[Tool] STREAMING execution started: {}",
                    tool_name
                );
                let mut full = String::new();
                // 捕获工具广播的 user_prompt(WaitingUserAction) 节点，作为本轮"待用户响应"结果返回
                let mut captured_prompt: Option<ChatMessage> = None;

                while let Some(frame) = tool_chan.rx.recv().await {
                    if is_aborted.load(Ordering::Relaxed) {
                        break;
                    }
                    if channel.cancel_token.is_cancelled() {
                        is_aborted.store(true, Ordering::Relaxed)
                    }
                    while let Ok(f) = channel.rx.try_recv() {
                        match f {
                            PluginFrame::Data(m)
                                if m.get("type").and_then(|v| v.as_str()) == Some("abort") =>
                            {
                                is_aborted.store(true, Ordering::Relaxed)
                            }
                            _ => {}
                        }
                    }
                    if is_aborted.load(Ordering::Relaxed) {
                        break;
                    }

                    match frame {
                        PluginFrame::Data(d) => {
                            // Check if this is a StreamEvent::Update (nested events from any tool execution)
                            if let Ok(event) = serde_json::from_value::<
                                session_chat_response::StreamEvent,
                            >(d.clone())
                            {
                                match event {
                                    session_chat_response::StreamEvent::Update { mut message } => {
                                        // 子会话的委托 prompt（user 消息）不透传：
                                        // 其内容已可见于 ToolCall 的请求参数（args.prompt），
                                        // 且 role=user 的临时节点会在前端获得"编辑"入口
                                        //（该 id 不在父会话存储中，操作必然失败）。
                                        if message.role == Some(MessageRole::User) {
                                            continue;
                                        }
                                        // Tool execution message handling:
                                        // - Root messages (no parent_id) get their parent_id set to tool_call_id
                                        // - All other messages are passed through as-is
                                        // - This maintains the internal hierarchy while anchoring to the tool call
                                        if message.parent_id.is_none() {
                                            // 子 agent 会话的顶层响应节点（原 parent_id=None）锚定到
                                            // ToolCall 之下；其角色应为 Tool（工具响应）而非 Assistant，
                                            // 以符合分型结构 ToolCall(Assistant) → Turn(Tool)，并能被
                                            // flatten 的 find_tool_result 正确识别。
                                            message.parent_id = Some(tool_call_id.to_string());
                                            if message.role == Some(MessageRole::Assistant) {
                                                message.role = Some(MessageRole::Tool);
                                            }
                                        }
                                        // 捕获工具广播的 user_prompt(WaitingUserAction) 节点，
                                        // 作为本轮"待用户响应"结果落库。
                                        if message.msg_type == Some(MessageType::UserPrompt)
                                            && message.status
                                                == Some(MessageStatus::WaitingUserAction)
                                        {
                                            let mut node = message.clone();
                                            node.parent_id = Some(tool_call_id.to_string());
                                            node.role = Some(MessageRole::Tool);
                                            captured_prompt = Some(node);
                                            // 不转发到 channel —— process_tool_calls_async 会以统一 id
                                            //（result_msg_id）重新广播该节点。若此处也转发，前端会收到
                                            // 两个不同 id 但同 parent_id 的 user_prompt 节点，导致：
                                            //  1) 重复的审批 UI；
                                            //  2) resume 时后端只删一个（后端 messages 仅一份），
                                            //     另一个残留在前端 store，审批 UI 永不消失。
                                            continue;
                                        }
                                        // 其余消息：parent_id 已指向正确父节点，透传
                                        let _ = channel
                                            .tx
                                            .send(PluginFrame::Data(
                                                serde_json::to_value(
                                                    session_chat_response::StreamEvent::Update {
                                                        message,
                                                    },
                                                )
                                                .unwrap_or_default(),
                                            ))
                                            .await;
                                    }
                                    session_chat_response::StreamEvent::Error { error } => {
                                        plugin_error!(
                                            "session",
                                            format!("[Tool] NESTED Error: {}", error)
                                        );
                                        return (format!("Error: {error}"), false, None);
                                    }
                                    _ => {}
                                }
                            } else if let Some(text) = d.get("content").and_then(|v| v.as_str()) {
                                // Plain content frame (final result sentinel from run.rs)
                                full = text.to_string();
                            }
                        }
                        PluginFrame::Error(e, _) => {
                            plugin_error!("session", format!("[Tool] STREAM Error: {}", e));
                            return (format!("Error: {e}"), false, None);
                        }
                    }
                }
                plugin_info!(
                    "session",
                    "[Tool] STREAMING finished: {} (Total Len: {})",
                    tool_name,
                    full.len()
                );
                // Mark the result message as completed (携带实际累积结果 full)
                emit_tool_update(
                    channel,
                    &result_msg_id,
                    tool_call_id,
                    full.clone(),
                    MessageStatus::Completed,
                )
                .await;
                (full, true, captured_prompt)
            }
            _ => ("Error: Unexpected payload type".into(), false, None),
        },
        Err(e) => {
            plugin_error!("session", format!("[Tool] ROUTE Error: {}", e));
            (format!("Error: {e}"), false, None)
        }
    }
}

// 批量工具调用处理

/// 记录协议级工具调用失败（工具调用 id/name 缺失或非法）。
///
/// 不再简单跳过：跳过会让已落库的 ToolCall 节点没有结果子节点，
/// 下一轮请求携带"无结果的 tool_call"触发 provider 400（Bug 2 同类问题）。
/// 处理口径与普通工具执行失败一致（失败属信息性）：
/// - 生成一条 `role=Tool` 的错误结果子节点（Completed + success=false），错误内容喂回 LLM；
/// - 生成父 ToolCall 节点的失败补丁（failure_kind=error）并广播。
///
/// 两条消息分别加入 tool_messages / parent_updates，由调用方统一持久化。
async fn record_protocol_failure(
    channel: &mut PluginChannel,
    tool_call_id: &str,
    error_text: &str,
    tool_messages: &mut Vec<ChatMessage>,
    parent_updates: &mut Vec<ChatMessage>,
) {
    let result_msg_id = uuid::Uuid::new_v4().to_string();
    let mut tool_msg = build_tool_message(
        tool_call_id,
        &format!("Error: {error_text}"),
        Some(false),
        Some(result_msg_id),
    );
    // 失败属信息性：结果以 Completed 留在上下文（Failed 会被 get_context_messages
    // 过滤，导致"孤儿 tool 结果"使下一轮 LLM 请求非法）。
    tool_msg.status = Some(MessageStatus::Completed);

    let _ = channel
        .tx
        .send(PluginFrame::Data(
            serde_json::to_value(session_chat_response::StreamEvent::Update {
                message: tool_msg.clone(),
            })
            .unwrap_or_default(),
        ))
        .await;

    let parent_update = ChatMessage {
        id: tool_call_id.to_string(),
        status: Some(MessageStatus::Completed),
        error: Some(error_text.to_string()),
        meta: Some(json!({
            "success": false,
            "failure_kind": "error",
        })),
        ..Default::default()
    };
    let _ = channel
        .tx
        .send(PluginFrame::Data(
            serde_json::to_value(session_chat_response::StreamEvent::Update {
                message: parent_update.clone(),
            })
            .unwrap_or_default(),
        ))
        .await;

    plugin_info!(
        "session",
        "[Tool] Protocol failure recorded as failed tool call: {}",
        error_text
    );

    tool_messages.push(tool_msg);
    parent_updates.push(parent_update);
}

/// 顺序处理一批工具调用，向 channel 广播每个工具的结果，
/// 并返回 `(tool_messages, parent_updates)`：
/// - `tool_messages`：工具结果子节点（用于追加到对话历史）
/// - `parent_updates`：ToolCall 父节点的状态补丁（id + status + meta + error），
///   供 chat_loop 调用 `update_messages` 持久化（解决父节点状态不持久化问题）。
///
/// 交互模式（interactive）下，若前一个工具产出 user_prompt（待审批/询问）或失败，
/// 则中止本批剩余工具（用户需逐个处理）；auto 模式不中止，失败结果传 LLM 继续。
#[allow(clippy::too_many_arguments)]
pub async fn process_tool_calls_async(
    tool_calls: Vec<ToolCallInfo>,
    parent: &Option<Arc<dyn Plugin>>,
    channel: &mut PluginChannel,
    is_aborted: &Arc<AtomicBool>,
    ctx: Arc<dyn InvokeRequest>,
) -> (Vec<ChatMessage>, Vec<ChatMessage>) {
    let mut tool_messages = Vec::new();
    let mut parent_updates: Vec<ChatMessage> = Vec::new();
    if tool_calls.is_empty() {
        return (tool_messages, parent_updates);
    }

    let mode = ctx.get(crate::symbio_core::MODE).unwrap_or_default();

    plugin_info!(
        "session",
        "Processing batch of {} tool calls (mode={})...",
        tool_calls.len(),
        mode
    );

    for tc in tool_calls {
        if is_aborted.load(Ordering::Relaxed) {
            break;
        }
        // 交互模式下，前一个工具待审批/失败 → 中止本批剩余（用户需逐个处理）
        if mode == "interactive" && !parent_updates.is_empty() {
            let last_blocked = parent_updates
                .last()
                .map(|p| {
                    p.status == Some(MessageStatus::WaitingUserAction)
                        || p.meta
                            .as_ref()
                            .and_then(|m| m.get("failure_kind"))
                            .and_then(|v| v.as_str())
                            .map(|k| {
                                k == "error" || k == "needs_approval" || k == "needs_interaction"
                            })
                            .unwrap_or(false)
                })
                .unwrap_or(false);
            if last_blocked {
                plugin_info!(
                    "session",
                    "[Tool] 交互模式下前一个工具待用户恢复，中止本批剩余工具"
                );
                break;
            }
        }

        // 工具调用 id 缺失/非法 → 作为工具调用失败处理（不跳过）。
        // 正常情况下 ToolCallAccumulator 已保证 id 非空；此分支为兜底防御。
        // 注意：兜底 id 仅用于挂载失败结果与父节点补丁（保持结构完整）。
        let id = match tc.id.as_ref() {
            Some(id) if !id.trim().is_empty() => id.clone(),
            _ => {
                plugin_error!(
                    "session",
                    "Protocol Error: Tool call ID missing/invalid, recording as failed tool call"
                );
                record_protocol_failure(
                    channel,
                    &short_id(),
                    "模型未返回有效的工具调用 ID（协议错误）",
                    &mut tool_messages,
                    &mut parent_updates,
                )
                .await;
                continue;
            }
        };
        // 工具名缺失/非法 → 同样作为工具调用失败处理（不跳过），
        // 避免已落库的 ToolCall 节点没有结果子节点。
        let name = match tc.name.as_ref() {
            Some(name) if !name.trim().is_empty() => name.clone(),
            _ => {
                plugin_error!("session",
                    format!(
                        "Protocol Error: Tool call name missing/invalid, recording as failed tool call. ID: {id}"
                    )
                );
                record_protocol_failure(
                    channel,
                    &id,
                    "模型未返回有效的工具名称（协议错误）",
                    &mut tool_messages,
                    &mut parent_updates,
                )
                .await;
                continue;
            }
        };

        let result_msg_id = uuid::Uuid::new_v4().to_string();

        let pre_output = fire_hook(
            parent,
            HookEvent::PreToolUse {
                tool_name: name.clone(),
                tool_input: tc.arguments.clone(),
            },
            ctx.clone(),
        )
        .await;
        if !pre_output.should_proceed {
            let block_msg = pre_output
                .block_reason
                .unwrap_or_else(|| "Blocked by pre hook".to_string());
            plugin_warn!(
                "session",
                "[Tool] BLOCKED by PreToolUse hook: {}",
                block_msg
            );
            let tool_msg = build_tool_message(
                &id,
                &format!("Blocked: {block_msg}"),
                Some(false),
                Some(result_msg_id.clone()),
            );
            tool_messages.push(tool_msg);
            continue;
        }

        let (res, success, mut pending_user_prompt) = execute_tool_async(
            parent,
            &name,
            tc.arguments.clone(),
            &id,
            channel,
            is_aborted,
            result_msg_id.clone(),
            ctx.clone(),
        )
        .await;

        let final_res = res;

        let tool_output = if success {
            serde_json::json!({ "content": final_res.clone() })
        } else {
            serde_json::json!({ "error": final_res.clone() })
        };
        let _post_output = fire_hook(
            parent,
            HookEvent::PostToolUse {
                tool_name: name.clone(),
                tool_input: tc.arguments.clone(),
                tool_output,
            },
            ctx.clone(),
        )
        .await;

        // 若本轮工具产出了 user_prompt(WaitingUserAction) 节点，则用它作为 tool 结果
        // （携带 meta.prompt 与 WaitingUserAction 状态，供编排层结束本轮并等待用户输入）。
        let mut tool_msg = if let Some(mut prompt) = pending_user_prompt.take() {
            prompt.id = result_msg_id.clone();
            prompt
        } else {
            // L0 守卫：超长工具结果存档 + head/tail 摘要，避免单条撑爆上下文窗口
            // （对应"单次工具调用内容太长"的压缩诉求；物理字节上限不再是唯一防线）。
            // 传入 session_id：存档跟随会话目录（tool_archives/），历史可取回不被 OS 清理。
            let guard_session = ctx.get(crate::symbio_core::SESSION_ID).unwrap_or_default();
            let guarded = guard_tool_result(
                &final_res,
                DEFAULT_TOOL_RESULT_TOKEN_CAP,
                Some(&guard_session),
            );
            let mut tool_msg =
                build_tool_message(&id, &guarded.text, Some(success), Some(result_msg_id));
            if guarded.truncated {
                let mut meta = tool_msg.meta.clone().unwrap_or_else(|| json!({}));
                meta["tool_result_truncated"] = json!(true);
                if let Some(p) = guarded.archive_path {
                    meta["archive_path"] = json!(p);
                }
                meta["origin_tokens"] = json!(guarded.original_tokens);
                tool_msg.meta = Some(meta);
            }
            tool_msg
        };
        // 任何模式：工具失败属"信息性"，结果仍以合法 tool 结果（Completed）留在上下文，
        // 让 LLM 看到错误并继续；其父节点在下方也标 Completed（不暂停会话）。
        // 若此处仍标 Failed，则会被 get_context_messages 过滤，导致"孤儿 tool 结果"
        // （父 tool_call 被过滤、结果残留）使下一轮 LLM 请求非法（Bug 2 同类问题）。
        // 仅对普通工具结果生效（user_prompt 走 WaitingUserAction 分支，不受此覆盖）。
        if !success && tool_msg.msg_type != Some(MessageType::UserPrompt) {
            tool_msg.status = Some(MessageStatus::Completed);
        }

        if tool_msg.msg_type == Some(MessageType::UserPrompt) {
            // user_prompt 节点本身即是工具"结果"（待用户审批/回答）：
            // 广播该节点（WaitingUserAction），并把父节点 ToolCall 标为
            // WaitingUserAction（持久化 failure_kind 供 resume 提取）。
            let failure_kind = tool_msg
                .meta
                .as_ref()
                .and_then(|m| m.get("failure_kind"))
                .and_then(|v| v.as_str())
                .unwrap_or("needs_approval")
                .to_string();

            let _ = channel
                .tx
                .send(PluginFrame::Data(
                    serde_json::to_value(session_chat_response::StreamEvent::Update {
                        message: tool_msg.clone(),
                    })
                    .unwrap_or_default(),
                ))
                .await;

            let parent_update = ChatMessage {
                id: id.clone(),
                status: Some(MessageStatus::WaitingUserAction),
                meta: Some(json!({
                    "success": false,
                    "failure_kind": failure_kind,
                })),
                ..Default::default()
            };
            let _ = channel
                .tx
                .send(PluginFrame::Data(
                    serde_json::to_value(session_chat_response::StreamEvent::Update {
                        message: parent_update.clone(),
                    })
                    .unwrap_or_default(),
                ))
                .await;
            parent_updates.push(parent_update);
        } else {
            // 广播 action 结果（最终定格）。
            // 结果状态直接使用 tool_msg.status：已在上方正确设置
            // （成功或普通失败 => Completed；user_prompt 不进入本分支）。
            let _ = channel
                .tx
                .send(PluginFrame::Data(
                    serde_json::to_value(session_chat_response::StreamEvent::Update {
                        message: ChatMessage {
                            id: tool_msg.id.clone(),
                            parent_id: Some(id.clone()),
                            role: Some(MessageRole::Tool),
                            msg_type: Some(MessageType::Text),
                            content: tool_msg.content.clone(),
                            status: tool_msg.status.clone(),
                            meta: Some(json!({ "success": success })),
                            ..Default::default()
                        },
                    })
                    .unwrap_or_default(),
                ))
                .await;

            // 标记父节点最终状态：
            // - 成功 => Completed
            // - 普通工具失败 => Completed（失败属信息性，错误结果作为合法 tool 结果留在
            //   上下文，父节点不再标 Failed，不暂停会话、不触发重试/补参渲染）。
            //   仅真正需要用户输入的 UserPrompt 场景在上方以 WaitingUserAction 处理。
            let parent_update = if success {
                ChatMessage {
                    id: id.clone(),
                    status: Some(MessageStatus::Completed),
                    meta: Some(json!({ "success": true })),
                    ..Default::default()
                }
            } else {
                ChatMessage {
                    id: id.clone(),
                    status: Some(MessageStatus::Completed),
                    error: Some(final_res.clone()),
                    meta: Some(json!({
                        "success": false,
                        "failure_kind": "error",
                        "tool_name": name,
                        "args": tc.arguments,
                    })),
                    ..Default::default()
                }
            };
            let _ = channel
                .tx
                .send(PluginFrame::Data(
                    serde_json::to_value(session_chat_response::StreamEvent::Update {
                        message: parent_update.clone(),
                    })
                    .unwrap_or_default(),
                ))
                .await;
            parent_updates.push(parent_update);
        }

        tool_messages.push(tool_msg);
    }

    (tool_messages, parent_updates)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::SimpleRequest;
    use std::sync::atomic::AtomicBool;

    fn test_ctx() -> Arc<dyn InvokeRequest> {
        Arc::new(SimpleRequest::new(None, None))
    }

    /// 需求 2 回归：工具调用 id 缺失/非法时不得跳过，必须作为工具调用失败处理——
    /// 生成错误结果子节点（喂回 LLM）+ 父节点失败补丁，保证已落库的 ToolCall
    /// 节点不会成为"无结果 tool_call"（下轮请求 400）。
    #[tokio::test]
    async fn missing_tool_call_id_is_recorded_as_failure() {
        let (_host, mut plugin_chan) = PluginChannel::pair(64);
        let abort = Arc::new(AtomicBool::new(false));
        let tcs = vec![ToolCallInfo {
            id: None,
            name: Some("dir_list".into()),
            arguments: json!({ "path": "." }),
        }];

        let (msgs, updates) =
            process_tool_calls_async(tcs, &None, &mut plugin_chan, &abort, test_ctx()).await;

        assert_eq!(msgs.len(), 1, "必须生成失败结果子节点（而非跳过）");
        assert_eq!(msgs[0].role, Some(MessageRole::Tool));
        assert_eq!(msgs[0].status, Some(MessageStatus::Completed));
        assert!(
            msgs[0]
                .content
                .as_ref()
                .map(|c| c.to_text().contains("Error:"))
                .unwrap_or(false),
            "结果内容应携带错误信息"
        );
        assert!(!msgs[0].parent_id.as_deref().unwrap_or("").is_empty());

        assert_eq!(updates.len(), 1, "必须生成父节点失败补丁");
        assert_eq!(updates[0].status, Some(MessageStatus::Completed));
        assert_eq!(
            updates[0]
                .meta
                .as_ref()
                .and_then(|m| m.get("failure_kind"))
                .and_then(|v| v.as_str()),
            Some("error")
        );
    }

    /// 空串 id 与缺失同等对待（"不合法"）。
    #[tokio::test]
    async fn empty_tool_call_id_is_recorded_as_failure() {
        let (_host, mut plugin_chan) = PluginChannel::pair(64);
        let abort = Arc::new(AtomicBool::new(false));
        let tcs = vec![ToolCallInfo {
            id: Some(String::new()),
            name: Some("dir_list".into()),
            arguments: json!({}),
        }];

        let (msgs, updates) =
            process_tool_calls_async(tcs, &None, &mut plugin_chan, &abort, test_ctx()).await;

        assert_eq!(msgs.len(), 1);
        assert_eq!(updates.len(), 1);
    }

    /// 工具名缺失/非法 → 同样作为失败处理（结果挂到已知 id 上，结构完整）。
    #[tokio::test]
    async fn missing_tool_name_is_recorded_as_failure() {
        let (_host, mut plugin_chan) = PluginChannel::pair(64);
        let abort = Arc::new(AtomicBool::new(false));
        let tcs = vec![ToolCallInfo {
            id: Some("tc-known".into()),
            name: Some(String::new()),
            arguments: json!({}),
        }];

        let (msgs, updates) =
            process_tool_calls_async(tcs, &None, &mut plugin_chan, &abort, test_ctx()).await;

        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].parent_id.as_deref(), Some("tc-known"));
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].id, "tc-known");
    }
}
