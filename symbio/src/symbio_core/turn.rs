//! 单轮 LLM 执行机器
//!
//! 职责（单轮网关基建，协议无关、插件无关，供 `ModelProvider::execute_turn`
//! 统一实现与 model/session 双侧共同使用）：
//! - HTTP 客户端单例 + 支持中止的 POST 重试机器（`execute_post_with_abort` → 五态 `PostResult`）
//! - SSE 流解析与协议事件累积（`parse_sse_stream` → `TurnOutput`），流式子节点经
//!   [`EventSink`] 实时下发
//! - 工具调用增量累积（`ToolCallAccumulator`）
//! - 消息构造家族（`short_id`/`StreamChildIds`/`build_assistant_messages`/`build_tool_message`）：
//!   `TurnOutput::into_messages` 与 `ToolCallAccumulator` 直接依赖它，
//!   孤儿规则要求定义与使用同处 core
//!
//! ## 执行期只与两个原语打交道（不再与通道打交道）
//!
//! 本模块的所有函数只依赖 [`EventSink`]（出：节点事件）与 [`AbortSignal`]
//! （入：中止），**不再接受 `PluginChannel`**。历史上两者都压在同一个通道上，
//! 进程内调用因此要付 serde 装箱 + 反序列化的往返代价；现在「去哪」与
//! 「怎么中止」分别由两个语义单一的原语承担，`PluginChannel` 退回纯跨进程传输
//! （见 `symbio_core::exec` 的模块文档）。

use crate::symbio_core::exec::{AbortSignal, EventSink};
use crate::symbio_core::model_provider::{FinishReason, ProtocolEvent, Usage};
use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
};
use crate::symbio_core::sse::{utf8_chunk, PartialLineExtractor, SseLineParser};
use crate::{plugin_error, plugin_info, plugin_warn};
use futures::StreamExt;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;
use tracing::warn;

// HTTP 客户端（单例）

/// SSE 流空闲超时：两次数据块之间的最大间隔。
///
/// 整体 1800s 超时覆盖不了 provider 半途挂起（连接不断、但不发任何字节）：
/// 此类请求会静默挂满 30 分钟，期间无任何日志，表现为「Turn 开始后卡死」。
/// 180s 无任何字节即判定流已死，显式报错终止，让上层走 Failed 收尾而非无限等待。
pub const STREAM_IDLE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

pub fn get_http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .pool_max_idle_per_host(10)
            .tcp_keepalive(Some(std::time::Duration::from_secs(60)))
            .connect_timeout(std::time::Duration::from_secs(10))
            // 读空闲超时：与 STREAM_IDLE_TIMEOUT 双保险（连接层 + 应用层）。
            .read_timeout(STREAM_IDLE_TIMEOUT)
            .timeout(std::time::Duration::from_secs(1800)) // 整体流式请求超时
            .build()
            .expect("Failed to build shared reqwest Client")
    })
}

// 执行期出口与中止（唯一两个原语）

/// 发送一条**完整消息**（`content` = 整条替换，幂等）。
///
/// 用在正文对接收端是**新的权威副本**的帧上：一次性节点（工具结果 / 用户消息
/// 回填）的单帧完成、存储回执、压缩快照。流式节点的正文已由 [`emit_delta`]
/// 逐帧传过，它的终态走 [`emit_converge`]，不在这里重发。
pub async fn emit_message(sink: &EventSink, msg: ChatMessage) {
    sink.emit(message_frame(&msg)).await;
}

/// 由一条完整消息派生**消息帧**（`content` = 整条替换）：缺省补 `completed`。
///
/// 直接写转写（`Transcript::apply`，不经出口）的发布路径也用它——「完整消息必然
/// 带状态」这条约定只在这里实现一次。
pub fn message_frame(m: &ChatMessage) -> ChatMessage {
    let mut frame = m.clone();
    if frame.status.is_none() {
        frame.status = Some(MessageStatus::Completed);
    }
    frame
}

