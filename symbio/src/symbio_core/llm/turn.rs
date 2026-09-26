//! 单轮 LLM 产物与消息帧原语（core `llm/` 契约层的共享面）。
//!
//! 职责（协议无关、插件无关，供 session 与 model 双侧共同使用）：
//! - 消息帧家族（[`llm_emit_message`]/[`llm_emit_delta`]/[`llm_emit_state`]/[`llm_emit_removed`]
//!   与 `llm_message_frame`/`llm_state_frame`/`llm_removed_frame`）：[`ExecEventSink`]
//!   唯一写入点的帧语义（完整消息 / 增量 / 状态 / 删除）
//! - 单轮产物（[`TurnOutput`] + [`TurnToolCallAccumulator`]）：
//!   `ModelProvider::execute_turn` 的返回类型（见 [`super::model_provider`]）
//! - 消息构造家族（`llm_short_id`/`TurnStreamChildIds`/`llm_build_assistant_messages`/`llm_build_tool_message`）：
//!   `TurnOutput::into_messages` 与 `TurnToolCallAccumulator` 直接依赖它，
//!   孤儿规则要求定义与使用同处 core
//!
//! ## 依赖方对照表（ADR-023 决策 2）
//!
//! 本模块**全部符号都是两侧共用**的，没有单消费方残留。两个最容易被误判为
//! 「只有一侧认」的符号，实测如下——**按类型名 grep 会漏掉它们**，因为消费点走的是
//! `TurnOutput` 的**字段访问**（`.tool_accumulator`）而非类型名：
//!
//! | 符号 | 消费方 | 消费方式 |
//! |---|---|---|
//! | [`TurnToolCallAccumulator`] | model（`stream.rs` 逐块 `process_delta`）· session（`chat_loop/turn.rs` 读 `get_completed` / `had_any_tool_call`）· 本模块（`TurnOutput::into_messages`） | 经 `TurnOutput.tool_accumulator` 字段访问 |
//! | [`TurnStreamChildIds`] | model（`message_builder.test.rs`）· 本模块（[`llm_build_assistant_messages`] 的形参、`into_messages` 的构造点） | 作为**多消费方函数的形参类型** |
//!
//! 两者都不能下沉：前者有 **2 个跨插件**消费方；后者是多消费方函数
//! [`llm_build_assistant_messages`] 的签名组成部分——沉到任一侧，另一侧就调不动该函数
//! （插件间禁止互引，`plugin-entry-audit` E-009）。
//!
//! **HTTP 重试机器与 SSE 流循环不在这里**：它们只有 model 插件的
//! `execute_turn` 使用（实现细节而非契约），住在 `plugins/model/`
//! （`http.rs` / `stream.rs`），行解析契约同处该插件（`protocols/sse.rs`）。
//! 与本模块同层的兄弟模块：[`super::model_provider`]（trait 与结束原因 / 用量）。
//!
//! ## 执行期只与两个原语打交道（不再与通道打交道）
//!
//! 本模块的帧函数只依赖 [`ExecEventSink`]（出：节点事件），**不接受 `PluginChannel`**。
//! 历史上两者都压在同一个通道上，进程内调用因此要付 serde 装箱 + 反序列化的往返
//! 代价；现在「去哪」与「怎么中止」分别由两个语义单一的原语承担，`PluginChannel`
//! 退回纯跨进程传输（见 `symbio_core::exec` 的模块文档）。

use super::model_provider::{ModelFinishReason, ModelUsage};
use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
};
use crate::symbio_core::ExecEventSink;
use serde_json::Value;
use std::collections::HashMap;
use tracing::warn;

// 执行期出口与中止（唯一两个原语）

