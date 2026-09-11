//! 单轮 LLM 执行机器 —— E-① 自 `plugins/model/{context,tool_call,message_builder}.rs` 迁入
//!
//! 职责（单轮网关基建，协议无关、插件无关，供 `ModelProvider::execute_turn`
//! 统一实现与 model/session 双侧共同使用，详见
//! docs/archive/implementation-logs/model-session-refactor.md §8）：
//! - HTTP 客户端单例 + 支持中止的 POST 重试机器（`execute_post_with_abort` → 五态 `PostResult`）
//! - SSE 流解析与协议事件累积（`parse_sse_stream` → `TurnOutput`），流式子节点经
//!   `session_chat_response::StreamEvent::Update` 帧实时下发
//! - 工具调用增量累积（`ToolCallAccumulator`）
//! - 消息构造家族（`short_id`/`StreamChildIds`/`build_assistant_messages`/`build_tool_message`）：
//!   因 `TurnOutput::into_messages` 与 `ToolCallAccumulator` 直接依赖而随依赖闭包迁入
//!   （孤儿规则要求定义与使用同处 core）
//!
//! 事实来源说明：model 侧原文件保留 re-export shim 维持既有符号路径
//! （`plugins::model::{protocol, tool_call, message_builder, context}`），
//! 本模块是唯一权威实现。

use crate::plugin_warn;
use crate::symbio_core::model_provider::{FinishReason, ProtocolEvent, Usage};
use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
};
use crate::symbio_core::schemas::session::session_chat_response;
use crate::symbio_core::{PluginChannel, PluginFrame};
use futures::StreamExt;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;
use tracing::warn;

// HTTP 客户端（单例）

pub fn get_http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .pool_max_idle_per_host(10)
            .tcp_keepalive(Some(std::time::Duration::from_secs(60)))
            .connect_timeout(std::time::Duration::from_secs(10))
            .timeout(std::time::Duration::from_secs(1800)) // 整体流式请求超时
            .build()
            .expect("Failed to build shared reqwest Client")
    })
}

// 通道辅助

/// 统一发送消息更新事件到前端。
pub async fn emit_update(channel: &PluginChannel, msg: ChatMessage) {
    let _ = channel
        .tx
        .send(PluginFrame::Data(
            serde_json::to_value(session_chat_response::StreamEvent::Update { message: msg })
                .unwrap_or_default(),
        ))
        .await;
}

/// 发送简化的状态更新（仅包含 ID 和 Status）。
pub async fn emit_status(channel: &PluginChannel, id: String, status: MessageStatus) {
    emit_update(
        channel,
        ChatMessage {
            id,
            status: Some(status),
            ..Default::default()
        },
    )
    .await;
}

/// 发送中止帧（`PostResult::RetryWithoutContextId` 路径使用：通知前端停止流式渲染）。
pub async fn emit_abort(channel: &PluginChannel) {
    let _ = channel
        .tx
        .send(PluginFrame::Data(
            serde_json::to_value(session_chat_response::StreamEvent::Abort {}).unwrap_or_default(),
        ))
        .await;
}

// 控制信号处理

/// 处理单个控制帧。返回 `true` 表示收到中断信号。
fn handle_signal_frame(frame: PluginFrame, abort_flag: &AtomicBool) -> bool {
    match frame {
        PluginFrame::Data(ref m) if m.get("type").and_then(|v| v.as_str()) == Some("abort") => {
            abort_flag.store(true, Ordering::SeqCst);
            true
        }
        _ => false,
    }
}

/// 排空当前挂起的控制帧（非阻塞）。
fn drain_pending_signals(channel: &mut PluginChannel, abort_flag: &AtomicBool) {
    while let Ok(frame) = channel.rx.try_recv() {
        if handle_signal_frame(frame, abort_flag) {
            break;
        }
    }
}

/// 持续轮询直至中断（阻塞，用于 select!）。
async fn wait_for_abort_signal(channel: &mut PluginChannel, abort_flag: &AtomicBool) {
    // 检查标志位
    if abort_flag.load(Ordering::SeqCst) {
        return;
    }

    // 中止感知有两条独立通道，任一触发即返回：
    // 1. rx 收到显式 Abort 帧（消费循环转发的 transport 信号）；
    // 2. abort_flag 被外部置位——**这是内部请求（如上下文压缩）唯一的中止感知路径**：
    //    压缩请求挂在静默哑通道上（rx 永无帧），用户停止时 Abort 帧堆在主通道、
    //    消费循环正 await 在压缩请求上无暇收取，只有共享的 abort_flag 会被置位。
    //    因此这里必须轮询标志位，否则压缩请求在用户中止后仍会跑完整整轮 LLM 流。
    let flag = abort_flag;
    loop {
        tokio::select! {
            _ = async {
                while !flag.load(Ordering::SeqCst) {
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            } => {
                return;
            }
            frame = channel.rx.recv() => match frame {
                Some(frame) => {
                    if handle_signal_frame(frame, abort_flag) {
                        return;
                    }
                    if abort_flag.load(Ordering::SeqCst) {
                        return;
                    }
                }
                None => {
                    return;
                }
            }
        }
    }
}

// HTTP 请求（支持自动重试）

pub enum PostResult {
    Ok(reqwest::Response),
    RetryWithoutContextId,
    Err(String),
    /// 命中限流/服务端过载且重试耗尽：携带面向用户的可读提示（不应再显示原始 API JSON）。
    RateLimited(String),
    Aborted,
}

/// 可瞬时恢复、值得退避重试的 HTTP 状态。
fn is_retryable_status(status: u16) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504)
}

