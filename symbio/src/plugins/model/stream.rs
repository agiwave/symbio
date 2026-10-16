//! SSE 流循环 —— 把响应字节流转成单轮产物（core `llm/` 契约的实现细节）。
//! 只有 model 插件使用，故住在插件内而非 core `llm/`。行解析契约
//! [`SseLineParser`] 与其实现方同处本插件（[`super::protocols`]），core 只
//! 提供 [`TurnOutput`] 产物结构；本模块负责按 `\n` 切分、按前缀截断去重、把协议事件
//! 分发给 [`TurnOutput`] 并经 `sink` 实时下发流式子节点——**不认识任何协议字段名**。
//! 生命周期日志（请求发起 → 响应头 → 首块 → 首条内容 → 流结束）让卡死可定位到阶段，
//! 阶段对照表见 [`parse_sse_stream`] 注释。
use crate::plugin_error;
use crate::plugin_info;
use crate::plugin_warn;
use crate::plugins::model::http::STREAM_IDLE_TIMEOUT;
use crate::symbio_core::chat_node::{
    ChatNode, MessageContent, NodeKind, NodeRole, NodeStatus, CHAT_STYLE_TAG_REASONING,
    CHAT_STYLE_TAG_TOOL,
};
use crate::symbio_core::ModelUsage;
use crate::symbio_core::{llm_emit_message, llm_short_id, TurnOutput, TurnToolCallInfo};
use crate::symbio_core::{ExecAbortSignal, ExecEventSink};
use futures::StreamExt;
use std::collections::HashMap;

use super::protocols::{utf8_chunk, ModelProtocolEvent, SseLineParser, SsePartialLineExtractor};
use super::tool_accumulator::TurnToolCallAccumulator;

/// 未结束的行超过这个长度才尝试增量提取。
/// 太短时开提取器没有意义（下一块多半就把整行补齐了），反而多付一次字段扫描。
const PARTIAL_LINE_MIN_BYTES: usize = 256;

/// 单次响应内工具调用数上限。
/// 为什么必须存在：`6cf9520d` 号会话里，模型在**一条响应**内流式输出了 56 个工具
/// 调用（同一次复读里 9 个调用原样重复 6 遍），而批执行器要等流结束才开工 → 60 余秒
/// 零结果，用户视角即「工具永远不闭合的死循环」，只能手动终止。空闲超时防不了它
/// （复读也是字节）。
///
/// 处置是**丢弃 + 反馈**，不是整轮失败：超限的调用不再上线 UI 节点（防刷屏），
/// 流结束后由 [`mark_overflow_dropped`] 打上丢弃标记，执行层据此各落一条结果
/// 子节点、正文即提示——模型在下一轮从工具结果里读到「超限了、后面的调用
/// 丢了、该分批发起」。整轮 `Err` 会让模型对发生了什么一无所知（流程无故中断），
/// 已被否决。见 `dispatch_and_track` 的 ToolCallDelta 分支。
const MAX_TOOL_CALLS_PER_RESPONSE: usize = 32;

/// 超限丢弃调用的**工具结果正文**（模型下一轮读到的反馈）。
fn overflow_notice_text() -> String {
    format!(
        "本次调用未执行：本条响应内的工具调用数超过上限（{MAX_TOOL_CALLS_PER_RESPONSE}），\
         超出上限的调用已被系统丢弃。请减少单轮发起的工具调用数量（至多 \
         {MAX_TOOL_CALLS_PER_RESPONSE} 个），把剩余调用放到收到结果后的下一轮再发起。"
    )
}

/// 收口：把上限之外的调用打上丢弃标记（正文 = [`overflow_notice_text`]）。
/// `tool_calls` 按**首次出现顺序**排列（累加器 `finish` 保序），故位置 ≥ 上限
/// 之位的即超限调用。执行层读到 `overflow_notice` 非空就不执行、直接落结果。
fn mark_overflow_dropped(tool_calls: &mut [TurnToolCallInfo]) {
    let notice = overflow_notice_text();
    for tc in tool_calls.iter_mut().skip(MAX_TOOL_CALLS_PER_RESPONSE) {
        tc.overflow_notice = Some(notice.clone());
    }
}

