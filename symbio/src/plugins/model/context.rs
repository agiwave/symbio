//! MODEL 会话编排器 (ChatOrchestrator)
//!
//! 职责：HTTP 客户端单例 + ChatOrchestrator 结构体定义
//!
//! 其余职责分散到：
//! - `chat_loop`      — 主循环入口
//! - `turn_processor` — 单轮消息处理
//! - `tool_executor`   — 工具执行与批量分发
//! - `message_builder` — NativeMessage 构造与 Session 持久化

use super::message_builder::short_id;
use super::protocols::{ModelProtocol, ProtocolEvent};
use super::tool_call::{ToolCallAccumulator, ToolCallInfo};
use super::types::*;
use crate::symbio_core::schemas::{
    session::chat_message::{ChatMessage, MessageStatus, MessageType},
    session::{session_chat_response, session_compress},
};

use crate::plugin_info;
use crate::plugin_warn;
use crate::symbio_core::{
    InvokeRequest, InvokeRequestExt, Plugin, PluginChannel, PluginFrame, SESSION_COMPRESS,
};
use futures::StreamExt;
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

// 全局 HTTP 客户端（单例）

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
async fn emit_update(channel: &PluginChannel, msg: ChatMessage) {
    let _ = channel
        .tx
        .send(PluginFrame::Data(
            serde_json::to_value(session_chat_response::StreamEvent::Update { message: msg })
                .unwrap_or_default(),
        ))
        .await;
}

/// 发送简化的状态更新（仅包含 ID 和 Status）。
async fn emit_status(channel: &PluginChannel, id: String, status: MessageStatus) {
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
        plugin_info!(
            "model",
            "[DIAG] wait_for_abort_signal: abort_flag already true at entry"
        );
        return;
    }

    plugin_info!(
        "model",
        "[DIAG] wait_for_abort_signal: waiting for abort frames"
    );
    while let Some(frame) = channel.rx.recv().await {
        plugin_info!(
            "model",
            "[DIAG] wait_for_abort_signal: received frame {:?}",
            frame
        );
        if handle_signal_frame(frame, abort_flag) {
            plugin_info!(
                "model",
                "[DIAG] wait_for_abort_signal: signal handler returned true, aborting"
            );
            return;
        }
        if abort_flag.load(Ordering::SeqCst) {
            plugin_info!(
                "model",
                "[DIAG] wait_for_abort_signal: abort_flag now true, returning"
            );
            return;
        }
    }
    plugin_warn!("model", "[DIAG] wait_for_abort_signal: channel rx closed (no senders alive), returning -> PostResult::Aborted");
}

// HTTP 请求（支持自动重试）

pub enum PostResult {
    Ok(reqwest::Response),
    RetryWithoutContextId,
    Err(String),
    Aborted,
}

pub async fn execute_post_with_abort(
    url: &str,
    headers: reqwest::header::HeaderMap,
    body: &Value,
    channel: &mut PluginChannel,
    abort_flag: &AtomicBool,
) -> PostResult {
    plugin_info!(
        "model",
        "[DIAG] execute_post_with_abort: url={}, body_keys={:?}",
        url,
        body.as_object().map(|m| m.keys().collect::<Vec<_>>())
    );
    let result = tokio::select! {
        res = get_http_client().post(url).headers(headers).json(body).send() => {
            plugin_info!("model", "[DIAG] execute_post_with_abort: HTTP request future completed first");
            res
        },
        _ = wait_for_abort_signal(channel, abort_flag) => {
            plugin_warn!("model", "[DIAG] execute_post_with_abort: wait_for_abort_signal returned first -> Aborted");
            return PostResult::Aborted;
        }
    };

    let response = match result {
        Err(e) => return PostResult::Err(format!("网络传输失败: {e}")),
        Ok(r) => r,
    };

    if response.status().is_success() {
        return PostResult::Ok(response);
    }

    let status = response.status();
    let err_text = response.text().await.unwrap_or_default();

    // 处理上下文失效重试
    if status == 400 && err_text.contains("previous_response_not_found") {
        return PostResult::RetryWithoutContextId;
    }

    PostResult::Err(format!("API Error ({status}): {err_text}"))
}

// SSE 流解析与结果积累

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

    pub fn into_messages(
        self,
        root_id: &str,
        n_tools: usize,
    ) -> Vec<crate::symbio_core::schemas::session::chat_message::ChatMessage> {
        let effective = self.effective_text(n_tools).to_owned();
        let reasoning = if self.reasoning.is_empty() {
            None
        } else {
            Some(self.reasoning)
        };
        let tools = self.tool_accumulator.get_completed();
        super::message_builder::build_assistant_messages(
            root_id,
            &effective,
            &tools,
            self.response_id,
            reasoning,
        )
    }
}