/// 发送一帧**增量**：`delta` 追加到目标节点正文尾部（流式热路径，O(delta)）。
///
/// 目标未知时写入点用帧内信息建占位（帧自给自足，不依赖任何先行帧）。
pub async fn emit_delta(sink: &EventSink, message_id: &str, delta: &str) {
    sink.emit(ChatMessage {
        id: message_id.to_string(),
        delta: Some(delta.to_string()),
        ..Default::default()
    })
    .await;
}

/// 发送一帧**状态**：身份 + 状态 + 元数据 + 错误，**不带正文**。
///
/// 流式节点的正文已由 [`emit_delta`] 逐帧上线，这里再带一次只是把同一段文字
/// 二次传输（且会把权威副本的完整正文重新发一遍）。`content` / `delta` 一律
/// 剥掉，免得调用方传了一条「内容齐全的副本」就顺手把它送上热路径。
pub async fn emit_state(sink: &EventSink, msg: ChatMessage) {
    sink.emit(state_frame(&msg)).await;
}

/// 发送一帧**删除**：`status = removed`。
///
/// 协议里没有 `remove` 操作——删除就是一次状态迁移，与出现、增长、完成同走
/// 一条消息帧，接收端据此就地移除节点。
pub async fn emit_removed(sink: &EventSink, message_id: &str) {
    sink.emit(removed_frame(message_id)).await;
}

/// 由一条完整消息派生**状态帧**：身份 + 状态 + 元数据 + 错误，**不带正文**。
///
/// 直接写转写（`Transcript::apply`，不经出口）的收口路径也用它——那些节点的
/// 正文早已由 `delta` 逐帧上线，重发一遍只是把同一段文字二次传输。
pub fn state_frame(m: &ChatMessage) -> ChatMessage {
    let mut frame = m.clone();
    frame.content = None;
    frame.delta = None;
    if frame.status.is_none() {
        frame.status = Some(MessageStatus::Completed);
    }
    frame
}

/// 删除帧：`status = removed`（发射方与收口路径的唯一构造点，避免各写一份）。
pub fn removed_frame(message_id: &str) -> ChatMessage {
    ChatMessage {
        id: message_id.to_string(),
        status: Some(MessageStatus::Removed),
        ..Default::default()
    }
}

/// 等到中止（`abort` 已置位则立即返回）。
///
/// 收口前这里是一个 `select!` 三臂：100ms 轮询标志位 | 通道取消 | 收 Abort 帧。
/// 现在只剩一条——[`AbortSignal::abort`] 置位的同时就唤醒等待者，**无需轮询**；
/// 而「通道关闭 ⇒ 中止」的语义改由发起方在退出时显式调用 `abort()` 承担
/// （隐式的 sender drop 换成一次命名调用，行为不变、意图更清楚）。
async fn wait_for_abort_signal(abort: &AbortSignal) {
    abort.cancelled().await;
}

// HTTP 请求（支持自动重试）