/// 发送一条**完整消息**（`content` = 整条替换，幂等）。
///
/// 用在正文对接收端是**新的权威副本**的帧上：一次性节点（工具结果 / 用户消息
/// 回填）的单帧完成、存储回执、压缩快照。流式节点的正文已由 [`llm_emit_delta`]
/// 逐帧传过，它的终态走 [`llm_emit_state`]，不在这里重发。
pub async fn llm_emit_message(sink: &ExecEventSink, msg: ChatMessage) {
    sink.emit(llm_message_frame(&msg)).await;
}

/// 由一条完整消息派生**消息帧**（`content` = 整条替换）：缺省补 `completed`。
///
/// 直接写转写（`Transcript::apply`，不经出口）的发布路径也用它——「完整消息必然
/// 带状态」这条约定只在这里实现一次。
pub fn llm_message_frame(m: &ChatMessage) -> ChatMessage {
    let mut frame = m.clone();
    if frame.status.is_none() {
        frame.status = Some(MessageStatus::Completed);
    }
    frame
}

/// 发送一帧**增量**：`delta` 追加到目标节点正文尾部（流式热路径，O(delta)）。
///
/// 目标未知时写入点用帧内信息建占位（帧自给自足，不依赖任何先行帧）。
pub async fn llm_emit_delta(sink: &ExecEventSink, message_id: &str, delta: &str) {
    sink.emit(ChatMessage {
        id: message_id.to_string(),
        delta: Some(delta.to_string()),
        ..Default::default()
    })
    .await;
}

/// 发送一帧**状态**：身份 + 状态 + 元数据 + 错误，**不带正文**。
///
/// 流式节点的正文已由 [`llm_emit_delta`] 逐帧上线，这里再带一次只是把同一段文字
/// 二次传输（且会把权威副本的完整正文重新发一遍）。`content` / `delta` 一律
/// 剥掉，免得调用方传了一条「内容齐全的副本」就顺手把它送上热路径。
pub async fn llm_emit_state(sink: &ExecEventSink, msg: ChatMessage) {
    sink.emit(llm_state_frame(&msg)).await;
}

/// 发送一帧**删除**：`status = removed`。
///
/// 协议里没有 `remove` 操作——删除就是一次状态迁移，与出现、增长、完成同走
/// 一条消息帧，接收端据此就地移除节点。
pub async fn llm_emit_removed(sink: &ExecEventSink, message_id: &str) {
    sink.emit(llm_removed_frame(message_id)).await;
}

/// 由一条完整消息派生**状态帧**：身份 + 状态 + 元数据 + 错误，**不带正文**。
///
/// 直接写转写（`Transcript::apply`，不经出口）的收口路径也用它——那些节点的
/// 正文早已由 `delta` 逐帧上线，重发一遍只是把同一段文字二次传输。
pub fn llm_state_frame(m: &ChatMessage) -> ChatMessage {
    let mut frame = m.clone();
    frame.content = None;
    frame.delta = None;
    if frame.status.is_none() {
        frame.status = Some(MessageStatus::Completed);
    }
    frame
}

