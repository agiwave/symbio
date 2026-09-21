//! 单轮 LLM 执行机器
//!
//! 职责（单轮网关基建，协议无关、插件无关，供 `ModelProvider::execute_turn`
//! 统一实现与 model/session 双侧共同使用）：
//! - HTTP 客户端单例 + 支持中止的 POST 重试机器（`execute_post_with_abort` → 五态 `PostResult`）
//! - SSE 流解析与协议事件累积（`parse_sse_stream` → `TurnOutput`），流式子节点经
//!   `session_chat_response::NodeOp::Upsert` 帧实时下发
//! - 工具调用增量累积（`ToolCallAccumulator`）
//! - 消息构造家族（`short_id`/`StreamChildIds`/`build_assistant_messages`/`build_tool_message`）：
//!   `TurnOutput::into_messages` 与 `ToolCallAccumulator` 直接依赖它，
//!   孤儿规则要求定义与使用同处 core

use crate::symbio_core::model_provider::{FinishReason, ProtocolEvent, Usage};
use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
};
use crate::symbio_core::schemas::session::session_chat_response::{ControlSignal, NodeOp};
use crate::symbio_core::{PluginChannel, PluginFrame};
use crate::{plugin_error, plugin_info, plugin_warn};
use futures::StreamExt;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
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

// 通道辅助

/// 统一发送消息更新帧到消费循环（`Upsert` = 完整消息快照，接收端按 id 整条替换）。
pub async fn emit_update(channel: &PluginChannel, msg: ChatMessage) {
    let _ = channel
        .tx
        .send(PluginFrame::Data(
            serde_json::to_value(NodeOp::Upsert {
                message: Box::new(msg),
            })
            .unwrap_or_default(),
        ))
        .await;
}

/// 发送流式追加帧：`delta` 追加到已存在消息的 Text 内容尾部。
///
/// 窄载荷（O(delta)）——正文流式每帧都走这里，整条重发会是 O(n²)。
/// 目标消息必须已由 [`emit_update`] 创建；对未知 id 追加是协议违例，
/// 消费循环会报错丢弃，而不是静默造一个幽灵节点。状态迁移不走这里：
/// 那是完整快照（[`emit_update`]）的职责，帧面只有一种含义。
pub async fn emit_append(channel: &PluginChannel, message_id: &str, delta: &str) {
    let _ = channel
        .tx
        .send(PluginFrame::Data(
            serde_json::to_value(NodeOp::Append {
                message_id: message_id.to_string(),
                delta: delta.to_string(),
            })
            .unwrap_or_default(),
        ))
        .await;
}

// 控制信号处理

