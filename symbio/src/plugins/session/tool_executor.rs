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
use crate::symbio_core::{dir_from_ctx, PLUGIN_SESSION};
use crate::symbio_core::{
    schemas::{
        hook::{HookEvent, HookOutput},
        session::chat_message::{
            ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
        },
        session::session_chat_response,
    },
    InvokeRequestExt,
};
use crate::symbio_core::{
    InvokeRequest, Plugin, PluginChannel, PluginError, PluginFrame, PluginPayload, HOOK_FIRE,
};
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

/// 工具流式接收的空闲超时（秒）：超过该时长未收到任何新帧则判定工具挂死，
/// 返回错误结果而非无限挂起（防卡死兜底之一）。
pub const TOOL_STREAM_IDLE_TIMEOUT_SECS: u64 = 180;

/// 参数摘要：截断到 max_chars，用于日志打印（避免超长参数刷屏）。
///
/// 安全截断：`max_chars` 按**字节**解释但必须落在字符边界上。
/// 历史事故：中文参数（如 `write_file` 的正文）使 `&s[..200]` 落在
/// 多字节字符内部 → `tokio-rt-worker` panic → 整轮 ChatLoop 异常终止。
fn args_summary(args: &Value, max_chars: usize) -> String {
    let s = args.to_string();
    if s.len() <= max_chars {
        s
    } else {
        let end = crate::symbio_core::floor_char_boundary(&s, max_chars);
        format!("{}…(len={})", &s[..end], s.len())
    }
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
    let started_at = std::time::Instant::now();
    plugin_info!(
        "session",
        "[Tool] 请求发起: {} args={}",
        tool_name,
        args_summary(&args, 200)
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
    // 流式工具（如 shell）据此 id 广播增量帧：与 result_msg_id 占位节点同 id，
    // 前端按 role=tool 全量替换合并；最终哨兵帧被捕获为工具结果。
    tool_ctx.set(crate::symbio_core::RESULT_MSG_ID, result_msg_id.clone());

    // 工具执行硬超时：防止某个工具插件内部挂死（如子进程永不退出、管道断裂）
    // 把整个消费循环永久卡住。非流式路径 route() 是单次 await，无任何帧可观测，
    // 一旦挂死既无日志也无退出——必须在此兜底。
    const TOOL_EXEC_HARD_TIMEOUT_SECS: u64 = 600; // 10 分钟
    let route_fut: std::pin::Pin<
        Box<
            dyn std::future::Future<Output = Result<crate::symbio_core::PluginPayload, PluginError>>
                + Send,
        >,
    > = if let Some(tool_visitor) = ctx.get(crate::symbio_core::CAPABILITY_VISITOR) {
        if tool_visitor.has_capability(tool_name).await {
            plugin_info!("session", "[Tool] Using ToolManager for: {}", tool_name);
            let _ = tool_ctx.set_payload(args.clone());
            let visitor = tool_visitor.clone();
            let tool_ctx2 = tool_ctx.clone();
            let tool_name2 = tool_name.to_string();
            Box::pin(async move { visitor.invoke(&tool_name2, tool_ctx2.clone()).await })
        } else {
            plugin_info!(
                "session",
                "[Tool] ToolManager does not have tool: {}, falling back to route",
                tool_name
            );
            tool_ctx.set(crate::symbio_core::PATH, invoke_name.clone());
            let _ = tool_ctx.set_payload(args.clone());
            let p2 = p.clone();
            let tool_ctx2 = tool_ctx.clone();
            Box::pin(async move { p2.route(tool_ctx2).await })
        }
    } else {
        tool_ctx.set(crate::symbio_core::PATH, invoke_name.clone());
        let _ = tool_ctx.set_payload(args.clone());
        let p2 = p.clone();
        let tool_ctx2 = tool_ctx.clone();
        Box::pin(async move { p2.route(tool_ctx2).await })
    };

    let route_result = match tokio::time::timeout(
        std::time::Duration::from_secs(TOOL_EXEC_HARD_TIMEOUT_SECS),
        route_fut,
    )
    .await
    {
        Ok(r) => r,
        Err(_) => {
            plugin_error!(
                "session",
                format!(
                    "[Tool] 执行硬超时 ({}s): {}，已中断。call_id={}",
                    TOOL_EXEC_HARD_TIMEOUT_SECS, tool_name, tool_call_id
                )
            );
            return (
                format!(
                    "Error: 工具 {} 执行超过 {} 秒未返回，已强制中断（疑似挂死）",
                    tool_name, TOOL_EXEC_HARD_TIMEOUT_SECS
                ),
                false,
                None,
            );
        }
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
                    "[Tool] 正常结束: {} (耗时 {}ms, 结果长度 {})",
                    tool_name,
                    started_at.elapsed().as_millis(),
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
                let mut last_frame_at = std::time::Instant::now();
                let mut frame_count: usize = 0;

                loop {
                    // 空闲超时：工具流超过 TOOL_STREAM_IDLE_TIMEOUT_SECS 无任何新帧
                    // 判定为挂死（如子进程 hang、管道断裂），显式报错退出而非无限等待。
                    let frame = match tokio::time::timeout(
                        std::time::Duration::from_secs(TOOL_STREAM_IDLE_TIMEOUT_SECS),
                        tool_chan.rx.recv(),
                    )
                    .await
                    {
                        Ok(f) => f,
                        Err(_) => {
                            plugin_error!(
                                "session",
                                format!(
                                    "[Tool] 流式执行空闲超时 ({}s 无新帧): {}，已中断。call_id={}",
                                    TOOL_STREAM_IDLE_TIMEOUT_SECS, tool_name, tool_call_id
                                )
                            );
                            return (
                                format!(
                                    "Error: 工具流式执行超过 {} 秒无输出，已强制中断（疑似挂死）",
                                    TOOL_STREAM_IDLE_TIMEOUT_SECS
                                ),
                                false,
                                None,
                            );
                        }
                    };
                    let Some(frame) = frame else {
                        // 发送端全部关闭：工具正常结束（run.rs drop 了 tx）
                        break;
                    };
                    frame_count += 1;
                    if last_frame_at.elapsed().as_secs() >= 30 {
                        plugin_warn!(
                            "session",
                            "[Tool] 流式帧间隔过长: {} 距上一帧 {}s（帧 #{}, call_id={}）",
                            tool_name,
                            last_frame_at.elapsed().as_secs(),
                            frame_count,
                            tool_call_id
                        );
                    }
                    last_frame_at = std::time::Instant::now();
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
                                            format!(
                                                "[Tool] NESTED Error: {} (耗时 {}ms)",
                                                error,
                                                started_at.elapsed().as_millis()
                                            )
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
                            plugin_error!(
                                "session",
                                format!(
                                    "[Tool] STREAM Error: {} (耗时 {}ms)",
                                    e,
                                    started_at.elapsed().as_millis()
                                )
                            );
                            return (format!("Error: {e}"), false, None);
                        }
                    }
                }
                if is_aborted.load(Ordering::Relaxed) {
                    plugin_warn!(
                        "session",
                        "[Tool] 流式执行被中止（abort）: {} (耗时 {}ms, 帧数 {}, 已累积长度 {})",
                        tool_name,
                        started_at.elapsed().as_millis(),
                        frame_count,
                        full.len()
                    );
                } else {
                    plugin_info!(
                        "session",
                        "[Tool] 流式正常结束: {} (耗时 {}ms, 帧数 {}, 结果长度 {})",
                        tool_name,
                        started_at.elapsed().as_millis(),
                        frame_count,
                        full.len()
                    );
                }
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
            plugin_error!(
                "session",
                format!(
                    "[Tool] ROUTE Error: {} (耗时 {}ms)",
                    e,
                    started_at.elapsed().as_millis()
                )
            );
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

        // 参数 JSON 非空却解析失败（典型：被 max_tokens 截断）→ **拒绝执行**。
        // 旧行为是带着占位 `{}` 继续执行，工具必然报「缺少必填参数」，而这条错误
        // 对模型毫无信息量（它认为自己发了完整参数），于是原样重试 → 卡思考死循环。
        // 这里以协议错误形态回报，明确告诉模型「参数残破，需重发完整调用」。
        if let Some(raw) = tc.parse_error.as_ref() {
            let preview: String = raw.chars().take(400).collect();
            let truncated = raw.chars().count() > 400;
            plugin_error!(
                "session",
                "Protocol Error: Tool call arguments JSON parse failed, refusing to execute. ID: {id}, name: {name}"
            );
            record_protocol_failure(
                channel,
                &id,
                &format!(
                    "工具参数 JSON 解析失败（可能被长度上限截断），本次调用未执行。\
                     只有完整且可解析的 JSON 参数才会被执行，请勿以相同内容重试。\n\
                     参数原文（{} 字符{}）：{preview}",
                    raw.chars().count(),
                    if truncated { "，已截断展示" } else { "" }
                ),
                &mut tool_messages,
                &mut parent_updates,
            )
            .await;
            continue;
        }

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
            // 会话存储根 = 本插件自己的目录（装配态由父插件经 `PLUGIN_DIR` 告知）
            let storage_root = dir_from_ctx(&*ctx, PLUGIN_SESSION);
            let guarded = guard_tool_result(
                &final_res,
                DEFAULT_TOOL_RESULT_TOKEN_CAP,
                Some(&guard_session),
                storage_root.dir(),
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
#[path = "tool_executor.test.rs"]
mod tests;