/// 删除帧：`status = removed`（发射方与收口路径的唯一构造点，避免各写一份）。
pub fn llm_removed_frame(message_id: &str) -> ChatMessage {
    ChatMessage {
        id: message_id.to_string(),
        status: Some(MessageStatus::Removed),
        ..Default::default()
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
pub struct TurnToolCallInfo {
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
pub struct TurnToolCallAccumulator {
    calls: HashMap<usize, AccumulatedToolCall>,
}

impl TurnToolCallAccumulator {
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

        // 节点 id 在诞生时确定并写入 entry：流式广播、落库（llm_build_assistant_messages）、
        // 执行（process_tool_calls_async）三处使用同一节点 id。
        let is_new_node = entry.node_id.is_empty();
        if is_new_node {
            entry.node_id = llm_short_id();
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
    /// 保证返回的每个 TurnToolCallInfo.id（节点 id）均为非空：正常情况下 process_delta
    /// 已在首个增量确定，此处为幂等兜底——重复调用返回相同 id，**绝不**重新随机生成
    /// （chat_loop 与 into_messages 会各取一次，两次结果不一致会使工具结果子节点变孤儿）。
    pub fn get_completed(&mut self) -> Vec<TurnToolCallInfo> {
        self.calls
            .values_mut()
            .map(|call| {
                if call.node_id.is_empty() {
                    call.node_id = llm_short_id();
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
                TurnToolCallInfo {
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
pub fn llm_short_id() -> String {
    uuid::Uuid::new_v4().to_string()[..8].to_string()
}

/// 流式期间已经广播给前端的子节点 id。
///
/// **落库时必须复用这些 id**：流式层（`parse_sse_stream` 的 `emit_update`）
/// 与存储层（`llm_build_assistant_messages`）是同一批节点的两个视图。若两层各自
/// `llm_short_id()` 生成新 id，同一个文本子节点在「前端流式快照」里是 id=A、
/// 在「会话存储」里是 id=B，会被上层判定为两条不同消息——于是失败收尾时
/// id=A 的节点被当作"尚未落库的流式半截"补写进存储，同一个 Turn 下出现两份内容相同的文本节点。
#[derive(Debug, Default, Clone)]
pub struct TurnStreamChildIds {
    /// 回复正文子节点的流式 id（`TurnOutput::response_text_child_id`）
    pub text: Option<String>,
    /// 思考子节点的流式 id（`TurnOutput::reasoning_child_id`）
    pub reasoning: Option<String>,
}

impl TurnStreamChildIds {
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
///   └─ `Text`(响应结果, `Tool`, 子)  ← 由 `llm_build_tool_message` 补充
pub fn llm_build_assistant_messages(
    id: &str,
    content: &str,
    tool_calls: &[TurnToolCallInfo],
    rid: Option<String>,
    reasoning: Option<String>,
    child_ids: TurnStreamChildIds,
) -> Vec<ChatMessage> {
    let child_ids = child_ids.normalized();
    let mut msgs = Vec::new();
    let timestamp = crate::symbio_core::clock_now_ms();

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
                id: child_ids.reasoning.clone().unwrap_or_else(llm_short_id),
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
            id: text_child_id.unwrap_or_else(llm_short_id),
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
        let tc_id = tc.id.clone().unwrap_or_else(llm_short_id);
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
pub fn llm_build_tool_message(
    tool_call_id: &str,
    content: &str,
    success: Option<bool>,
    msg_id: Option<String>,
) -> ChatMessage {
    let success = success.unwrap_or(true);
    ChatMessage {
        id: msg_id.unwrap_or_else(llm_short_id),
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
        timestamp: Some(crate::symbio_core::clock_now_ms()),
        ..Default::default()
    }
}

// 单轮产物

#[derive(Default)]
pub struct TurnOutput {
    pub text: String,
    pub reasoning: String,
    pub response_id: Option<String>,
    pub tool_accumulator: TurnToolCallAccumulator,
    /// Short ID for the response text child node (consistent across delta updates)
    pub response_text_child_id: String,
    /// Short ID for the reasoning child node
    pub reasoning_child_id: String,
    /// 流结束原因（一次响应最多一次）。用于区分「自然结束」与「max_tokens 截断」
    /// （不区分即表现为「对话突然结束」）。默认 Stop。
    pub finish: ModelFinishReason,
    /// 用量统计（provider 不一定给，故可选）。用于校准 token 估算器。
    pub usage: Option<ModelUsage>,
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
        llm_build_assistant_messages(
            root_id,
            &effective,
            &tools,
            self.response_id,
            reasoning,
            // 复用流式期间已经广播给前端的子节点 id：落库节点与流式节点必须是同一身份，
            // 否则失败收尾时流式节点会被当成"未落库的半截"再补写一份（重复节点）。
            TurnStreamChildIds {
                text: Some(self.response_text_child_id),
                reasoning: Some(self.reasoning_child_id),
            },
        )
    }
}

#[cfg(test)]
#[path = "turn.test.rs"]
mod tests;