pub async fn parse_sse_stream(
    response: reqwest::Response,
    root_id: &str,
    channel: &mut PluginChannel,
    abort_flag: &AtomicBool,
    protocol: &dyn ModelProtocol,
) -> Result<TurnOutput, String> {
    let mut stream = response.bytes_stream();
    let mut buffer = Vec::<u8>::new();
    let mut out = TurnOutput::default();

    // 用于追踪当前行（正在积攒中）已经发送给前端的增量长度，防止重复发送
    #[derive(Default)]
    struct LineProgress {
        content: usize,
        reasoning: usize,
        tool_args: std::collections::HashMap<usize, usize>,
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
                    for mut event in protocol.parse_response_line(trimmed) {
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
        ProtocolEvent::Error(e) => return Err(e),
    }
    Ok(())
}

pub async fn compress_temporary_messages(
    parent: &Option<Arc<dyn Plugin>>,
    session_id: &str,
    messages: &mut [ChatMessage],
    ctx: Arc<dyn InvokeRequest>,
) {
    if messages.is_empty() {
        return;
    }

    // “除最后一轮外”：这里简单处理，保留最后一条消息（通常是当前用户输入）不被主动压缩
    let split_at = messages.len().saturating_sub(1);
    if split_at == 0 {
        return;
    }

    let to_compress = &messages[..split_at];

    let compress_ctx = ctx.fork();
    compress_ctx.set(crate::symbio_core::PATH, SESSION_COMPRESS.to_string());
    let _ = compress_ctx.set_payload(serde_json::json!({
        "session_id": session_id,
        "messages": to_compress,
    }));

    if let Some(p) = parent {
        if let Ok(resp) = p.clone().route(compress_ctx).await {
            if let Ok(res) = resp.get::<session_compress::Response>() {
                // 更新消息列表的前半部分
                for (i, msg) in res.messages.into_iter().enumerate() {
                    if i < split_at {
                        messages[i] = msg;
                    }
                }
            }
        }
    }
}

// ChatOrchestrator

pub struct ChatOrchestrator {
    pub config: ModelConfig,
    pub parent: Option<Arc<dyn Plugin>>,
    pub protocol: Box<dyn ModelProtocol>,
}

impl ChatOrchestrator {
    pub fn new(
        config: ModelConfig,
        parent: Option<Arc<dyn Plugin>>,
        protocol: Box<dyn ModelProtocol>,
    ) -> Self {
        Self {
            config,
            parent,
            protocol,
        }
    }

    pub async fn finalize_assistant_turn(
        &self,
        root_id: &str,
        out: &TurnOutput,
        tools: &[ToolCallInfo],
        channel: &PluginChannel,
    ) {
        if out.is_reasoning_only(tools.len()) {
            // reasoning-only：模型只产生了 reasoning，没有独立的文本回复。
            //
            // 同一段 reasoning 在落库时由 build_assistant_messages 以「Text 响应子节点」承载
            // （effective_text 对「无文本回复」的回退语义）。因此这里**绝不能**再额外广播一个
            // content=reasoning 的 Text 节点——否则前端会同时持有「Reasoning 子节点」与
            // 「Text 响应子节点」两份相同内容，表现为：
            //   · 流式期间：思考块 + 一段相同文本先后出现，看起来像"同一段文本被重复写入"；
            //   · 历史刷新后：存储层本就重复（factor≈2），渲染出两份。
            //
            // 流式期间 ReasoningDelta 已经把 reasoning 累积进 reasoning_child_id 节点，
            // 此处仅将其与根 Turn 标记 Completed 即可。仅当流式期间因故未建立 Reasoning 节点时，
            // 才补发一个 Text 节点兜底（此时不存在 Reasoning 节点，不会造成重复）。
            if !out.reasoning_child_id.is_empty() {
                emit_status(
                    channel,
                    out.reasoning_child_id.clone(),
                    MessageStatus::Completed,
                )
                .await;
            } else {
                let resp_id = if out.response_text_child_id.is_empty() {
                    short_id()
                } else {
                    out.response_text_child_id.clone()
                };
                emit_update(
                    channel,
                    ChatMessage {
                        id: resp_id,
                        parent_id: Some(root_id.into()),
                        role: Some(MessageRole::Assistant),
                        msg_type: Some(MessageType::Text),
                        content: Some(MessageContent::Text(out.reasoning.clone())),
                        status: Some(MessageStatus::Completed),
                        ..Default::default()
                    },
                )
                .await;
            }
            emit_status(channel, root_id.into(), MessageStatus::Completed).await;
            return;
        }

        // Mark reasoning child as completed
        if !out.reasoning.is_empty() && !out.reasoning_child_id.is_empty() {
            emit_status(
                channel,
                out.reasoning_child_id.clone(),
                MessageStatus::Completed,
            )
            .await;
        }

        // Mark response text child as completed (exists if there was text content)
        if !out.text.is_empty() && !out.response_text_child_id.is_empty() {
            emit_status(
                channel,
                out.response_text_child_id.clone(),
                MessageStatus::Completed,
            )
            .await;
        }

        // Mark tool calls (composite) as completed
        for tc in tools {
            if let Some(tc_id) = &tc.id {
                emit_status(channel, tc_id.clone(), MessageStatus::Completed).await;
            }
        }

        // Mark the root Turn node as completed
        emit_status(channel, root_id.into(), MessageStatus::Completed).await;
    }
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
mod turn_output_tests {
    use super::*;
    use crate::symbio_core::schemas::session::chat_message::MessageType;

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
