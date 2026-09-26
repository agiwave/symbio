//! SSE 流循环 —— 把响应字节流转成单轮产物（core `llm/` 契约的实现细节）。
//!
//! 只有 model 插件使用，故住在插件内而非 core `llm/`。行解析契约
//! [`SseLineParser`] 与其实现方同处本插件（[`super::protocols::sse`]），core 只
//! 提供 [`TurnOutput`] 产物结构；本模块负责按 `\n` 切分、按前缀截断去重、把协议事件
//! 分发给 [`TurnOutput`] 并经 `sink` 实时下发流式子节点——**不认识任何协议字段名**。
//!
//! 生命周期日志（请求发起 → 响应头 → 首块 → 首条内容 → 流结束）让卡死可定位到阶段，
//! 阶段对照表见 [`parse_sse_stream`] 注释。

use crate::plugin_error;
use crate::plugin_info;
use crate::plugin_warn;
use crate::plugins::model::http::STREAM_IDLE_TIMEOUT;
use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
};
use crate::symbio_core::ModelUsage;
use crate::symbio_core::{emit_delta, emit_message, short_id, TurnOutput};
use crate::symbio_core::{ExecAbortSignal, ExecEventSink};
use futures::StreamExt;
use std::collections::HashMap;

use super::protocols::{utf8_chunk, ModelProtocolEvent, SseLineParser, SsePartialLineExtractor};

/// 未结束的行超过这个长度才尝试增量提取。
///
/// 太短时开提取器没有意义（下一块多半就把整行补齐了），反而多付一次字段扫描。
const PARTIAL_LINE_MIN_BYTES: usize = 256;

/// 当前「未结束行」已经发送给前端的增量长度（按字段分别记）。
///
/// ## 为什么必须记账
///
/// 同一段文本有两条到达路径：**未结束的行**由协议层的增量提取器边收边吐，
/// **完整行**由 `SseLineParser::parse_line` 一次性给出全量。若不记账，完整行
/// 到达时会把已经发过的前缀**再发一遍**（前端重复）。因此完整行路径按这里的
/// 长度截断，而增量路径每发一次就累加一次。
///
/// 这套机制成立的前提是：增量路径吐出的文本**恰好是**完整行文本的前缀。
/// 该不变量由协议层的提取器保证（见 `crate::symbio_core::sse`）——它必须
/// 与协议解析器用**同一套转义规则**，否则按前缀截断会吃字。
#[derive(Default)]
struct LineProgress {
    content: usize,
    reasoning: usize,
    tool_args: HashMap<usize, usize>,
}

impl LineProgress {
    /// 记一次增量并返回可转发的事件；**非增量类事件一律丢弃**。
    ///
    /// 未结束的行里只有「字符串值还在长」这件事是确定的，`finish` / `usage` /
    /// `error` 这些字段在半截 JSON 里读到的值不可信——等完整行。
    fn record(&mut self, ev: ModelProtocolEvent) -> Option<ModelProtocolEvent> {
        match ev {
            ModelProtocolEvent::ContentDelta(c) => {
                self.content += c.len();
                Some(ModelProtocolEvent::ContentDelta(c))
            }
            ModelProtocolEvent::ReasoningDelta(r) => {
                self.reasoning += r.len();
                Some(ModelProtocolEvent::ReasoningDelta(r))
            }
            ModelProtocolEvent::ToolCallDelta(idx, id, name, Some(a)) => {
                *self.tool_args.entry(idx).or_insert(0) += a.len();
                Some(ModelProtocolEvent::ToolCallDelta(idx, id, name, Some(a)))
            }
            _ => None,
        }
    }
}