pub enum PostResult {
    Ok(reqwest::Response),
    RetryWithoutContextId,
    Err(String),
    /// 命中限流/服务端过载且重试耗尽：携带面向用户的可读提示（不回显原始 API JSON）。
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
async fn sleep_with_abort(d: std::time::Duration, abort: &AbortSignal) {
    tokio::select! {
        _ = tokio::time::sleep(d) => {}
        _ = abort.cancelled() => {}
    }
}

/// 日志用的端点标签：`host/path`（不含查询串）。
///
/// 只取「主机 + 路径」——排查时关心的是**打到了哪个端点**；完整 URL 会带上无意义的
/// 查询串，也可能包含 token 类参数（不该进日志）。解析失败时原样返回，绝不吞掉信息。
pub fn endpoint_label(url: &str) -> String {
    reqwest::Url::parse(url)
        .ok()
        .map(|u| format!("{}{}", u.host_str().unwrap_or("?"), u.path()))
        .unwrap_or_else(|| url.to_string())
}

/// 发出一次（可重试的）LLM 请求。
///
/// ## `body` 是**已序列化的字节**，不是 `Value`
///
/// 收 `&[u8]` 而不是 `&Value`，是为了让「一次序列化」这件事**在结构上成立**：
/// `.json(value)` 会在 reqwest 内部 `serde_json::to_vec` 一遍，而它位于**重试循环内**
/// （最多 `1 + MAX_RETRIES` 次发送）——同一份请求体被反复序列化，重试越多浪费越多，
/// 而重试恰恰发生在网络/服务端已经不健康的时候。
///
/// 现在由调用方序列化一次（它同时要用那份字节量出日志里的「体量」），
/// 重试只复用同一份 buffer（`to_vec()` 是一次 memcpy，比序列化便宜两个数量级）。
pub async fn execute_post_with_abort(
    url: &str,
    headers: reqwest::header::HeaderMap,
    body: &[u8],
    abort: &AbortSignal,
) -> PostResult {
    // 限流/瞬时 5xx/网络抖动：有界重试 + 指数退避，避免一次瞬时错误就中断整轮对话。
    // 重试在同一 turn 内进行（复用同一个 root_id），不会额外产生 Turn/文本节点，
    // 因此不会造成"错误刷屏"。重试耗尽才向上返回错误，由上层停止并展示重试入口。
    const MAX_RETRIES: u32 = 4;
    // Content-Type 的**真源是各协议适配器的 `get_headers`**（四个适配器都设了
    // `application/json`）。`.body(bytes)` 不像 `.json()` 那样会自己补，所以这里
    // 兜一次底——只为防后来的适配器漏掉，当前四条路径都不会走到。
    let mut headers = headers;
    if !headers.contains_key(reqwest::header::CONTENT_TYPE) {
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/json"),
        );
    }
    let started = std::time::Instant::now();
    // 「请求发起」锚点不在这里：它由调用方（`model::bound_provider::execute_turn`）
    // 在**请求体构建之后**统一打一条，便于在同一条里同时给出模型/规模/端点/体量。
    // 这里的进程起点紧随其后，仍在「等待响应头」之前，故阶段定位语义不变
    // （见下方 `parse_sse_stream` 的阶段对照表）。
    let mut attempt: u32 = 0;

    loop {
        if abort.is_aborted() {
            return PostResult::Aborted;
        }

        let result = tokio::select! {
            res = get_http_client().post(url).headers(headers.clone()).body(body.to_vec()).send() => {
                res
            },
            _ = wait_for_abort_signal(abort) => {
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
                    sleep_with_abort(delay, abort).await;
                    continue;
                }
                return PostResult::Err(format!("网络传输失败: {e}"));
            }
            Ok(r) => r,
        };

        if abort.is_aborted() {
            plugin_info!(
                "model",
                "[LLM] 请求在等待响应阶段被中止 (耗时 {:?})",
                started.elapsed()
            );
        }

        if response.status().is_success() {
            // ② 响应头到达日志：此后卡死则卡在「流已建立、SSE 无数据」阶段。
            plugin_info!(
                "model",
                "[LLM] 响应头到达 HTTP {} (等待 {:?}, attempt {})",
                response.status(),
                started.elapsed(),
                attempt + 1
            );
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
            sleep_with_abort(delay, abort).await;
            continue;
        }

        // 不可重试，或重试耗尽：返回面向用户的友好提示（不回显原始 API JSON）。
        if status.as_u16() == 429 {
            plugin_error!(
                "model",
                "[LLM] 请求最终失败：429 限流 (总耗时 {:?}, 重试 {} 次)",
                started.elapsed(),
                attempt
            );
            return PostResult::RateLimited(
                "请求过于频繁（429 限流）。请稍后重试，或切换其他模型继续。".to_string(),
            );
        }
        if status.as_u16() >= 500 {
            plugin_error!(
                "model",
                "[LLM] 请求最终失败：HTTP {status} (总耗时 {:?}, 重试 {} 次)",
                started.elapsed(),
                attempt
            );
            return PostResult::Err(format!("模型服务暂时不可用（HTTP {status}），请稍后重试。"));
        }
        plugin_error!(
            "model",
            "[LLM] 请求最终失败：HTTP {status} (总耗时 {:?})",
            started.elapsed()
        );
        return PostResult::Err(format!("API Error ({status}): {err_text}"));
    }
}