// ── 流阶段失败的分类与措辞 ────────────────────────────────────────────
//
// `http.rs` 的请求级重试只管「响应头之前」（429 / 5xx / 网络层），流一旦建立就
// 再也没有任何重试：网关在流中途回一个错误体、或 TCP 被对端关掉（实测 AMD 网关
// 一次 `{"code":"streaming_error",…UND_ERR_SOCKET…}`（会话 `c2a87a21`），整轮
// 直接失败。同一种瞬时故障，发生在响应头之前会被重试 4 次、发生在 1ms 之后则
// 必死——这个不对称就是「会话莫名卡住、只能手动重发」的成因。
//
// 于是分两件事做：把失败**分类**（瞬时 / 非瞬时，供调用方决定是否原样重发），
// 并把措辞**收敛**（与 `http.rs` 221 行同一条纪律：不回显原始 API JSON）。
/// SSE 阶段的失败：面向用户的提示 + 是否瞬时（值得原样重发）。
#[derive(Debug)]
pub struct StreamFailure {
    /// 面向用户的提示。**不含原始网关 JSON**。
    pub message: String,
    /// 瞬时故障：同一份请求体原样重发有望成功。
    pub transient: bool,
}

/// 瞬时故障关键词（小写包含匹配）。
const TRANSIENT_MARKERS: &[&str] = &[
    "und_err_socket",
    "socket",
    "other side closed",
    "upstream stream terminated",
    "upstream",
    "streaming_error",
    "gateway_error",
    "502",
    "503",
    "504",
    "timeout",
    "timed out",
    "connection",
    "reset by peer",
    "prematurely",
    "incomplete",
    "broken pipe",
    "stream read error",
    "空闲超时",
];

/// 非瞬时故障关键词：命中则**不重试**——原样重发注定再失败一次，只会让用户多等。
const PERMANENT_MARKERS: &[&str] = &[
    "api key",
    "api_key",
    "authentication",
    "unauthorized",
    "401",
    "403",
    "permission",
    "quota",
    "insufficient",
    "context length",
    "context_length_exceeded",
    "invalid_api_key",
    "invalid_request",
    "model not found",
    "not_found",
];

/// 把协议层的原始错误载荷归成一个 [`StreamFailure`]。
///
/// `raw` 可能是三种东西：网关错误体（JSON，`{"error":{"message":…}}` 或
/// `{"message":…}`）、协议适配器取出的 message 字符串、或 reqwest 的
/// IO 错误。
/// 判据只看文本内容，不猜形状——形状千差万别，而「这条该不该重试」是有共性的。
pub(crate) fn classify_stream_error(raw: &str) -> StreamFailure {
    let lower = raw.to_lowercase();
    // 非瞬时优先：`403` 与 `connection` 同现时不该重试。
    let permanent = PERMANENT_MARKERS.iter().any(|m| lower.contains(m));
    let transient = !permanent && TRANSIENT_MARKERS.iter().any(|m| lower.contains(m));
    let detail = extract_error_detail(raw);

    let message = if permanent {
        format!("模型服务拒绝了本次请求：{detail}")
    } else if transient {
        format!("模型连接中断：{detail}。上游流在完成前终止，请重试或切换模型。")
    } else {
        format!("模型流式响应异常：{detail}")
    };
    StreamFailure { message, transient }
}