/// 解析 SSE 字节流为标准化事件并累积成单轮产物。
///
/// `parser` 是**协议层**提供的行解析契约（`SseLineParser`）：它既能解析完整行，
/// 也能为「尚未结束的行」开一个增量提取器。本模块只负责按 `\n` 切分、按前缀截断
/// 去重、把事件交给 `dispatch_and_track`——**不认识任何协议字段名**。
///
/// 历史上这里收的是闭包 `Fn(&str) -> Vec<ModelProtocolEvent>`，增量提取则由内置的
/// 启发式解析器代劳（硬编码 `"content":"` / `"partial_json":"` 等字段名）。那套写法
/// 有三重问题：加协议要改循环、增量与完整行两套转义规则（会吃字）、每块重扫整行
/// （O(n²)）。契约拆成两个方法后，三件事一起解决。
///
/// 流式产出的子节点经 `sink` 实时下发（进程内直连转写唯一写入点）；
/// 中止只经 `abort` 感知（不再在流循环里排空控制帧）。
pub async fn parse_sse_stream(
    response: reqwest::Response,
    root_id: &str,
    sink: &ExecEventSink,
    abort: &ExecAbortSignal,
    parser: &dyn SseLineParser,
) -> Result<TurnOutput, String> {
    let mut stream = response.bytes_stream();
    let mut buffer = Vec::<u8>::new();
    let mut out = TurnOutput::default();

    // ── 生命周期日志与卡死防护 ──────────────────────────────────────────
    // 让每轮 SSE 流在控制台留下完整轨迹：何时建立、首字节何时到达、首条内容
    // 何时产出、正常/异常如何结束。若卡死，可从最后一条日志精确定位阶段：
    //   有「请求发起」无「响应头」     → 卡在 POST 等待（网关/网络）；
    //                                    「请求发起」由调用方在构建请求体后打出，
    //                                    「响应头到达」在 http.rs 的 `PostResult::Ok` 前打出。
    //   有「响应头」无「首个数据块」   → 卡在 SSE 建立后无数据（provider 挂起）；
    //   有「首个数据块」无「首条内容」 → 收到字节但协议解析无内容事件（协议异常）；
    //   有「首条内容」后长时间静默     → 流中途挂起 → 由 STREAM_IDLE_TIMEOUT 兜底报错。
    let started = std::time::Instant::now();
    let mut first_chunk_at: Option<std::time::Instant> = None;
    let mut first_content_logged = false;
    let mut chunk_count: u64 = 0;
    let mut total_bytes: u64 = 0;

    let mut progress = LineProgress::default();

    // 当前「未结束的行」的增量提取状态（见 `llm::sse`）。
    // - `partial`：协议层给的提取器；`None` = 本行没有（尚未开 / 已放弃）
    // - `partial_gave_up`：本行已问过协议、答案是「不做」——不再重试，
    //   否则每一块都要把整行重扫一遍，正是收口前那个 O(n²)
    // - `partial_fed`：已喂给提取器的字节位置（只喂新增部分）
    let mut partial: Option<Box<dyn SsePartialLineExtractor>> = None;
    let mut partial_gave_up = false;
    let mut partial_fed = 0usize;
    // 增量提取器产出事件的暂存区（跨块复用，见 ② 处说明）
    let mut partial_out: Vec<ModelProtocolEvent> = Vec::new();

    loop {
        // 空闲超时包裹：流若中途挂起（连接在、数据停），最多等 STREAM_IDLE_TIMEOUT
        // 就显式报错终止，而不是静默挂到整体 1800s 超时（用户视角即「卡死」）。
        let next = match tokio::time::timeout(STREAM_IDLE_TIMEOUT, stream.next()).await {
            Ok(n) => n,
            Err(_) => {
                let msg = format!(
                    "SSE 流空闲超时（{}s 内无任何数据）。模型服务可能已挂起，本轮终止以避免会话卡死（已收到 {} chunks / {} bytes，流持续 {:?}）",
                    STREAM_IDLE_TIMEOUT.as_secs(),
                    chunk_count,
                    total_bytes,
                    started.elapsed()
                );
                plugin_error!("model", "[LLM] {}", msg);
                return Err(msg);
            }
        };
        let Some(chunk) = next else {
            break; // 流自然结束
        };

        if first_chunk_at.is_none() {
            first_chunk_at = Some(std::time::Instant::now());
            plugin_info!(
                "model",
                "[LLM] 首个数据块到达 (首字节延迟 {:?})",
                first_chunk_at.unwrap().duration_since(started)
            );
        }
        chunk_count += 1;
        total_bytes += chunk.as_ref().map(|c| c.len()).unwrap_or(0) as u64;

        if abort.is_aborted() {
            plugin_info!(
                "model",
                "[LLM] 流式响应被中止 (已收 {} chunks / {} bytes, 耗时 {:?})",
                chunk_count,
                total_bytes,
                started.elapsed()
            );
            break;
        }

        let chunk = chunk.map_err(|e| {
            let msg = format!(
                "Stream read error: {e} (已收 {} chunks / {} bytes, 耗时 {:?})",
                chunk_count,
                total_bytes,
                started.elapsed()
            );
            plugin_error!("model", "[LLM] {}", msg);
            msg
        })?;
        buffer.extend_from_slice(&chunk);

        // ① 先把**完整的行**全部消化掉（一块里可能不止一行）
        while let Some(p) = buffer.iter().position(|&b| b == b'\n') {
            let line_bytes = buffer.drain(..p + 1).collect::<Vec<_>>();
            let line_str = String::from_utf8_lossy(&line_bytes);
            let trimmed = line_str.trim();
            if !trimmed.is_empty() {
                for mut event in parser.parse_line(trimmed) {
                    // 扣除已经通过增量模式发送的部分
                    match event {
                        ModelProtocolEvent::ContentDelta(ref mut c) if progress.content > 0 => {
                            *c = safe_substring(c, progress.content);
                        }
                        ModelProtocolEvent::ReasoningDelta(ref mut r) if progress.reasoning > 0 => {
                            *r = safe_substring(r, progress.reasoning);
                        }
                        ModelProtocolEvent::ToolCallDelta(idx, _, _, Some(ref mut a)) => {
                            if let Some(&len) = progress.tool_args.get(&idx) {
                                *a = safe_substring(a, len);
                            }
                        }
                        _ => {}
                    }
                    dispatch_and_track(
                        event,
                        root_id,
                        sink,
                        &mut out,
                        &mut first_content_logged,
                        started,
                    )
                    .await?;
                }
            }
            // 换行 ⇒ 本行的增量提取作废、记账归零（下一行重新开始）
            progress = LineProgress::default();
            partial = None;
            partial_gave_up = false;
            partial_fed = 0;
        }

        // ② 尾巴（尚未结束的行）：交给协议层的增量提取器边收边吐。
        //
        // 只在超过阈值时才尝试——短尾巴等下一块补齐即可，开提取器纯属浪费。
        if buffer.len() > PARTIAL_LINE_MIN_BYTES {
            // 复用同一个 `Vec` 接住提取器产出的事件（每块最多几条，但每块都新建
            // 一个 `Vec` 会在长流上累积成可观的分配量）。
            let mut events = std::mem::take(&mut partial_out);
            if let Some(ext) = partial.as_mut() {
                if buffer.len() > partial_fed {
                    // 只喂**新增**字节，且不消费被切断的多字节字符
                    let (tail, consumed) = utf8_chunk(&buffer, partial_fed);
                    partial_fed = consumed;
                    ext.push(tail, &mut events);
                }
            } else if !partial_gave_up {
                let (head, consumed) = utf8_chunk(&buffer, 0);
                match parser.open_partial_line(head) {
                    // 协议声明本行不做增量提取：记下，本行不再问第二次
                    None => partial_gave_up = true,
                    Some(mut ext) => {
                        // 注意：`head` 可能比 `buffer` 短（尾部多字节字符被切断），
                        // 消费位必须取 `consumed` 而不是 `buffer.len()`，否则那几个
                        // 残字节会被永久跳过、字符丢失。
                        partial_fed = consumed;
                        ext.push(head, &mut events);
                        partial = Some(ext);
                    }
                }
            }
            for ev in events.drain(..) {
                if let Some(ev) = progress.record(ev) {
                    dispatch_and_track(
                        ev,
                        root_id,
                        sink,
                        &mut out,
                        &mut first_content_logged,
                        started,
                    )
                    .await?;
                }
            }
            partial_out = events; // 归还容量
        }
    }

    // ③ 流结束日志：正常结束 / 中止 / 空流，均带统计信息。
    // 若此处之后长时间无下文（工具执行/下一轮请求），可据此定位卡死发生在「流结束后」阶段。
    if abort.is_aborted() {
        // 中止已在上方记录，此处不重复。
    } else if out.text.is_empty()
        && out.reasoning.is_empty()
        && !out.tool_accumulator.had_any_tool_call()
    {
        plugin_warn!(
            "model",
            "[LLM] 流结束但未产出任何内容（空流，{} chunks / {} bytes, 耗时 {:?}, finish={:?}）——上游可能返回了错误页或空响应",
            chunk_count,
            total_bytes,
            started.elapsed(),
            out.finish
        );
    } else {
        plugin_info!(
            "model",
            "[LLM] 流正常结束 (耗时 {:?}, {} chunks / {} bytes, 文本 {} 字符, 推理 {} 字符, 工具调用 {} 个, finish={:?})",
            started.elapsed(),
            chunk_count,
            total_bytes,
            out.text.len(),
            out.reasoning.len(),
            out.tool_accumulator.get_completed().len(),
            out.finish
        );
    }
    Ok(out)
}

