//! session 侧的**落库视图**：把一轮 LLM 产物写成助手消息组，以及工具结果消息。
//!
//! 原住 [`symbio_core::llm::turn`](crate::symbio_core::llm::turn)。按 ADR-023 的
//! 「依赖方数量」判据（ADR-038）整组迁下——生产路径上它们的消费方**只有 session**：
//!
//! | 符号 | 消费方 |
//! |---|---|
//! | [`llm_build_tool_message`] | `tool_executor.rs`（工具结果节点的唯一写入者）· `chat_loop/turn.rs` |
//! | [`llm_build_assistant_messages`] | 本文件的 [`TurnOutput::into_messages`]（`chat_loop/turn.rs` 唯一调用点） |
//! | [`TurnStreamChildIds`] | 同上——落库复用流式期已广播的子节点 id |
//! | [`TurnOutput::into_messages`] / [`TurnOutput::is_reasoning_only`] / [`TurnOutput::effective_text`] | `chat_loop/{turn,state,compress}.rs` |
//!
//! **留在 core 的是「结果形态」本身**：`TurnOutput`（`ModelProvider::execute_turn`
//! 的返回类型，model 产出 · session 消费 · `providers/collectors` 读取）、
//! `TurnToolCallInfo`（`TurnOutput::tool_calls` 的元素类型，两侧共用）、
//! `llm_short_id`（消息构造与流式累积共用的 id 原语）。core 侧只剩数据与 id，
//! **没有一个会调到本模块**——方向永远是 session → core。
//!
//! `impl TurnOutput` 的三个方法定义在这里而非 core：它们的外部消费方只有
//! session，且 [`Self::into_messages`] 的实现依赖本模块的 [`llm_build_assistant_messages`]。
//! 同一个 crate 内固有实现可以落在任意模块（插件与 `symbio_core` 同属 crate
//! `symbio`），调用方**不需要 import 实现所在的模块**——`out.into_messages(root_id)`
//! 与从前完全一样。
//!
//! model 的 `message_builder.test.rs` 曾把 [`llm_build_assistant_messages`] 当测试
//! fixture 用（构造 Turn 树喂给 `flatten_chat_messages`）。那不是生产消费：跨插件
//! 互引被 `plugin-entry-audit` E-009 禁止，故那侧改为本地构造 fixture，形状契约
//! 由本文件的测试锁定（见 `message_build.test.rs` 里的用例 A / A2 / B / C / D）。

use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
};
use crate::symbio_core::{llm_short_id, TurnOutput, TurnToolCallInfo};

// 流式子节点 id 的复用

/// 流式期间已经广播给前端的子节点 id。
///
/// **落库时必须复用这些 id**：流式层（`parse_sse_stream` 的 `emit_update`）
/// 与存储层（[`llm_build_assistant_messages`]）是同一批节点的两个视图。若两层各自
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

// 消息构造（ChatMessage 家族）

/// 构造助手消息组（基于 Turn / ToolCall 的分型层级结构）。
///
/// 结构：
/// - `Turn`(根级, `Assistant` 组合)：与 `User` 互为兄弟
///   ├─ `Reasoning`(子)
///   ├─ `Text`(回复, 子)
/// - `ToolCall`(`Assistant` 组合, 子)：自身 `content` 携带请求参数（JSON 文本）
///   └─ `Text`(响应结果, `Tool`, 子)  ← 由 [`llm_build_tool_message`] 补充
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

// 单轮产物的**落库视图**（固有实现住本模块，见文件头）

impl TurnOutput {
    /// 本轮只有思考、没有独立文本回复，且**没有**工具调用。
    ///
    /// 有工具调用时不算：此时 reasoning 需要作为独立子节点保留。
    pub fn is_reasoning_only(&self) -> bool {
        self.text.trim().is_empty() && !self.reasoning.is_empty() && self.tool_calls.is_empty()
    }

    /// 有效正文：reasoning-only 时回退为 reasoning（见 [`Self::is_reasoning_only`]）。
    pub fn effective_text(&self) -> &str {
        if self.is_reasoning_only() {
            &self.reasoning
        } else {
            &self.text
        }
    }

    /// 按值消费本产物，落库为助手消息组（Turn + 子节点）。
    pub fn into_messages(self, root_id: &str) -> Vec<ChatMessage> {
        let effective = self.effective_text().to_owned();
        let reasoning = if self.reasoning.is_empty() {
            None
        } else {
            Some(self.reasoning)
        };
        llm_build_assistant_messages(
            root_id,
            &effective,
            &self.tool_calls,
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
#[path = "message_build.test.rs"]
mod tests;