/// 从原始错误载荷里取出**人读的一行**。
///
/// 网关把可读原因放在 `error.message` / `message`，其余（`details.cause`、
/// `errorCode`、`statusCode`…）是给机器看的实现细节——糊在错误条上只会淹没重点。
/// 一个字段都取不到时截断原文：宁可少给信息，也不回显整坨 JSON。
fn extract_error_detail(raw: &str) -> String {
    use serde_json::Value;
    if let Ok(json) = serde_json::from_str::<Value>(raw) {
        for path in [
            &["error", "message"][..],
            &["message"][..],
            &["error", "type"][..],
            &["detail"][..],
        ] {
            let mut cur = Some(&json);
            for key in path {
                cur = cur.and_then(|c| c.get(*key));
            }
            if let Some(s) = cur.and_then(|v| v.as_str()) {
                let s = s.trim();
                if !s.is_empty() {
                    return s.to_string();
                }
            }
        }
    }
    truncate_for_display(raw.trim(), 160)
}

/// 按字符数截断（不是字节）：错误里常有中文，按字节切会切出非法 UTF-8。
fn truncate_for_display(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    format!("{head}…")
}

/// 发送一帧**增量**：`delta` 追加到目标节点正文尾部（流式热路径，O(delta)）。
/// 目标未知时写入点用帧内信息建占位（帧自给自足，不依赖任何先行帧）。
/// 原住 `symbio_core::llm::turn`（帧家族），按 ADR-023 的「依赖方数量」判据随
/// **唯一调用方**迁到本模块（ADR-023）：只有这里的流循环发增量帧。终态帧走
/// session 的 `llm_emit_state`（`plugins/session/transcript/frames.rs`），完整消息帧走 core 的
/// `llm_emit_message`——三者同一套帧语义，位置由消费方数量决定。
async fn llm_emit_delta(sink: &ExecEventSink, message_id: &str, delta: &str) {
    sink.emit(ChatNode {
        id: message_id.to_string(),
        delta: Some(delta.to_string()),
        ..Default::default()
    })
    .await;
}