/// 处理单个控制帧。返回 `true` 表示收到中断信号（同时置位 `abort_flag`，
/// 让不经过本函数返回值的轮询路径也能感知中断）。
fn handle_signal_frame(frame: PluginFrame, abort_flag: &AtomicBool) -> bool {
    let is_abort = match frame {
        PluginFrame::Data(ref m) => matches!(
            serde_json::from_value::<ControlSignal>(m.clone()),
            Ok(ControlSignal::Abort)
        ),
        _ => false,
    };
    if is_abort {
        abort_flag.store(true, Ordering::SeqCst);
    }
    is_abort
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
            // 第三条中止感知路径：通道被强制取消（消费循环超时兜底 / 会话销毁）。
            // 缺失此分支时，POST 等待会无视 cancel_token 继续挂起。
            _ = channel.cancel_token.cancelled() => {
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
    let started = std::time::Instant::now();
    let body_len = serde_json::to_vec(body).map(|v| v.len()).unwrap_or(0);
    let host = reqwest::Url::parse(url)
        .ok()
        .map(|u| format!("{}{}", u.host_str().unwrap_or("?"), u.path()))
        .unwrap_or_else(|| url.to_string());
    // ① 请求发起日志：此后若卡死，可确定卡在「已发出 POST、未收到响应头」阶段。
    plugin_info!(
        "model",
        "[LLM] 请求发起 POST {} (body {} bytes)",
        host,
        body_len
    );
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

        if abort_flag.load(Ordering::SeqCst) {
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
            sleep_with_abort(delay, abort_flag).await;
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

    // ── 生命周期日志与卡死防护 ──────────────────────────────────────────
    // 让每轮 SSE 流在控制台留下完整轨迹：何时建立、首字节何时到达、首条内容
    // 何时产出、正常/异常如何结束。若卡死，可从最后一条日志精确定位阶段：
    //   有「请求发起」无「响应头」     → 卡在 POST 等待（网关/网络）；
    //   有「响应头」无「首个数据块」   → 卡在 SSE 建立后无数据（provider 挂起）；
    //   有「首个数据块」无「首条内容」 → 收到字节但协议解析无内容事件（协议异常）；
    //   有「首条内容」后长时间静默     → 流中途挂起 → 由 STREAM_IDLE_TIMEOUT 兜底报错。
    let started = std::time::Instant::now();
    let mut first_chunk_at: Option<std::time::Instant> = None;
    let mut first_content_logged = false;
    let mut chunk_count: u64 = 0;
    let mut total_bytes: u64 = 0;

    // 用于追踪当前行（正在积攒中）已经发送给前端的增量长度，防止重复发送
    #[derive(Default)]
    struct LineProgress {
        content: usize,
        reasoning: usize,
        tool_args: HashMap<usize, usize>,
    }
    let mut progress = LineProgress::default();

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
                "[LLM] 首个数据块到达 (TTFB {:?})",
                first_chunk_at.unwrap().duration_since(started)
            );
        }
        chunk_count += 1;
        total_bytes += chunk.as_ref().map(|c| c.len()).unwrap_or(0) as u64;

        drain_pending_signals(channel, abort_flag);
        if abort_flag.load(Ordering::SeqCst) {
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
                        dispatch_and_track(
                            event,
                            root_id,
                            channel,
                            &mut out,
                            &mut first_content_logged,
                            started,
                        )
                        .await?;
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
                        dispatch_and_track(
                            ev,
                            root_id,
                            channel,
                            &mut out,
                            &mut first_content_logged,
                            started,
                        )
                        .await?;
                    }
                }
                break;
            } else {
                break;
            }
        }
    }

    // ③ 流结束日志：正常结束 / 中止 / 空流，均带统计信息。
    // 若此处之后长时间无下文（工具执行/下一轮请求），可据此定位卡死发生在「流结束后」阶段。
    if abort_flag.load(Ordering::SeqCst) {
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
    channel: &PluginChannel,
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
                "[LLM] 收到第一条消息（{}内容，自请求发起 {:?}）",
                kind,
                started.elapsed()
            );
        }
    }
    dispatch_protocol_event(ev, root_id, channel, out).await
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
            // 首个增量：发**完整快照**创建正文子节点（id 此后保持一致）；
            // 后续增量：显式 Append 窄帧，消费循环按 id 追加——不重发整条，
            // 也不需要消费端从「补丁形状」里猜这是追加还是替换。
            if out.response_text_child_id.is_empty() {
                out.response_text_child_id = short_id();
                emit_update(
                    channel,
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
                emit_append(channel, &out.response_text_child_id, &c).await;
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
                emit_update(
                    channel,
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
                emit_append(channel, &out.reasoning_child_id, &r).await;
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
                        tool_call_id: Some(wire_id),
                        ..Default::default()
                    },
                )
                .await;
            } else if let Some(delta) = args.as_deref().filter(|d| !d.is_empty()) {
                emit_append(channel, &tc_id, delta).await;
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

    /// apinex qwen-3.8-max 网关的真实行为：
    /// 首个增量携带合法 id，后续增量重复发送 `id:""`。
    /// 空串不得覆盖合法 wire id——否则最终得到 Some("")，工具调用被误判为
    /// id 缺失而跳过，落库的 ToolCall 节点无结果子节点。
    /// 节点 id 与 wire id 分离：节点 id 稳定，wire id 保留 provider 原值。
    #[test]
    fn empty_id_delta_does_not_overwrite_real_id() {
        let mut acc = ToolCallAccumulator::default();
        let (node1, wire1, _, _, snapshot1) = acc.process_delta(
            0,
            Some("call_8f3a59f5f8e14258a427e432"),
            Some("get_weather"),
            Some(""),
        );
        assert!(snapshot1, "首个增量必须要求完整快照（接收端尚无此节点）");
        assert_eq!(wire1, "call_8f3a59f5f8e14258a427e432");
        assert!(!node1.is_empty());
        assert_ne!(node1, wire1, "节点 id 是本地分配的，不等于 wire id");

        // 后续增量：id:""（该网关的真实行为）
        let (node2, wire2, args, _, snapshot2) =
            acc.process_delta(0, Some(""), None, Some("{\"city\": \"Paris\"}"));
        assert!(
            !snapshot2,
            "后续增量未改身份字段 ⇒ 只需窄追加（Append），不得整条重发"
        );
        assert_eq!(node2, node1, "节点 id 在同一调用内稳定");
        assert_eq!(wire2, "call_8f3a59f5f8e14258a427e432");
        assert_eq!(args, "{\"city\": \"Paris\"}");

        let done = acc.get_completed();
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].id.as_deref(), Some(node1.as_str()));
        assert_eq!(
            done[0].wire_id.as_deref(),
            Some("call_8f3a59f5f8e14258a427e432")
        );
        assert_eq!(done[0].name.as_deref(), Some("get_weather"));
    }

    /// 供应商始终未返回 id 时，节点 id 在首个增量即确定（流式/落库/执行三处一致，
    /// 且重复取值幂等——chat_loop 与 into_messages 各取一次，不一致会使结果子节点
    /// 变孤儿）；wire id 为 None，请求构建回退节点 id。
    #[test]
    fn missing_id_gets_stable_generated_guid() {
        let mut acc = ToolCallAccumulator::default();
        let (stream_id, wire, _, _, _) =
            acc.process_delta(0, None, Some("vdfs_list"), Some("{\"path\": \".\"}"));
        assert!(!stream_id.is_empty(), "流式期间即应有非空节点 id");
        assert_ne!(stream_id, "tc-0", "不得再使用 index 占位符");
        assert_eq!(wire, stream_id, "无 wire id 时回退为节点 id");

        let done1 = acc.get_completed();
        let done2 = acc.get_completed();
        assert_eq!(done1[0].id.as_deref(), Some(stream_id.as_str()));
        assert_eq!(
            done2[0].id.as_deref(),
            Some(stream_id.as_str()),
            "重复调用 get_completed 必须返回同一节点 id"
        );
        assert!(
            done1[0].wire_id.is_none(),
            "供应商未提供 id ⇒ wire_id 为 None"
        );
    }

    /// 纯空白 id 视为"不合法"，与缺失同等对待。
    #[test]
    fn whitespace_id_treated_as_missing() {
        let mut acc = ToolCallAccumulator::default();
        let (node_id, _, _, _, _) = acc.process_delta(0, Some("   "), None, Some("{}"));
        assert!(!node_id.trim().is_empty());
    }

    /// 空串 name 不得覆盖首个增量的合法 name（与 id 同理）。
    #[test]
    fn empty_name_delta_does_not_overwrite_real_name() {
        let mut acc = ToolCallAccumulator::default();
        acc.process_delta(0, Some("call_x"), Some("cmd.exe"), Some(""));
        acc.process_delta(0, Some(""), Some(""), Some("{}"));

        let done = acc.get_completed();
        assert_eq!(done[0].wire_id.as_deref(), Some("call_x"));
        assert_eq!(done[0].name.as_deref(), Some("cmd.exe"));
    }

    /// 多个并行工具调用（不同 index）互不干扰，各自持有独立的节点 id 与 wire id。
    #[test]
    fn parallel_tool_calls_keep_separate_ids() {
        let mut acc = ToolCallAccumulator::default();
        acc.process_delta(0, Some("call_a"), Some("f1"), Some("{}"));
        acc.process_delta(1, Some("call_b"), Some("f2"), Some("{}"));

        let mut done = acc.get_completed();
        done.sort_by(|a, b| a.id.cmp(&b.id));
        let wire_ids: Vec<_> = done.iter().filter_map(|t| t.wire_id.clone()).collect();
        assert!(wire_ids.contains(&"call_a".to_string()));
        assert!(wire_ids.contains(&"call_b".to_string()));
        let node_ids: Vec<_> = done.iter().filter_map(|t| t.id.clone()).collect();
        assert_ne!(node_ids[0], node_ids[1], "节点 id 互不相同");
    }

    /// 回归（本次迁移的根因之一）：跨轮复用同一 provider id 的两个累积器
    /// （模拟同一会话的两轮 LLM 请求）必须产生**不同的节点 id**——否则第二轮
    /// 的工具调用会更新到第一轮的老节点。
    #[test]
    fn reused_wire_id_across_turns_yields_distinct_node_ids() {
        let mut turn1 = ToolCallAccumulator::default();
        let (node1, _, _, _, _) = turn1.process_delta(0, Some("call_0"), Some("f"), Some("{}"));
        let mut turn2 = ToolCallAccumulator::default();
        let (node2, wire2, _, _, _) = turn2.process_delta(0, Some("call_0"), Some("f"), Some("{}"));

        assert_eq!(wire2, "call_0");
        assert_ne!(node1, node2, "跨轮同 wire id 必须产生不同节点 id");
    }

    /// 空串/纯空白参数是无参工具的合法形态（`from_str("")` 必失败）：
    /// 必须视为 `{}` 且**不得**标记 parse_error，否则无参工具会被误拒。
    #[test]
    fn empty_arguments_treated_as_empty_object() {
        let mut acc = ToolCallAccumulator::default();
        acc.process_delta(0, Some("call_e"), Some("vdfs_list"), Some(""));
        acc.process_delta(1, Some("call_w"), Some("vdfs_list"), Some("   "));

        let done = acc.get_completed();
        assert_eq!(done.len(), 2);
        for tc in &done {
            assert_eq!(tc.arguments, serde_json::json!({}));
            assert!(tc.parse_error.is_none(), "空参数不得标记为解析失败");
        }
    }

    /// 合法 JSON 参数照常解析，parse_error 为 None。
    #[test]
    fn valid_arguments_parse_without_error() {
        let mut acc = ToolCallAccumulator::default();
        acc.process_delta(
            0,
            Some("call_v"),
            Some("cmd"),
            Some(r#"{"command": "dir"}"#),
        );

        let done = acc.get_completed();
        assert_eq!(done[0].arguments, serde_json::json!({"command": "dir"}));
        assert!(done[0].parse_error.is_none());
    }

    /// 回归（卡思考根因）：非空但非法的参数 JSON（典型为 max_tokens 截断）
    /// **不得**静默回退 `{}`——必须保留原文于 parse_error，由执行侧拒绝执行。
    /// 静默 `{}` 会让工具报「缺少必填参数」，模型看不懂原因便原样重试。
    #[test]
    fn truncated_arguments_flagged_not_silently_emptied() {
        let mut acc = ToolCallAccumulator::default();
        let raw = r#"{"command": "cargo test"#;
        acc.process_delta(0, Some("call_t"), Some("cmd"), Some(raw));

        let done = acc.get_completed();
        assert_eq!(done[0].arguments, serde_json::json!({}), "占位仍为 空对象");
        assert_eq!(
            done[0].parse_error.as_deref(),
            Some(raw),
            "必须保留残破原文"
        );
    }

    /// 参数分片：**只有首个增量**要求完整快照，其后每一片都只要求窄追加。
    ///
    /// 这是「ToolCall 请求参数与 Text / Reasoning 同构」的协议契约——帧面只有
    /// 「整条替换」与「尾部追加」两种语义，接收端不必按节点类型去猜。
    #[test]
    fn only_first_args_fragment_requires_snapshot() {
        let mut acc = ToolCallAccumulator::default();
        let (_, _, args1, _, snap1) =
            acc.process_delta(0, Some("call_s"), Some("vdfs_read"), Some("{\"pa"));
        let (_, _, args2, _, snap2) = acc.process_delta(0, None, None, Some("th\": \".\"}"));
        let (_, _, args3, _, snap3) = acc.process_delta(0, None, None, Some(""));

        assert!(snap1, "首片建节点 ⇒ 完整快照");
        assert!(!snap2, "第二片仅为参数增长 ⇒ 窄追加");
        assert!(!snap3, "空片不改动任何东西 ⇒ 窄追加（调用方跳过）");
        assert_eq!(args1, "{\"pa");
        assert_eq!(args2, "{\"path\": \".\"}");
        assert_eq!(args3, args2, "空片不改变累积值");
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

    /// 端到端（纯内存）：reasoning-only 的 TurnOutput 落库消息里
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