// 工具调用增量累积

#[derive(Debug, Default, Clone)]
struct AccumulatedToolCall {
    /// provider 原始 `tool_call_id`（wire id）。供应商未返回（或全空白）时为 `None`，
    /// 此时请求构建回退用节点 id。
    id: Option<String>,
    /// 消息节点 id：**首个增量到达时分配**，会话内唯一。
    ///
    /// 许多 OpenAI 兼容网关**跨轮复用** `call_0` / `call_xxx` 这类短 id；若直接把
    /// wire id 当节点 id，第二轮的同 id 工具调用会更新到第一轮的老节点（后端
    /// `Vec` 存储不撞、前端按 id 的 map 撞——"后端正常、前端显示混乱"的根源）。
    node_id: String,
    name: Option<String>,
    arguments: String,
}

/// Tool call information
#[derive(Debug, Clone)]
pub struct ToolCallInfo {
    /// 消息节点 id（会话内唯一）：流式帧 / 落库 / 工具结果锚定都用它。
    pub id: Option<String>,
    /// provider 原始 `tool_call_id`（wire id）。`None` = 供应商未提供，
    /// 请求构建回退节点 id（历史上节点 id 就是 wire id，旧数据天然成立）。
    pub wire_id: Option<String>,
    pub name: Option<String>,
    pub arguments: Value,
    /// 参数 JSON **非空且解析失败**时的原始文本；其余情况为 `None`。
    ///
    /// 存在理由：`get_completed` 早先对解析失败静默回退 `{}`，于是「参数被
    /// max_tokens 截断、参数残破」与「无参工具的空参数」在下游长得一模一样——
    /// 工具收到空参后报「缺少必填参数」，模型误以为调用合法而原样重试，
    /// 形成卡思考死循环。此字段把「解析失败」这一事实显式携带到执行侧，
    /// 由 `process_tool_calls_async` 拒绝执行并回报明确错误。
    ///
    /// `None` 且 `arguments == {}` 是合法的：无参工具的空串/纯空白参数。
    pub parse_error: Option<String>,
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
    /// Process a tool call delta from the API.
    ///
    /// 返回 `(node_id, wire_id, accumulated_args, name, snapshot_required)`：
    /// - `node_id`：消息节点 id（**首个增量分配，会话内唯一**）——流式帧与落库都用它；
    /// - `wire_id`：provider 的原始 tool_call_id（未提供时等于 `node_id`）——
    ///   仅在构建 LLM 请求包时使用；
    /// - `accumulated_args`：**迄今累积**的参数 JSON；
    /// - `name`：工具名（空串增量不覆盖已定名）；
    /// - `snapshot_required`：本次增量是否改动了节点的**身份字段**（新建节点 / 首次定名）。
    ///   是 → 调用方必须发**完整快照**（`Upsert`，身份与内容一次给全）；
    ///   否 → 只是正文增长，调用方发**窄追加**（`Append`，O(delta)）。
    ///
    /// 这条划分与 Text / Reasoning 子节点**同构**：帧面只有两种语义——「整条替换」与
    /// 「尾部追加」——由覆盖方式决定，而不是由接收端去猜。
    pub fn process_delta(
        &mut self,
        index: usize,
        id: Option<&str>,
        name: Option<&str>,
        args_delta: Option<&str>,
    ) -> (String, String, String, Option<String>, bool) {
        let entry = self.calls.entry(index).or_default();

        // 节点 id 在诞生时确定并写入 entry：流式广播、落库（build_assistant_messages）、
        // 执行（process_tool_calls_async）三处使用同一节点 id。
        let is_new_node = entry.node_id.is_empty();
        if is_new_node {
            entry.node_id = short_id();
        }
        // 新建节点必须发快照（接收端尚无此节点）；首次定名亦然——身份字段
        // （name / tool_call_id / 父子）只随快照下发，之后的增量只带参数片段。
        let mut snapshot_required = is_new_node;

        // 仅接受非空 id/name：
        // 部分 OpenAI 兼容网关（如实测 apinex qwen-3.8-max）只在首个增量携带合法 id，
        // 后续增量重复发送 `id:""`。若用空值覆盖，会把首个增量的合法 id 冲掉，
        // 最终得到 Some("") → 工具调用被误判为 id 缺失而被跳过。
        if let Some(id) = id.filter(|s| !s.trim().is_empty()) {
            entry.id = Some(id.to_string());
        }
        if let Some(name) = name.filter(|s| !s.trim().is_empty()) {
            if entry.name.is_none() {
                snapshot_required = true;
            }
            entry.name = Some(name.to_string());
        }

        let node_id = entry.node_id.clone();
        // 供应商始终未返回 id（缺失或全为空串）时，wire id 回退为节点 id——
        // 请求包里的 tool_call 与 tool 结果引用同一节点 id，依然自洽。
        let wire_id = entry
            .id
            .clone()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| node_id.clone());