/// 当前「未结束行」已经发送给前端的增量长度（按字段分别记）。
/// ## 为什么必须记账
///
/// 同一段文本有两条到达路径：**未结束的行**由协议层的增量提取器边收边吐；
/// **完整行**由 `SseLineParser::parse_line` 一次性给出全量。若不记账，完整行
/// 到达时会把已经发过的前缀**再发一遍**（前端重复）。因此完整行路径按这里的
/// 长度截断，而增量路径每发一次就累加一次。
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
/// `parser` 是**协议层**提供的行解析契约（`SseLineParser`）：它既能解析完整行，
/// 也能为「尚未结束的行」开一个增量提取器。本模块只负责按 `\n` 切分、按前缀截断
/// 去重、把事件交给 `dispatch_and_track`——**不认识任何协议字段名**。
/// 历史上这里收的是闭包 `Fn(&str) -> Vec<ModelProtocolEvent>`，增量提取则由内置的
/// 启发式解析器代劳（硬编码 `"content":"` / `"partial_json":"` 等字段名）。那套写法
/// 有三重问题：加协议要改循环、增量与完整行两套转义规则（会吃字）、每块重扫整行
/// （O(n²)）。契约拆成两个方法后，三件事一起解决。
///
/// 流式产出的子节点经 `sink` 实时下发（进程内直连转写唯一写入点）。
/// 中止只经 `abort` 感知（不再在流循环里排空控制帧）。
/// SSE 流循环：把响应字节转成单轮产物，**写入调用方给的 `out`**。
///
/// 两个入参是为**重试**准备的（调用方 [`crate::plugins::model::bound_provider`]）：
/// - `reuse_node_ids`：上一批尝试已广播过的工具调用节点 id，按 wire index 升序。
///   空 = 首次尝试。非空 = 累积器以此播种，同序调用复用原节点 id（首片增量即整条
///   快照覆盖），**不**在前端留重复卡片。
/// - `out`：由调用方持有并跨尝试保留——正文子节点 id 因此稳定，失败重试时同一节点
///   被快照覆盖而不是新建一条。产物字段（`text` / `tool_calls`…）由本函数覆盖。
///
/// 返回 [`StreamFailure`] 而非 [`String`]：调用方需要区分「瞬时（值得原样重发）」
/// 与「非瞬时（重试只是白等）」，措辞也已在此收敛（不回显原始网关 JSON）。
pub async fn parse_sse_stream(
    response: reqwest::Response,
    root_id: &str,
    sink: &ExecEventSink,
    abort: &ExecAbortSignal,
    parser: &dyn SseLineParser,
    reuse_node_ids: &[String],
    out: &mut TurnOutput,
) -> Result<(), StreamFailure> {
    let mut stream = response.bytes_stream();
    let mut buffer = Vec::<u8>::new();
    // 工具调用的分片累积器**只在本插件内活着**：它是流式状态机（过程），
    // 收口后才把结果（`Vec<TurnToolCallInfo>`）写进 `out.tool_calls` 交出去。
    let mut tool_acc = if reuse_node_ids.is_empty() {
        TurnToolCallAccumulator::default()
    } else {
        TurnToolCallAccumulator::with_node_ids(reuse_node_ids)
    };

    // ── 生命周期日志与卡死防护 ──────────────────────────────────────────
    // 让每轮 SSE 流在控制台留下完整轨迹：何时建立、首字节何时到达、首条内容
    // 何时产出、正常/异常如何结束。若卡死，可从最后一条日志精确定位阶段：
    //   有「请求发起」无「响应头」     → 卡在 POST 等待（网关/网络）；
    //                                    「请求发起」由调用方在构建请求体后打出；
    //                                    「响应头到达」在 http.rs 的 `PostResult::Ok` 前打出。
    //   有「响应头」无「首个数据块」   → 卡在 SSE 建立后无数据（provider 挂起）；
    //   有「首个数据块」无「首条内容」 → 收到字节但协议解析无内容事件（协议异常）。
    //   有「首条内容」后长时间静默     → 流中途挂死 → `STREAM_IDLE_TIMEOUT` 兜底报错。
    let started = std::time::Instant::now();
    let mut first_chunk_at: Option<std::time::Instant> = None;
    let mut first_content_logged = false;
    let mut chunk_count: u64 = 0;
    let mut total_bytes: u64 = 0;

    let mut progress = LineProgress::default();
    // 超限调用集合（见 `dispatch_and_track` 的 ToolCallDelta 分支）。
    let mut overflow_ids = std::collections::HashSet::<String>::new();

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
    // 流中途的失败**不再直接 return**：累积器必须先收口（调用方要靠 `out.tool_calls`
    // 知道这一批建过哪些节点，才能在重试时复用 id），故在此记账、循环外统一出口。
    let mut failure: Option<StreamFailure> = None;
    // 协议层 `Error` 事件回收槽（网关 JSON 错误体等），语义见 `dispatch_and_track`。
    let mut ev_error: Option<String> = None;

    loop {
        // 空闲超时包裹：流若中途挂起（连接在、数据停），最多等 STREAM_IDLE_TIMEOUT
        // 就显式报错终止，而不是静默挂到整个 1800s 超时（用户视角即「卡死」）。
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
                failure = Some(StreamFailure {
                    message: msg,
                    transient: true,
                });
                break;
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

        // 读取失败（TCP 被对端关闭 / 连接重置）即**瞬时**故障：请求级重试管不到这里，
        // 由调用方原样重发。措辞不带 reqwest 的内部错误串。
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                plugin_error!(
                    "model",
                    "[LLM] 流读取中断：{e} (已收 {} chunks / {} bytes, 耗时 {:?})",
                    chunk_count,
                    total_bytes,
                    started.elapsed()
                );
                // 读流失败**在结构上就是瞬时**（TCP 已被对端关掉/重置；成因不在
                // 请求体里），不靠关键词判定——reqwest 的措辞版本间会变。原始错误
                // 只进日志（上一行），面向用户的措辞不回显 reqwest 的内部错误串。
                failure = Some(StreamFailure {
                    message:
                        "模型连接中断：上游流在完成前终止（连接被对端关闭），请重试或切换模型。"
                            .to_string(),
                    transient: true,
                });
                break;
            }
        };
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
                    // `out` 是 `&mut` 形参（本身不可变绑定）：reborrow 而非 `&mut out`。
                    match dispatch_and_track(
                        event,
                        root_id,
                        sink,
                        out,
                        &mut tool_acc,
                        &mut first_content_logged,
                        started,
                        &mut overflow_ids,
                        &mut ev_error,
                    )
                    .await
                    {
                        Ok(()) => {}
                        Err(f) => {
                            failure = Some(f);
                            break;
                        }
                    }
                }
                if failure.is_some() {
                    break;
                }
            }
            // 换行 ⇒ 本行的增量提取作废、记账归零（下一行重新开始）
            progress = LineProgress::default();
            partial = None;
            partial_gave_up = false;
            partial_fed = 0;
        }
        // 流已中断：不再解析剩余缓冲，交收口（`out.tool_calls` 已含已建节点）。
        if failure.is_some() {
            break;
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
            let mut dispatch_failed = false;
            for ev in events.drain(..) {
                if let Some(ev) = progress.record(ev) {
                    if let Err(f) = dispatch_and_track(
                        ev,
                        root_id,
                        sink,
                        out,
                        &mut tool_acc,
                        &mut first_content_logged,
                        started,
                        &mut overflow_ids,
                        &mut ev_error,
                    )
                    .await
                    {
                        failure = Some(f);
                        dispatch_failed = true;
                        break;
                    }
                }
            }
            partial_out = events; // 归还容量
            if dispatch_failed {
                break;
            }
        }
    }

    // ── 统一出口：流中途失败 ────────────────────────────────────────────
    //
    // **必须在累积器收口之后**：失败也要把已建节点交还调用方。调用方（session）要靠
    // `out.tool_calls` 知道这一批建过哪些节点 id，才能在重试时复用同一批 id。不做这
    // 一步，瞬时失败原样重试会让同一批节点**在 UI 上出现两批**（新 id 新建而非就地
    // 覆盖），表现为用户看到的「工具全部重来一遍」。
    //
    // 协议层 `Error` 事件（网关 JSON 错误体）：交给 `classify_stream_error` 一并做
    // 「取人读原因 + 判瞬时性 + 收敛措辞」——**不回显原始 JSON**（与 `http.rs` 221 行
    // 同一条纪律：用户看到的是「连接中断，请重试」，不是一坨 `details.cause`）。
    if let Some(e) = ev_error.take() {
        failure = Some(classify_stream_error(&e));
    }

    // 收口：分片累积器按值消费，产物以**结果形态**写回单轮产物。
    // 成功与失败都要走这一步（失败路径同样要把已建节点 id 交回调用方）。
    out.tool_calls = tool_acc.finish();
    mark_overflow_dropped(&mut out.tool_calls);

    if let Some(f) = failure {
        plugin_error!(
            "model",
            "[LLM] 流中断终止 (耗时 {:?}, {} chunks / {} bytes, 已建工具调用 {} 个, transient={}): {}",
            started.elapsed(),
            chunk_count,
            total_bytes,
            out.tool_calls.len(),
            f.transient,
            f.message
        );
        return Err(f);
    }

    // ③ 流结束日志：正常结束 / 中止 / 空流，均带统计信息。
    // 若此处之后长时间无下文（工具执行/下一轮请求），可据此定位卡死发生在「流结束后」阶段。
    if abort.is_aborted() {
        // 中止已在上方记录，此处不重复。
    } else if out.text.is_empty() && out.reasoning.is_empty() && out.tool_calls.is_empty() {
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
            out.tool_calls.len(),
            out.finish
        );
    }
    // 产物已写入调用方持有的 `out`（见本函数签名）：这里只报「这一批流跑完了」。
    Ok(())
}