/// 计算退避时长：优先尊重服务端 `Retry-After`，否则指数退避（500ms 起，封顶 8s）。
fn backoff_delay(attempt: u32, retry_after: Option<std::time::Duration>) -> std::time::Duration {
    if let Some(d) = retry_after {
        // 服务端给的等待时间通常已合理，仍封顶到 30s 避免极端值卡死。
        return d.min(std::time::Duration::from_secs(30));
    }
    let base_ms: u64 = 500;
    let exp = base_ms.saturating_mul(2u64.saturating_pow(attempt.saturating_sub(1)));
    std::time::Duration::from_millis(exp.min(8000))
}

/// 解析 `Retry-After` 头（仅支持整数秒形式，HTTP 日期形式极少用，忽略）。
fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<std::time::Duration> {
    let v = headers.get(reqwest::header::RETRY_AFTER)?.to_str().ok()?;
    let secs = v.trim().parse::<u64>().ok()?;
    Some(std::time::Duration::from_secs(secs))
}

/// 退避等待期间持续响应中止信号，避免 abort 必须等满整个退避窗口。
async fn sleep_with_abort(d: std::time::Duration, abort_flag: &AtomicBool) {
    let step = std::time::Duration::from_millis(100);
    let mut elapsed = std::time::Duration::ZERO;
    while elapsed < d {
        if abort_flag.load(Ordering::SeqCst) {
            return;
        }
        let remain = d - elapsed;
        tokio::time::sleep(remain.min(step)).await;
        elapsed += remain.min(step);
    }
}

pub async fn execute_post_with_abort(
    url: &str,
    headers: reqwest::header::HeaderMap,
    body: &Value,
    channel: &mut PluginChannel,
    abort_flag: &AtomicBool,
) -> PostResult {
    // 限流/瞬时 5xx/网络抖动：有界重试 + 指数退避，避免一次瞬时错误就中断整轮对话。
    // 重试在同一 turn 内进行（复用同一个 root_id），不会额外产生 Turn/文本节点，
    // 因此不会造成"错误刷屏"。重试耗尽才向上返回错误，由上层停止并展示重试入口。
    const MAX_RETRIES: u32 = 4;
    let mut attempt: u32 = 0;

    loop {
        if abort_flag.load(Ordering::SeqCst) {
            return PostResult::Aborted;
        }

        let result = tokio::select! {
            res = get_http_client().post(url).headers(headers.clone()).json(body).send() => {
                res
            },
            _ = wait_for_abort_signal(channel, abort_flag) => {
                return PostResult::Aborted;
            }
        };

        let response = match result {
            Err(e) => {
                // 网络层错误（连接中断 / DNS / 超时）：可瞬时恢复，退避后重试。
                attempt += 1;
                if attempt <= MAX_RETRIES {
                    let delay = backoff_delay(attempt, None);
                    plugin_warn!(
                        "model",
                        "网络错误({}), 第{}/{}次重试, 退避{:?}",
                        e,
                        attempt,
                        MAX_RETRIES,
                        delay
                    );
                    sleep_with_abort(delay, abort_flag).await;
                    continue;
                }
                return PostResult::Err(format!("网络传输失败: {e}"));
            }
            Ok(r) => r,
        };

        if response.status().is_success() {
            return PostResult::Ok(response);
        }

        // 先读重试头（response 被 text() 消费后再也拿不到 header）。
        let retry_after = parse_retry_after(response.headers());

        let status = response.status();
        let err_text = response.text().await.unwrap_or_default();

        // 处理上下文失效重试（交给上层清 response_id 后重发整轮）。
        if status == 400 && err_text.contains("previous_response_not_found") {
            return PostResult::RetryWithoutContextId;
        }

        if is_retryable_status(status.as_u16()) && attempt < MAX_RETRIES {
            attempt += 1;
            let delay = backoff_delay(attempt, retry_after);
            plugin_warn!(
                "model",
                "HTTP {} 可重试, 第{}/{}次重试, 退避{:?}",
                status,
                attempt,
                MAX_RETRIES,
                delay
            );
            sleep_with_abort(delay, abort_flag).await;
            continue;
        }

        // 不可重试，或重试耗尽：返回面向用户的友好提示（不再回显原始 API JSON）。
        if status.as_u16() == 429 {
            return PostResult::RateLimited(
                "请求过于频繁（429 限流）。请稍后重试，或切换其他模型继续。".to_string(),
            );
        }
        if status.as_u16() >= 500 {
            return PostResult::Err(format!("模型服务暂时不可用（HTTP {status}），请稍后重试。"));
        }
        return PostResult::Err(format!("API Error ({status}): {err_text}"));
    }
}

// 工具调用增量累积

#[derive(Debug, Default, Clone)]
struct AccumulatedToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments: String,
}

