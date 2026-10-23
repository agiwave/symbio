//! session 侧的**落库视图**：把一轮 LLM 产物组合成助手消息组（根骨架 + 子节点），
//! 以及挂在用户消息 `prompt` 字段上的时间上下文。
//!
//! 纯构造器（`llm_build_text_node` / `llm_build_tool_call_nodes` / `llm_build_tool_message`
//! / `tool_call_args_text`）与建树折叠（`detach_if_flat` / `flatten_whole_tree` /
//! `turn_root_of` / 枚举 `PresentationTree`）住
//! [`crate::symbio_core::node_build`]（[D-082](../../../../docs/003-design/014-单环一表.md "D-082")
//! 裁决三，[plan/14 步 6](../../../../docs/003-design/014-单环一表.md "DE-014") 切片批）——
//! 驱动器切成独立插件 `loop` 后，留守的读侧投影（`rebuild`）与驱动器写侧隔着 E-009 的
//! 插件墙共用同一份形状源。本文件留下的是**只有驱动器有的组合**：
//!
//! | 符号 | 消费方 |
//! |---|---|
//! | [`llm_build_assistant_messages`] | 本文件的 [`TurnOutput::into_messages`]（`chat_loop/turn.rs` 唯一调用点） |
//! | [`TurnStreamChildIds`] | 同上——落库复用流式期已广播的子节点 id |
//! | [`TurnOutput::into_messages`] / [`TurnOutput::is_reasoning_only`] / [`TurnOutput::effective_text`] | `chat_loop/{turn,state,compress}.rs` |
//!
//! `impl TurnOutput` 的三个方法定义在这里而非 core：它们的外部消费方只有驱动器，
//! 且 [`Self::into_messages`] 的实现依赖本模块的 [`llm_build_assistant_messages`]。
//! 同一个 crate 内固有实现可以落在任意模块（插件与 `symbio_core` 同属 crate
//! `symbio`），调用方**不需要 import 实现所在的模块**——`out.into_messages(root_id)`
//! 与从前完全一样。
//!
//! model 的 `message_builder.test.rs` 曾把 [`llm_build_assistant_messages`] 当测试
//! fixture 用（构造 Turn 树喂给 `flatten_chat_messages`）。那不是生产消费：跨插件
//! 互引被 `plugin-entry-audit` E-009 禁止，故那侧改为本地构造 fixture，形状契约
//! 由本文件的测试锁定（见 `message_build.test.rs` 里的用例 A / A2 / B / C / D）。

use crate::symbio_core::chat_node::{
    ChatNode, MessageContent, NodeKind, NodeRole, NodeStatus, CHAT_STYLE_TAG_REASONING,
    CHAT_STYLE_TAG_RUN,
};
use crate::symbio_core::node_build::{llm_build_text_node, llm_build_tool_call_nodes};
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

// 消息构造（ChatNode 家族）

/// 构造助手消息组（基于 Turn / ToolCall 的分型层级结构）。
///
/// 结构：
/// - `Turn`(根级, `Assistant` 组合)：与 `User` 互为兄弟
///   ├─ `Reasoning`(子)
///   ├─ `Text`(回复, 子)
/// - `ToolCall`(`Assistant` 组合, 子)：自身 `content` 携带请求参数（JSON 文本）
///   └─ `Text`(响应结果, `Tool`, 子)  ← 由 [`crate::symbio_core::node_build::llm_build_tool_message`] 补充
pub fn llm_build_assistant_messages(
    id: &str,
    content: &str,
    tool_calls: &[TurnToolCallInfo],
    rid: Option<String>,
    reasoning: Option<String>,
    child_ids: TurnStreamChildIds,
) -> Vec<ChatNode> {
    let child_ids = child_ids.normalized();
    let mut msgs = Vec::new();
    let timestamp = crate::symbio_core::clock_now_ms();

    // ── 骨架（一整轮响应）消息（根级，与 User 互为兄弟）───────────────────
    msgs.push(
        ChatNode {
            id: id.to_string(),
            parent_id: None,
            role: Some(NodeRole::Assistant),
            msg_type: Some(NodeKind::Skeleton),
            content: None,
            status: Some(NodeStatus::Completed),
            timestamp: Some(timestamp),
            ..Default::default()
        }
        .with_style_tag(CHAT_STYLE_TAG_RUN),
    );

    // ── 推理正文消息（parent_id=turn_id）──────────────────────────────
    // 仅当 reasoning 与回复正文为「不同内容」（即存在独立的文本回复）时才单独生成思考子节点。
    // reasoning-only 场景下 `reasoning` 已通过 `content`（effective_text 对「无文本回复」的回退）
    // 承载于下方的正文子节点；若此处再生成推理子节点，同一段内容会在存储层出现两份
    // （factor=2：表现为历史会话里重复两份、流式期间"层层叠加"）。
    let reasoning_only = reasoning
        .as_ref()
        .map(|r| !r.trim().is_empty() && r.trim() == content.trim())
        .unwrap_or(false);
    if let Some(r) = reasoning {
        if !r.trim().is_empty() && !reasoning_only {
            msgs.push(
                ChatNode {
                    id: child_ids.reasoning.clone().unwrap_or_else(llm_short_id),
                    parent_id: Some(id.to_string()),
                    role: Some(NodeRole::Assistant),
                    msg_type: Some(NodeKind::Notice),
                    content: Some(MessageContent::Text(r)),
                    status: Some(NodeStatus::Completed),
                    timestamp: Some(timestamp),
                    ..Default::default()
                }
                .with_style_tag(CHAT_STYLE_TAG_REASONING),
            );
        }
    }

    // ── 回复正文消息（parent_id=turn_id）───────────────────────────────
    // 仅在存在非空白文本内容时添加，避免产生仅含 \n\n 的空节点
    if !content.trim().is_empty() {
        // reasoning-only 场景下这块内容在流式层是以推理子节点的形式存在的
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
        msgs.push(llm_build_text_node(
            id,
            content,
            rid,
            text_child_id.unwrap_or_else(llm_short_id),
        ));
    }

    // ── ToolCall 消息（parent_id=turn_id，组合节点）──────────────────────
    msgs.extend(llm_build_tool_call_nodes(id, tool_calls));

    msgs
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
    pub fn into_messages(self, root_id: &str) -> Vec<ChatNode> {
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