/// 单个协议事件的分发 + 首条内容日志。
///
/// ④「获取到第一条消息」日志：第一条文本/推理/工具调用内容事件到达时打点，
/// 标志「模型已开始实际产出」。此后若卡死，可确定卡在「产出过程中」或「产出完成后」。
#[allow(clippy::too_many_arguments)] // 参数全部是流级状态（进度记账 + 超限集合 + 错误回收），拆结构体反而隔一层
async fn dispatch_and_track(
    ev: ModelProtocolEvent,
    root_id: &str,
    sink: &ExecEventSink,
    out: &mut TurnOutput,
    tool_acc: &mut TurnToolCallAccumulator,
    first_content_logged: &mut bool,
    started: std::time::Instant,
    // 超限调用的节点 id 集合（跨事件持续——同一调用的后续参数增量靠它识别）。
    // 命中的调用不再上线 UI 节点，收口时由 `mark_overflow_dropped` 打丢弃标记。
    overflow_ids: &mut std::collections::HashSet<String>,
    // 协议层错误回收槽：`Error` 事件不再冒泡（冒泡会跳过收口），写在这里。
    // 流循环在收口后统一出口。
    ev_error: &mut Option<String>,
) -> Result<(), StreamFailure> {
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
    dispatch_protocol_event(ev, root_id, sink, out, tool_acc, overflow_ids, ev_error).await
}