/// Tool call information
#[derive(Debug, Clone)]
pub struct ToolCallInfo {
    pub id: Option<String>,
    pub name: Option<String>,
    pub arguments: Value,
}

/// Accumulates incremental tool call deltas.
///
/// LLM APIs stream tool calls incrementally. This struct handles the accumulation
/// so plugin authors don't need to manage index-based HashMaps.
#[derive(Debug, Default)]
pub struct ToolCallAccumulator {
    calls: HashMap<usize, AccumulatedToolCall>,
}

impl ToolCallAccumulator {
    /// Process a tool call delta from the API and return (tool_call_id, accumulated_args, name).
    pub fn process_delta(
        &mut self,
        index: usize,
        id: Option<&str>,
        name: Option<&str>,
        args_delta: Option<&str>,
    ) -> (String, String, Option<String>) {
        let entry = self.calls.entry(index).or_default();

        // 仅接受非空 id/name：
        // 部分 OpenAI 兼容网关（如实测 apinex qwen-3.8-max）只在首个增量携带合法 id，
        // 后续增量重复发送 `id:""`。若用空值覆盖，会把首个增量的合法 id 冲掉，
        // 最终得到 Some("") → 工具调用被误判为 id 缺失而被跳过（历史 Bug）。
        if let Some(id) = id.filter(|s| !s.trim().is_empty()) {
            entry.id = Some(id.to_string());
        }
        if let Some(name) = name.filter(|s| !s.trim().is_empty()) {
            entry.name = Some(name.to_string());
        }

        // 供应商始终未返回 id（缺失或全为空串）时，主动分配一个短 GUID 作为工具调用 id。
        // 该 id 在首个增量即确定并写入 entry，保证流式广播、落库（build_assistant_messages）
        // 与执行（process_tool_calls_async）三处使用同一 id。
        if entry.id.is_none() {
            entry.id = Some(short_id());
        }

        if let Some(delta) = args_delta {
            entry.arguments.push_str(delta);
        }

        (
            entry.id.clone().unwrap_or_default(),
            entry.arguments.clone(),
            entry.name.clone(),
        )
    }

    /// 本次响应是否出现过任何工具调用增量（无论其参数是否完整）。
    ///
    /// 用于区分「纯文本被 `max_tokens` 截断」与「工具调用参数 JSON 被截断」：
    /// 前者可安全自动续写，后者参数已残破、续写无法修复，必须显式报错。
    pub fn had_any_tool_call(&self) -> bool {
        !self.calls.is_empty()
    }

    /// Get the list of completed tool calls.
    ///
    /// 保证返回的每个 ToolCallInfo.id 均为非空：正常情况下 process_delta 已在首个增量
    /// 确定 id，此处为幂等兜底——重复调用返回相同 id，**绝不**重新随机生成
    /// （chat_loop 与 into_messages 会各取一次，两次结果不一致会使工具结果子节点变孤儿）。
    pub fn get_completed(&mut self) -> Vec<ToolCallInfo> {
        self.calls
            .values_mut()
            .map(|call| {
                if call
                    .id
                    .as_ref()
                    .map(|s| s.trim().is_empty())
                    .unwrap_or(true)
                {
                    call.id = Some(short_id());
                }
                let args: Value = match serde_json::from_str(&call.arguments) {
                    Ok(v) => v,
                    Err(e) => {
                        warn!(
                            error = %e,
                            raw_arguments = %call.arguments,
                            "tool call parse error"
                        );
                        serde_json::json!({})
                    }
                };
                ToolCallInfo {
                    id: call.id.clone(),
                    name: call.name.clone(),
                    arguments: args,
                }
            })
            .collect()
    }
}

// 消息构造（ChatMessage 家族，依赖闭包随迁）

/// 生成长度短的 ID（8 字符），替代完整 UUID v4
pub fn short_id() -> String {
    uuid::Uuid::new_v4().to_string()[..8].to_string()
}

/// 流式期间已经广播给前端的子节点 id。
///
/// **落库时必须复用这些 id**（M-001 修复）：流式层（`parse_sse_stream` 的 `emit_update`）
/// 与存储层（`build_assistant_messages`）是同一批节点的两个视图。此前两者各自
/// `short_id()` 生成新 id，导致同一个文本子节点在「前端流式快照」里是 id=A、
/// 在「会话存储」里是 id=B，被上层判定为两条不同消息——于是失败收尾时 id=A 的节点
/// 被当作"尚未落库的流式半截"补写进存储，同一个 Turn 下出现两份内容相同的文本节点。
#[derive(Debug, Default, Clone)]
pub struct StreamChildIds {
    /// 回复正文子节点的流式 id（`TurnOutput::response_text_child_id`）
    pub text: Option<String>,
    /// 思考子节点的流式 id（`TurnOutput::reasoning_child_id`）
    pub reasoning: Option<String>,
}

impl StreamChildIds {
    /// 空串视为「流式期间没有产生该节点」，规范化为 None。
    fn normalized(self) -> Self {
        Self {
            text: self.text.filter(|s| !s.is_empty()),
            reasoning: self.reasoning.filter(|s| !s.is_empty()),
        }
    }
}