        if let Some(delta) = args_delta {
            entry.arguments.push_str(delta);
        }

        (
            node_id,
            wire_id,
            entry.arguments.clone(),
            entry.name.clone(),
            snapshot_required,
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
    /// 保证返回的每个 ToolCallInfo.id（节点 id）均为非空：正常情况下 process_delta
    /// 已在首个增量确定，此处为幂等兜底——重复调用返回相同 id，**绝不**重新随机生成
    /// （chat_loop 与 into_messages 会各取一次，两次结果不一致会使工具结果子节点变孤儿）。
    pub fn get_completed(&mut self) -> Vec<ToolCallInfo> {
        self.calls
            .values_mut()
            .map(|call| {
                if call.node_id.is_empty() {
                    call.node_id = short_id();
                }
                let node_id = call.node_id.clone();
                // wire id：供应商提供了合法 id 才携带；否则 None（请求构建回退节点 id）
                let wire_id = call.id.clone().filter(|s| !s.trim().is_empty());
                // 参数解析：区分三种情况，**绝不**把解析失败伪装成空参数。
                //  - 空串/纯空白：无参工具的合法形态（`from_str("")` 必失败），视为 `{}`；
                //  - 合法 JSON：照常使用；
                //  - 非空且非法：参数已残破（典型为 max_tokens 截断），保留原文交给
                //    parse_error，由执行侧拒绝执行——静默 `{}` 会让工具报「缺少必填
                //    参数」，模型看不懂原因便原样重试，卡死在思考循环里。
                let raw = call.arguments.as_str();
                let (args, parse_error) = if raw.trim().is_empty() {
                    (serde_json::json!({}), None)
                } else {
                    match serde_json::from_str::<Value>(raw) {
                        Ok(v) => (v, None),
                        Err(e) => {
                            warn!(
                                error = %e,
                                raw_arguments = %raw,
                                "tool call arguments JSON invalid — refusing to execute"
                            );
                            (serde_json::json!({}), Some(call.arguments.clone()))
                        }
                    }
                };
                ToolCallInfo {
                    id: Some(node_id),
                    wire_id,
                    name: call.name.clone(),
                    arguments: args,
                    parse_error,
                }
            })
            .collect()
    }
}

// 消息构造（ChatMessage 家族）

/// 生成长度短的 ID（8 字符，取 UUID v4 前缀）
pub fn short_id() -> String {
    uuid::Uuid::new_v4().to_string()[..8].to_string()
}

/// 流式期间已经广播给前端的子节点 id。
///
/// **落库时必须复用这些 id**：流式层（`parse_sse_stream` 的 `emit_update`）
/// 与存储层（`build_assistant_messages`）是同一批节点的两个视图。若两层各自
/// `short_id()` 生成新 id，同一个文本子节点在「前端流式快照」里是 id=A、
/// 在「会话存储」里是 id=B，会被上层判定为两条不同消息——于是失败收尾时
/// id=A 的节点被当作"尚未落库的流式半截"补写进存储，同一个 Turn 下出现两份内容相同的文本节点。
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
    let timestamp = crate::symbio_core::now_ms();

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
    // ToolCall 组合节点自身携带请求参数（content = JSON 文本），不设独立的请求子节点。
    // `id` 是节点 id（与流式帧一致），provider 的 wire id 存 `tool_call_id`。
    for tc in tool_calls {
        let tc_id = tc.id.clone().unwrap_or_else(short_id);
        // 解析失败时落库**残破原文**而非占位 `{}`：存储层保真，事后能看出模型
        // 究竟发了什么（截断在哪一字符），而不是留下一个看似合法的假空参数。
        let args_text = match &tc.parse_error {
            Some(raw) => raw.clone(),
            None => tc.arguments.to_string(),
        };
        msgs.push(ChatMessage {
            id: tc_id.clone(),
            parent_id: Some(id.to_string()),
            role: Some(MessageRole::Assistant),
            msg_type: Some(MessageType::ToolCall),
            name: tc.name.clone(),
            content: Some(MessageContent::Text(args_text)),
            status: Some(MessageStatus::Completed),
            timestamp: Some(timestamp),
            tool_call_id: tc.wire_id.clone(),
            ..Default::default()
        });
    }