/// 单个协议事件的分发 + 首条内容日志。
///
/// ④「获取到第一条消息」日志：第一条文本/推理/工具调用内容事件到达时打点，
/// 标志「模型已开始实际产出」。此后若卡死，可确定卡在「产出过程中」或「产出完成后」。
async fn dispatch_and_track(
    ev: ModelProtocolEvent,
    root_id: &str,
    sink: &ExecEventSink,
    out: &mut TurnOutput,
    first_content_logged: &mut bool,
    started: std::time::Instant,
) -> Result<(), String> {
    if !*first_content_logged {
        let kind = match &ev {
            ModelProtocolEvent::ContentDelta(_) => Some("文本"),
            ModelProtocolEvent::ReasoningDelta(_) => Some("推理"),
            ModelProtocolEvent::ToolCallDelta(..) => Some("工具调用"),
            _ => None,
        };
        if let Some(kind) = kind {
            *first_content_logged = true;
            plugin_info!(
                "model",
                "[LLM] 收到第一条消息（{}内容，自流建立 {:?}）",
                kind,
                started.elapsed()
            );
        }
    }
    dispatch_protocol_event(ev, root_id, sink, out).await
}

async fn dispatch_protocol_event(
    ev: ModelProtocolEvent,
    root_id: &str,
    sink: &ExecEventSink,
    out: &mut TurnOutput,
) -> Result<(), String> {
    match ev {
        ModelProtocolEvent::ContentDelta(c) => {
            // Filter out truly empty content, but preserve newlines for markdown formatting
            if c.is_empty() {
                return Ok(());
            }
            out.text.push_str(&c);
            // 首个增量：发**完整快照**创建正文子节点（id 此后保持一致）；
            // 后续增量：显式 Append 窄帧，消费循环按 id 追加——不重发整条，
            // 也不需要消费端从「补丁形状」里猜这是追加还是替换。
            if out.response_text_child_id.is_empty() {
                out.response_text_child_id = short_id();
                emit_message(
                    sink,
                    ChatMessage {
                        id: out.response_text_child_id.clone(),
                        parent_id: Some(root_id.into()),
                        role: Some(MessageRole::Assistant),
                        msg_type: Some(MessageType::Text),
                        content: Some(MessageContent::Text(out.text.clone())),
                        status: Some(MessageStatus::Streaming),
                        ..Default::default()
                    },
                )
                .await;
            } else {
                emit_delta(sink, &out.response_text_child_id, &c).await;
            }
        }
        ModelProtocolEvent::ReasoningDelta(r) => {
            // Filter out truly empty content, but preserve newlines for markdown formatting
            if r.is_empty() {
                return Ok(());
            }
            out.reasoning.push_str(&r);
            if out.reasoning_child_id.is_empty() {
                out.reasoning_child_id = short_id();
                emit_message(
                    sink,
                    ChatMessage {
                        id: out.reasoning_child_id.clone(),
                        parent_id: Some(root_id.into()),
                        role: Some(MessageRole::Assistant),
                        msg_type: Some(MessageType::Reasoning),
                        content: Some(MessageContent::Text(out.reasoning.clone())),
                        status: Some(MessageStatus::Streaming),
                        ..Default::default()
                    },
                )
                .await;
            } else {
                emit_delta(sink, &out.reasoning_child_id, &r).await;
            }
        }
        ModelProtocolEvent::ToolCallDelta(idx, id, name, args) => {
            let (tc_id, wire_id, full_args, full_name, snapshot_required) = out
                .tool_accumulator
                .process_delta(idx, id.as_deref(), name.as_deref(), args.as_deref());

            // ToolCall 组合节点：自身 content 即**请求参数**（不设独立的请求子节点，
            // 见 `docs/node-state-streaming.md` §2.2）。两条路径与 Text / Reasoning
            // **同构**——帧面只有两种语义，由覆盖方式决定，接收端不做类型推断：
            // - 身份字段变化（新建节点 / 首次定名）→ 完整快照（`Upsert`）；
            // - 纯参数增长 → 窄追加（`Append`，O(delta)），接收端尾部拼接。
            if snapshot_required {
                emit_message(
                    sink,
                    ChatMessage {
                        id: tc_id.clone(),
                        parent_id: Some(root_id.into()),
                        role: Some(MessageRole::Assistant),
                        msg_type: Some(MessageType::ToolCall),
                        name: full_name,
                        content: Some(MessageContent::Text(full_args)),
                        status: Some(MessageStatus::Streaming),
                        tool_call_id: Some(wire_id),
                        ..Default::default()
                    },
                )
                .await;
            } else if let Some(delta) = args.as_deref().filter(|d| !d.is_empty()) {
                emit_delta(sink, &tc_id, delta).await;
            }
        }
        ModelProtocolEvent::ResponseId(id) => out.response_id = Some(id),
        ModelProtocolEvent::Finish(f) => out.finish = f,
        ModelProtocolEvent::Usage(u) => {
            // 同一响应可能多次收到 Usage（如 Anthropic 的 message_start + message_delta 分别携带
            // input/output tokens）。按字段合并，避免后者覆盖前者丢失数据。
            out.usage = Some(match out.usage {
                Some(prev) => ModelUsage {
                    input: u.input.or(prev.input),
                    output: u.output.or(prev.output),
                },
                None => u,
            });
        }
        ModelProtocolEvent::Error(e) => return Err(e),
    }
    Ok(())
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