/// 构造助手消息组（基于 Turn / ToolCall 的分型层级结构）。
///
/// 结构：
/// - `Turn`(根级, `Assistant` 组合)：与 `User` 互为兄弟
///   ├─ `Reasoning`(子)
///   ├─ `Text`(回复, 子)
/// - `ToolCall`(`Assistant` 组合, 子)：自身 `content` 携带请求参数（JSON 文本）
///   └─ `Text`(响应结果, `Tool`, 子)  ← 由 `build_tool_message` 补充
pub fn build_assistant_messages(
    id: &str,
    content: &str,
    tool_calls: &[ToolCallInfo],
    rid: Option<String>,
    reasoning: Option<String>,
    child_ids: StreamChildIds,
) -> Vec<ChatMessage> {
    let child_ids = child_ids.normalized();
    let mut msgs = Vec::new();
    let timestamp = (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64;

    // ── Turn 消息（根级，与 User 互为兄弟）───────────────────────────────
    msgs.push(ChatMessage {
        id: id.to_string(),
        parent_id: None,
        role: Some(MessageRole::Assistant),
        msg_type: Some(MessageType::Turn),
        content: None,
        status: Some(MessageStatus::Completed),
        timestamp: Some(timestamp),
        ..Default::default()
    });

    // ── Reasoning 消息（parent_id=turn_id）──────────────────────────────
    // 仅当 reasoning 与回复正文为「不同内容」（即存在独立的文本回复）时才单独生成思考子节点。
    // reasoning-only 场景下 `reasoning` 已通过 `content`（effective_text 对「无文本回复」的回退）
    // 承载于下方的 Text 子节点；若此处再生成 Reasoning 子节点，同一段内容会在存储层出现两份
    // （factor=2：表现为历史会话里重复两份、流式期间"层层叠加"）。
    let reasoning_only = reasoning
        .as_ref()
        .map(|r| !r.trim().is_empty() && r.trim() == content.trim())
        .unwrap_or(false);
    if let Some(r) = reasoning {
        if !r.trim().is_empty() && !reasoning_only {
            msgs.push(ChatMessage {
                id: child_ids.reasoning.clone().unwrap_or_else(short_id),
                parent_id: Some(id.to_string()),
                role: Some(MessageRole::Assistant),
                msg_type: Some(MessageType::Reasoning),
                content: Some(MessageContent::Text(r)),
                status: Some(MessageStatus::Completed),
                timestamp: Some(timestamp),
                ..Default::default()
            });
        }
    }

    // ── Response 文本消息（parent_id=turn_id）───────────────────────────
    // 仅在存在非空白文本内容时添加，避免产生仅含 \n\n 的空节点
    if !content.trim().is_empty() {
        // reasoning-only 场景下这块内容在流式层是以 Reasoning 子节点的形式存在的
        // （`finalize_assistant_turn` 会把 `reasoning_child_id` 定稿），因此优先复用
        // reasoning 的流式 id，保证存储层与流式层的节点身份一致。
        let text_child_id = if reasoning_only {
            child_ids
                .reasoning
                .clone()
                .or_else(|| child_ids.text.clone())
        } else {
            child_ids.text.clone()
        };
        msgs.push(ChatMessage {
            id: text_child_id.unwrap_or_else(short_id),
            parent_id: Some(id.to_string()),
            role: Some(MessageRole::Assistant),
            msg_type: Some(MessageType::Text),
            content: Some(MessageContent::Text(content.into())),
            status: Some(MessageStatus::Completed),
            timestamp: Some(timestamp),
            response_id: rid,
            ..Default::default()
        });
    }

    // ── ToolCall 消息（parent_id=turn_id，组合节点）──────────────────────
    // ToolCall 组合节点自身携带请求参数（content = JSON 文本），不再拆分出独立的请求子节点
    for tc in tool_calls {
        let tc_id = tc.id.clone().unwrap_or_else(short_id);
        msgs.push(ChatMessage {
            id: tc_id.clone(),
            parent_id: Some(id.to_string()),
            role: Some(MessageRole::Assistant),
            msg_type: Some(MessageType::ToolCall),
            name: tc.name.clone(),
            content: Some(MessageContent::Text(tc.arguments.to_string())),
            status: Some(MessageStatus::Completed),
            timestamp: Some(timestamp),
            ..Default::default()
        });
    }

    msgs
}

/// 构造工具执行结果消息（role: Tool，msg_type: Text，parent_id 指向 tool_call）。
/// 响应结果作为 `ToolCall` 的直接 `Text`(`Tool`) 子节点（组合节点可选，故不包 Turn）。
///
/// **重要（修复 Bug 2）**：结果子节点的 `status` 必须与实际执行结果一致——
/// 成功 `Completed`、失败 `Failed`。此前硬编码 `Completed`，导致失败工具的结果
/// 子节点被持久化为 `Completed`；当下一轮 `get_context_messages` 过滤掉 `Failed`
/// 的 `ToolCall` 父节点时，这个"孤儿"`role=Tool` 结果子节点（其 `tool_call_id`
/// 指向已被删除的 tool_call）被保留下来，使新一轮 LLM 请求携带非法
/// `tool_call_id` → 请求包出错（"发给大语言模型的数据包会出错"）。
pub fn build_tool_message(
    tool_call_id: &str,
    content: &str,
    success: Option<bool>,
    msg_id: Option<String>,
) -> ChatMessage {
    let success = success.unwrap_or(true);
    ChatMessage {
        id: msg_id.unwrap_or_else(short_id),
        parent_id: Some(tool_call_id.into()),
        role: Some(MessageRole::Tool),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text(content.into())),
        status: Some(if success {
            MessageStatus::Completed
        } else {
            MessageStatus::Failed
        }),
        meta: Some(serde_json::json!({ "success": success })),
        timestamp: Some(
            (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64,
        ),
        ..Default::default()
    }
}

// 单轮产物

#[derive(Default)]
pub struct TurnOutput {
    pub text: String,
    pub reasoning: String,
    pub response_id: Option<String>,
    pub tool_accumulator: ToolCallAccumulator,
    /// Short ID for the response text child node (consistent across delta updates)
    pub response_text_child_id: String,
    /// Short ID for the reasoning child node
    pub reasoning_child_id: String,
    /// 流结束原因（一次响应最多一次）。用于区分「自然结束」与「max_tokens 截断」，
    /// 是修复「对话突然结束」的根因字段。默认 Stop。
    pub finish: FinishReason,
    /// 用量统计（provider 不一定给，故可选）。用于校准 token 估算器。
    pub usage: Option<Usage>,
}

impl TurnOutput {
    pub fn is_reasoning_only(&self, n_tools: usize) -> bool {
        self.text.trim().is_empty() && !self.reasoning.is_empty() && n_tools == 0
    }

    pub fn effective_text(&self, n_tools: usize) -> &str {
        if self.is_reasoning_only(n_tools) {
            &self.reasoning
        } else {
            &self.text
        }
    }

    pub fn into_messages(mut self, root_id: &str, n_tools: usize) -> Vec<ChatMessage> {
        let effective = self.effective_text(n_tools).to_owned();
        let reasoning = if self.reasoning.is_empty() {
            None
        } else {
            Some(self.reasoning)
        };
        let tools = self.tool_accumulator.get_completed();
        build_assistant_messages(
            root_id,
            &effective,
            &tools,
            self.response_id,
            reasoning,
            // 复用流式期间已经广播给前端的子节点 id：落库节点与流式节点必须是同一身份，
            // 否则失败收尾时流式节点会被当成"未落库的半截"再补写一份（重复节点）。
            StreamChildIds {
                text: Some(self.response_text_child_id),
                reasoning: Some(self.reasoning_child_id),
            },
        )
    }
}

// SSE 流解析

/// 解析 SSE 字节流为标准化事件并累积成单轮产物。
///
/// `parse_line` 以闭包接收（`Fn(&str) -> Vec<ProtocolEvent>`）而非协议 trait：
/// 协议差异只体现在「一行 → 事件」的解析一个钩子上，core 无需（也不应）
/// 感知任何协议抽象——调用方（如 model 插件的 `BoundProvider::execute_turn`）
/// 直接传 `|line| protocol.parse_response_line(line)`。
pub async fn parse_sse_stream(
    response: reqwest::Response,
    root_id: &str,
    channel: &mut PluginChannel,
    abort_flag: &AtomicBool,
    parse_line: impl Fn(&str) -> Vec<ProtocolEvent>,
) -> Result<TurnOutput, String> {
    let mut stream = response.bytes_stream();
    let mut buffer = Vec::<u8>::new();
    let mut out = TurnOutput::default();

    // 用于追踪当前行（正在积攒中）已经发送给前端的增量长度，防止重复发送
    #[derive(Default)]
    struct LineProgress {
        content: usize,
        reasoning: usize,
        tool_args: HashMap<usize, usize>,
    }
    let mut progress = LineProgress::default();

    while let Some(chunk) = stream.next().await {
        drain_pending_signals(channel, abort_flag);
        if abort_flag.load(Ordering::SeqCst) {
            break;
        }

        let chunk = chunk.map_err(|e| format!("Stream read error: {e}"))?;
        buffer.extend_from_slice(&chunk);

        // 循环处理缓冲区
        loop {
            // 查找换行符
            let pos = buffer.iter().position(|&b| b == b'\n');

            // 如果找到换行符，或者缓冲区已经累积到足够大（处理超长行）
            if let Some(p) = pos {
                let line_bytes = buffer.drain(..p + 1).collect::<Vec<_>>();
                let line_str = String::from_utf8_lossy(&line_bytes);
                let trimmed = line_str.trim();
                if !trimmed.is_empty() {
                    for mut event in parse_line(trimmed) {
                        // 扣除已经通过增量模式发送的部分
                        match event {
                            ProtocolEvent::ContentDelta(ref mut c) if progress.content > 0 => {
                                *c = safe_substring(c, progress.content);
                            }
                            ProtocolEvent::ReasoningDelta(ref mut r) if progress.reasoning > 0 => {
                                *r = safe_substring(r, progress.reasoning);
                            }
                            ProtocolEvent::ToolCallDelta(idx, _, _, Some(ref mut a)) => {
                                if let Some(&len) = progress.tool_args.get(&idx) {
                                    *a = safe_substring(a, len);
                                }
                            }
                            _ => {}
                        }
                        dispatch_protocol_event(event, root_id, channel, &mut out).await?;
                    }
                }
                // 重置当前行的追踪
                progress = LineProgress::default();
            } else if buffer.len() > 256 {
                // 如果没有换行符但缓冲区较大，尝试“增量提取”
                let line_str = String::from_utf8_lossy(&buffer);
                if let Some(event) = try_parse_partial_sse_line(&line_str) {
                    let mut to_dispatch = None;
                    match event {
                        ProtocolEvent::ContentDelta(c) if c.len() > progress.content => {
                            let delta = safe_substring(&c, progress.content);
                            progress.content = c.len();
                            to_dispatch = Some(ProtocolEvent::ContentDelta(delta));
                        }
                        ProtocolEvent::ReasoningDelta(r) if r.len() > progress.reasoning => {
                            let delta = safe_substring(&r, progress.reasoning);
                            progress.reasoning = r.len();
                            to_dispatch = Some(ProtocolEvent::ReasoningDelta(delta));
                        }
                        ProtocolEvent::ToolCallDelta(idx, id, name, Some(args)) => {
                            let last_len = *progress.tool_args.get(&idx).unwrap_or(&0);
                            if args.len() > last_len {
                                let delta = safe_substring(&args, last_len);
                                progress.tool_args.insert(idx, args.len());
                                to_dispatch =
                                    Some(ProtocolEvent::ToolCallDelta(idx, id, name, Some(delta)));
                            }
                        }
                        _ => {}
                    }
                    if let Some(ev) = to_dispatch {
                        dispatch_protocol_event(ev, root_id, channel, &mut out).await?;
                    }
                }
                break;
            } else {
                break;
            }
        }
    }
    Ok(out)
}

async fn dispatch_protocol_event(
    ev: ProtocolEvent,
    root_id: &str,
    channel: &PluginChannel,
    out: &mut TurnOutput,
) -> Result<(), String> {
    match ev {
        ProtocolEvent::ContentDelta(c) => {
            // Filter out truly empty content, but preserve newlines for markdown formatting
            if c.is_empty() {
                return Ok(());
            }
            out.text.push_str(&c);
            // Each content delta gets its own short ID (for stream updates, the ID stays consistent
            // so the frontend can merge deltas)
            if out.response_text_child_id.is_empty() {
                out.response_text_child_id = short_id();
            }
            emit_update(
                channel,
                ChatMessage {
                    id: out.response_text_child_id.clone(),
                    parent_id: Some(root_id.into()),
                    role: Some(MessageRole::Assistant),
                    msg_type: Some(MessageType::Text),
                    content: Some(MessageContent::Text(c)),
                    status: Some(MessageStatus::Streaming),
                    ..Default::default()
                },
            )
            .await;
        }
        ProtocolEvent::ReasoningDelta(r) => {
            // Filter out truly empty content, but preserve newlines for markdown formatting
            if r.is_empty() {
                return Ok(());
            }
            out.reasoning.push_str(&r);
            if out.reasoning_child_id.is_empty() {
                out.reasoning_child_id = short_id();
            }
            emit_update(
                channel,
                ChatMessage {
                    id: out.reasoning_child_id.clone(),
                    parent_id: Some(root_id.into()),
                    role: Some(MessageRole::Assistant),
                    msg_type: Some(MessageType::Reasoning),
                    content: Some(MessageContent::Text(r)),
                    status: Some(MessageStatus::Streaming),
                    ..Default::default()
                },
            )
            .await;
        }
        ProtocolEvent::ToolCallDelta(idx, id, name, args) => {
            let (tc_id, full_args, full_name) = out.tool_accumulator.process_delta(
                idx,
                id.as_deref(),
                name.as_deref(),
                args.as_deref(),
            );

            // ToolCall 组合节点：自身 content 携带累积的全量请求参数（每次 delta 幂等全量重发，
            // 前端按 tool_call 类型全量替换，最终保证参数完整）；不再拆分独立的请求子节点
            emit_update(
                channel,
                ChatMessage {
                    id: tc_id.clone(),
                    parent_id: Some(root_id.into()),
                    role: Some(MessageRole::Assistant),
                    msg_type: Some(MessageType::ToolCall),
                    name: full_name,
                    content: Some(MessageContent::Text(full_args)),
                    status: Some(MessageStatus::Streaming),
                    ..Default::default()
                },
            )
            .await;
        }
        ProtocolEvent::ResponseId(id) => out.response_id = Some(id),
        ProtocolEvent::Finish(f) => out.finish = f,
        ProtocolEvent::Usage(u) => {
            // 同一响应可能多次收到 Usage（如 Anthropic 的 message_start + message_delta 分别携带
            // input/output tokens）。按字段合并，避免后者覆盖前者丢失数据。
            out.usage = Some(match out.usage {
                Some(prev) => Usage {
                    input: u.input.or(prev.input),
                    output: u.output.or(prev.output),
                },
                None => u,
            });
        }
        ProtocolEvent::Error(e) => return Err(e),
    }
    Ok(())
}

// 辅助解析函数（用于超长单行 SSE 增量提取）

/// 尝试从尚未结束（无换行符）的 SSE 行中提取已有的增量内容。
/// 这是一个“启发式”解析器，主要针对 OpenAI 和 Anthropic 格式。
pub fn try_parse_partial_sse_line(line: &str) -> Option<ProtocolEvent> {
    if !line.starts_with("data: ") {
        return None;
    }

    // 跳过 OpenAI Responses API 的完成事件，这些事件包含全量 text/content，
    // 若被当作增量提取会导致回复内容被重复追加。
    if line.contains("\"type\":\"response.completed\"")
        || line.contains("\"type\":\"response.output_item.done\"")
    {
        return None;
    }

    // 寻找常见的增量字段
    let patterns = [
        ("\"arguments\":\"", "tool_call"),
        ("\"content\":\"", "content"),
        ("\"reasoning_content\":\"", "reasoning"),
        ("\"partial_json\":\"", "tool_call"), // Anthropic
        ("\"text\":\"", "content"),           // Anthropic
    ];

    for (pattern, ev_type) in patterns {
        if let Some(p) = line.find(pattern) {
            let val_start = p + pattern.len();
            if line.len() > val_start {
                let mut raw_val = &line[val_start..];

                // 如果最后是反斜杠，去掉它，因为它可能是一个转义字符的一部分
                if raw_val.ends_with('\\') {
                    raw_val = &raw_val[..raw_val.len() - 1];
                }

                let unescaped = unescape_partial(raw_val);

                return match ev_type {
                    "tool_call" => {
                        let prefix = &line[..val_start];
                        Some(ProtocolEvent::ToolCallDelta(
                            find_idx(prefix),
                            find_id(prefix),
                            find_name(prefix),
                            Some(unescaped),
                        ))
                    }
                    "content" => Some(ProtocolEvent::ContentDelta(unescaped)),
                    "reasoning" => Some(ProtocolEvent::ReasoningDelta(unescaped)),
                    _ => None,
                };
            }
        }
    }
    None
}

fn unescape_partial(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '"' {
            break;
        }
        if c == '\\' {
            match chars.next() {
                Some('"') => result.push('"'),
                Some('n') => result.push('\n'),
                Some('t') => result.push('\t'),
                Some('\\') => result.push('\\'),
                Some(r) => {
                    result.push('\\');
                    result.push(r);
                }
                None => result.push('\\'),
            }
        } else {
            result.push(c);
        }
    }
    result
}