    msgs
}

/// 构造工具执行结果消息（role: Tool，msg_type: Text，parent_id 指向 tool_call）。
/// 响应结果作为 `ToolCall` 的直接 `Text`(`Tool`) 子节点（组合节点可选，故不包 Turn）。
///
/// **重要**：结果子节点的 `status` 必须与实际执行结果一致——
/// 成功 `Completed`、失败 `Failed`。若失败结果被标成 `Completed`，
/// 当下一轮 `get_context_messages` 过滤掉 `Failed`
/// 的 `ToolCall` 父节点时，这个"孤儿"`role=Tool` 结果子节点（其 `tool_call_id`
/// 指向已被删除的 tool_call）会被保留下来，使新一轮 LLM 请求携带非法
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
        timestamp: Some(crate::symbio_core::now_ms()),
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
    /// 流结束原因（一次响应最多一次）。用于区分「自然结束」与「max_tokens 截断」
    /// （不区分即表现为「对话突然结束」）。默认 Stop。
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

/// 未结束的行超过这个长度才尝试增量提取。
///
/// 太短时开提取器没有意义（下一块多半就把整行补齐了），反而多付一次字段扫描。
const PARTIAL_LINE_MIN_BYTES: usize = 256;

/// 当前「未结束行」已经发送给前端的增量长度（按字段分别记）。
///
/// ## 为什么必须记账
///
/// 同一段文本有两条到达路径：**未结束的行**由协议层的增量提取器边收边吐，
/// **完整行**由 [`SseLineParser::parse_line`] 一次性给出全量。若不记账，完整行
/// 到达时会把已经发过的前缀**再发一遍**（前端重复）。因此完整行路径按这里的
/// 长度截断，而增量路径每发一次就累加一次。
///
/// 这套机制成立的前提是：增量路径吐出的文本**恰好是**完整行文本的前缀。
/// 该不变量由协议层的提取器保证（见 [`crate::symbio_core::sse`]）——它必须
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
    fn record(&mut self, ev: ProtocolEvent) -> Option<ProtocolEvent> {
        match ev {
            ProtocolEvent::ContentDelta(c) => {
                self.content += c.len();
                Some(ProtocolEvent::ContentDelta(c))
            }
            ProtocolEvent::ReasoningDelta(r) => {
                self.reasoning += r.len();
                Some(ProtocolEvent::ReasoningDelta(r))
            }
            ProtocolEvent::ToolCallDelta(idx, id, name, Some(a)) => {
                *self.tool_args.entry(idx).or_insert(0) += a.len();
                Some(ProtocolEvent::ToolCallDelta(idx, id, name, Some(a)))
            }
            _ => None,
        }
    }
}