async fn dispatch_protocol_event(
    ev: ModelProtocolEvent,
    root_id: &str,
    sink: &ExecEventSink,
    out: &mut TurnOutput,
    tool_acc: &mut TurnToolCallAccumulator,
    // 超限调用集合（语义见 `dispatch_and_track` 的参数说明）
    overflow_ids: &mut std::collections::HashSet<String>,
    ev_error: &mut Option<String>,
) -> Result<(), StreamFailure> {
    match ev {
        ModelProtocolEvent::ContentDelta(c) => {
            // Filter out truly empty content, but preserve newlines for markdown formatting
            if c.is_empty() {
                return Ok(());
            }
            out.text.push_str(&c);
            // 首个增量：发**完整快照**创建正文子节点（id 此后保持一致）。
            // 后续增量：显式 Append 窄帧，消费循环按 id 追加——不重发整条，
            // 也不需要消费端从「补丁形状」里猜这是追加还是替换。
            if out.response_text_child_id.is_empty() {
                out.response_text_child_id = llm_short_id();
                llm_emit_message(
                    sink,
                    ChatNode {
                        id: out.response_text_child_id.clone(),
                        parent_id: Some(root_id.into()),
                        role: Some(NodeRole::Assistant),
                        msg_type: Some(NodeKind::Body),
                        content: Some(MessageContent::Text(out.text.clone())),
                        status: Some(NodeStatus::Streaming),
                        ..Default::default()
                    },
                )
                .await;
            } else {
                llm_emit_delta(sink, &out.response_text_child_id, &c).await;
            }
        }
        ModelProtocolEvent::ReasoningDelta(r) => {
            // Filter out truly empty content, but preserve newlines for markdown formatting
            if r.is_empty() {
                return Ok(());
            }
            out.reasoning.push_str(&r);
            if out.reasoning_child_id.is_empty() {
                out.reasoning_child_id = llm_short_id();
                llm_emit_message(
                    sink,
                    ChatNode {
                        id: out.reasoning_child_id.clone(),
                        parent_id: Some(root_id.into()),
                        role: Some(NodeRole::Assistant),
                        msg_type: Some(NodeKind::Notice),
                        title: Some("思考过程".to_string()),
                        content: Some(MessageContent::Text(out.reasoning.clone())),
                        status: Some(NodeStatus::Streaming),
                        ..Default::default()
                    }
                    .with_style_tag(CHAT_STYLE_TAG_REASONING),
                )
                .await;
            } else {
                llm_emit_delta(sink, &out.reasoning_child_id, &r).await;
            }
        }
        ModelProtocolEvent::ToolCallDelta(idx, id, name, args) => {
            let (tc_id, wire_id, full_args, full_name, snapshot_required) =
                tool_acc.process_delta(idx, id.as_deref(), name.as_deref(), args.as_deref());

            // ── 单响应工具调用数上限（`6cf9520d` 号会话的死循环守卫）──────────
            //
            // 那次会话模型在**一次响应**内流式输出了 56 个工具调用（同一次复读里 9 个
            // 调用原样重复 6 遍，`tool_call_id` 全部唯一），批执行器要等流结束才
            // 开工 → 60 余秒零结果，用户视角即「工具永远不闭合」，只能手动终止。
            // 空闲超时防不了它——复读也是字节，`STREAM_IDLE_TIMEOUT` 永远不触发。
            //
            // 处置（反馈语义，见常量文档）：第 33 个调用诞生起记入 `overflow_ids`，
            // **不再上线 UI 节点**（防刷屏），但照常入账——流结束后由
            // `mark_overflow_dropped` 打丢弃标记，执行层据此落「超限丢弃」结果，
            // 模型下一轮从工具结果里读到发生了什么。上限取 32：本仓健康的单批是
            // 个位数～十几个，32 已是两倍余量；真需要更多调用应该分轮发
            // （每轮拿到结果再继续）。
            let is_overflow =
                if snapshot_required && tool_acc.call_count() > MAX_TOOL_CALLS_PER_RESPONSE {
                    overflow_ids.insert(tc_id.clone());
                    plugin_warn!(
                        "model",
                        "[LLM] 单响应内工具调用数超过上限（{MAX_TOOL_CALLS_PER_RESPONSE}），\
                     自本调用起丢弃（不执行、不显示），执行层将向模型反馈超限原因"
                    );
                    true
                } else {
                    overflow_ids.contains(&tc_id)
                };
            if is_overflow {
                return Ok(());
            }

            // 工具调用骨架节点（`Skeleton` + tag `tool`）：自身 content 即**请求参数**（不设独立的请求子节点，
            // 见 `symbio/src/plugins/session/docs/018-node-state-streaming.md` §2.2）。两条路径与正文 / 推理
            // **同构**——帧面只有两种语义，由覆盖方式决定，接收端不做类型推断：
            // - 身份字段变化（新建节点 / 首次定名）→ 完整快照（`Upsert`）；
            // - 纯参数增量 → 窄追加（`Append`，O(delta)），接收端尾部拼接。
            if snapshot_required {
                llm_emit_message(
                    sink,
                    ChatNode {
                        id: tc_id.clone(),
                        parent_id: Some(root_id.into()),
                        role: Some(NodeRole::Assistant),
                        msg_type: Some(NodeKind::Skeleton),
                        content: Some(MessageContent::Text(full_args)),
                        status: Some(NodeStatus::Streaming),
                        tool_call_id: Some(wire_id),
                        ..Default::default()
                    }
                    .with_style_tag(CHAT_STYLE_TAG_TOOL)
                    .with_tool_name(full_name),
                )
                .await;
            } else if let Some(delta) = args.as_deref().filter(|d| !d.is_empty()) {
                llm_emit_delta(sink, &tc_id, delta).await;
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
        // 协议层给出的错误（网关 JSON 错误体等）**不再冒泡**：本函数由流循环用
        // `?` 上调，冒泡会跳过累积器收口，调用方因而拿不到本批建过的节点 id
        // → 重试时无法复用 id（前端会出现「一次请求两套卡片」）。改为记进出参，
        // 由流循环在收口后统一出口。
        ModelProtocolEvent::Error(e) => {
            *ev_error = Some(e);
        }
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

// ── panic 面登记（PN-001…003）─────────────────────────────────────────
// 本文件每一处 `unwrap` / `expect` / `panic!` / `unreachable!` 的理由。登记放在
// 文件内而不是集中一张表：理由与它解释的那段代码会一起被 review、一起被删。
// 判据是 `scripts/panic-audit.mjs`：**加一处 panic 必须同时加一行登记，理由非空**。
// panic-allow symbio/src/plugins/model/stream.rs::parse_sse_stream: 前序步骤已记下该值，取不到说明本函数的前置条件被破坏，属代码缺陷。
#[cfg(test)]
#[path = "stream.test.rs"]
mod tests;