fn find_id(s: &str) -> Option<String> {
    s.find("\"id\":\"").and_then(|p| {
        let start = p + 6;
        s[start..]
            .find('"')
            .map(|end| s[start..start + end].to_string())
    })
}

fn find_name(s: &str) -> Option<String> {
    s.find("\"name\":\"").and_then(|p| {
        let start = p + 8;
        s[start..]
            .find('"')
            .map(|end| s[start..start + end].to_string())
    })
}

fn find_idx(s: &str) -> usize {
    s.find("\"index\":")
        .and_then(|p| {
            let start = p + 8;
            let end = s[start..]
                .find(|c: char| !c.is_ascii_digit())
                .unwrap_or(s[start..].len());
            s[start..start + end].parse().ok()
        })
        .unwrap_or(0)
}

fn safe_substring(s: &str, start: usize) -> String {
    if start >= s.len() {
        return String::new();
    }
    // 确保从合法的字符边界开始
    let mut current = start;
    while current < s.len() && !s.is_char_boundary(current) {
        current += 1;
    }
    s[current..].to_string()
}

#[cfg(test)]
mod tool_call_tests {
    use super::*;

    /// 回归（apinex qwen-3.8-max 网关真实行为）：
    /// 首个增量携带合法 id，后续增量重复发送 `id:""`。
    /// 空串不得覆盖合法 id——否则最终得到 Some("")，工具调用被误判为
    /// id 缺失而跳过，落库的 ToolCall 节点 id 为空串且无结果子节点。
    #[test]
    fn empty_id_delta_does_not_overwrite_real_id() {
        let mut acc = ToolCallAccumulator::default();
        let (id1, _, _) = acc.process_delta(
            0,
            Some("call_8f3a59f5f8e14258a427e432"),
            Some("get_weather"),
            Some(""),
        );
        assert_eq!(id1, "call_8f3a59f5f8e14258a427e432");

        // 后续增量：id:""（该网关的真实行为）
        let (id2, args, _) = acc.process_delta(0, Some(""), None, Some("{\"city\": \"Paris\"}"));
        assert_eq!(id2, "call_8f3a59f5f8e14258a427e432");
        assert_eq!(args, "{\"city\": \"Paris\"}");

        let done = acc.get_completed();
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].id.as_deref(), Some("call_8f3a59f5f8e14258a427e432"));
        assert_eq!(done[0].name.as_deref(), Some("get_weather"));
    }

    /// 需求 1：供应商始终未返回 id 时，必须主动分配短 GUID 作为工具调用 id。
    /// 流式返回值与 get_completed 结果必须一致，且重复取值幂等
    /// （chat_loop 与 into_messages 各取一次，两次结果不一致会使结果子节点变孤儿）。
    #[test]
    fn missing_id_gets_stable_generated_guid() {
        let mut acc = ToolCallAccumulator::default();
        let (stream_id, _, _) =
            acc.process_delta(0, None, Some("dir_list"), Some("{\"path\": \".\"}"));
        assert!(!stream_id.is_empty(), "流式期间即应有非空 id");
        assert_ne!(stream_id, "tc-0", "不得再使用 index 占位符");

        let done1 = acc.get_completed();
        let done2 = acc.get_completed();
        assert_eq!(done1[0].id.as_deref(), Some(stream_id.as_str()));
        assert_eq!(
            done2[0].id.as_deref(),
            Some(stream_id.as_str()),
            "重复调用 get_completed 必须返回同一 id"
        );
    }

    /// 纯空白 id 视为"不合法"，与缺失同等对待。
    #[test]
    fn whitespace_id_treated_as_missing() {
        let mut acc = ToolCallAccumulator::default();
        let (id, _, _) = acc.process_delta(0, Some("   "), None, Some("{}"));
        assert!(!id.trim().is_empty());
    }

    /// 空串 name 不得覆盖首个增量的合法 name（与 id 同理）。
    #[test]
    fn empty_name_delta_does_not_overwrite_real_name() {
        let mut acc = ToolCallAccumulator::default();
        acc.process_delta(0, Some("call_x"), Some("cmd.exe"), Some(""));
        acc.process_delta(0, Some(""), Some(""), Some("{}"));

        let done = acc.get_completed();
        assert_eq!(done[0].id.as_deref(), Some("call_x"));
        assert_eq!(done[0].name.as_deref(), Some("cmd.exe"));
    }

    /// 多个并行工具调用（不同 index）互不干扰，各自持有独立 id。
    #[test]
    fn parallel_tool_calls_keep_separate_ids() {
        let mut acc = ToolCallAccumulator::default();
        acc.process_delta(0, Some("call_a"), Some("f1"), Some("{}"));
        acc.process_delta(1, Some("call_b"), Some("f2"), Some("{}"));

        let mut done = acc.get_completed();
        done.sort_by(|a, b| a.id.cmp(&b.id));
        assert_eq!(done[0].id.as_deref(), Some("call_a"));
        assert_eq!(done[1].id.as_deref(), Some("call_b"));
    }
}