/// 解析 SSE 字节流为标准化事件并累积成单轮产物。
///
/// `parser` 是**协议层**提供的行解析契约（[`SseLineParser`]）：它既能解析完整行，
/// 也能为「尚未结束的行」开一个增量提取器。core 只负责按 `\n` 切分、按前缀截断
/// 去重、把事件交给 `dispatch_and_track`——**不认识任何协议字段名**。
///
/// 历史上这里收的是闭包 `Fn(&str) -> Vec<ProtocolEvent>`，增量提取则由 core 内置的
/// 启发式解析器代劳（硬编码 `"content":"` / `"partial_json":"` 等字段名）。那套写法
/// 有三重问题：加协议要改 core、增量与完整行两套转义规则（会吃字）、每块重扫整行
/// （O(n²)）。契约拆成两个方法后，三件事一起解决。
///
/// 流式产出的子节点经 `sink` 实时下发（进程内直连转写唯一写入点）；
/// 中止只经 `abort` 感知（不再在流循环里排空控制帧）。
pub async fn parse_sse_stream(
    response: reqwest::Response,
    root_id: &str,
    sink: &EventSink,
    abort: &AbortSignal,
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
    //                                    「响应头到达」在下方 `PostResult::Ok` 前打出。
    //   有「响应头」无「首个数据块」   → 卡在 SSE 建立后无数据（provider 挂起）；
    //   有「首个数据块」无「首条内容」 → 收到字节但协议解析无内容事件（协议异常）；
    //   有「首条内容」后长时间静默     → 流中途挂起 → 由 STREAM_IDLE_TIMEOUT 兜底报错。
    let started = std::time::Instant::now();
    let mut first_chunk_at: Option<std::time::Instant> = None;
    let mut first_content_logged = false;
    let mut chunk_count: u64 = 0;
    let mut total_bytes: u64 = 0;

    let mut progress = LineProgress::default();

    // 当前「未结束的行」的增量提取状态（见 `symbio_core::sse`）。
    // - `partial`：协议层给的提取器；`None` = 本行没有（尚未开 / 已放弃）
    // - `partial_gave_up`：本行已问过协议、答案是「不做」——不再重试，
    //   否则每一块都要把整行重扫一遍，正是收口前那个 O(n²)
    // - `partial_fed`：已喂给提取器的字节位置（只喂新增部分）
    let mut partial: Option<Box<dyn PartialLineExtractor>> = None;
    let mut partial_gave_up = false;
    let mut partial_fed = 0usize;
    // 增量提取器产出事件的暂存区（跨块复用，见 ② 处说明）
    let mut partial_out: Vec<ProtocolEvent> = Vec::new();

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
    ev: ProtocolEvent,
    root_id: &str,
    sink: &EventSink,
    out: &mut TurnOutput,
    first_content_logged: &mut bool,
    started: std::time::Instant,
) -> Result<(), String> {
    if !*first_content_logged {
        let kind = match &ev {
            ProtocolEvent::ContentDelta(_) => Some("文本"),
            ProtocolEvent::ReasoningDelta(_) => Some("推理"),
            ProtocolEvent::ToolCallDelta(..) => Some("工具调用"),
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
    ev: ProtocolEvent,
    root_id: &str,
    sink: &EventSink,
    out: &mut TurnOutput,
) -> Result<(), String> {
    match ev {
        ProtocolEvent::ContentDelta(c) => {
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
        ProtocolEvent::ReasoningDelta(r) => {
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
        ProtocolEvent::ToolCallDelta(idx, id, name, args) => {
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
#[path = "turn.test.rs"]
mod tests;