#[cfg(test)]
mod turn_output_tests {
    use super::*;

    fn out(text: &str, reasoning: &str) -> TurnOutput {
        TurnOutput {
            text: text.to_string(),
            reasoning: reasoning.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn is_reasoning_only_requires_empty_text_and_no_tools() {
        // 只有 reasoning、无正文、无工具 → reasoning-only
        assert!(out("", "思考").is_reasoning_only(0));
        // 空白正文同样视为「无文本回复」
        assert!(out("  \n ", "思考").is_reasoning_only(0));
        // 有正文 → 非 reasoning-only
        assert!(!out("回复", "思考").is_reasoning_only(0));
        // 有工具调用 → 非 reasoning-only（reasoning 需保留为独立子节点）
        assert!(!out("", "思考").is_reasoning_only(1));
        // 无 reasoning → 非 reasoning-only
        assert!(!out("", "").is_reasoning_only(0));
    }

    #[test]
    fn effective_text_falls_back_to_reasoning_only_when_reasoning_only() {
        assert_eq!(out("", "思考").effective_text(0), "思考");
        assert_eq!(out("回复", "思考").effective_text(0), "回复");
        // 有工具时不回退，正文为空即为空
        assert_eq!(out("", "思考").effective_text(1), "");
    }

    /// 端到端（纯内存）回归：reasoning-only 的 TurnOutput 落库消息里
    /// 同一段 reasoning 只出现一次，且没有 Reasoning 子节点。
    #[test]
    fn into_messages_reasoning_only_has_no_duplicate_content() {
        let reasoning = "让我想想这个问题的关键点。";
        let msgs = out("", reasoning).into_messages("turn-x", 0);

        assert_eq!(msgs.len(), 2, "应为 Turn + 单个 Text 子节点");
        assert_eq!(msgs[0].msg_type, Some(MessageType::Turn));
        assert!(
            !msgs
                .iter()
                .any(|m| m.msg_type == Some(MessageType::Reasoning)),
            "reasoning-only 不得产生 Reasoning 子节点"
        );

        let occurrences = msgs
            .iter()
            .filter(|m| {
                m.content
                    .as_ref()
                    .map(|c| c.to_text().contains(reasoning))
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(occurrences, 1, "同一段 reasoning 只能落库一份（factor=1）");
    }

    #[test]
    fn into_messages_reasoning_with_reply_keeps_two_children() {
        let msgs = out("这是回复", "这是思考").into_messages("turn-y", 0);
        assert_eq!(msgs.len(), 3);
        assert_eq!(
            msgs.iter()
                .filter(|m| m.msg_type == Some(MessageType::Reasoning))
                .count(),
            1
        );
        assert_eq!(
            msgs.iter()
                .filter(|m| m.msg_type == Some(MessageType::Text))
                .count(),
            1
        );
    }
}
