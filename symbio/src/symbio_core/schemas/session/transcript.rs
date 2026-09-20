//! VDFS 转写消费：**节点 → 消息**的反向投影 + **全量 → 增量**折算
//!
//! ## 谁需要它
//!
//! 会话转写在 VDFS 上是**列表**（`<根>/session/<sid>/消息/<mid>`），实时入口是
//! `kind = "vdfs"` 的变更。前端有一份自己的投影（`services/vdfsTranscriptSync.ts`
//! 的 `messageFromNode`）；**进程内**的两个消费者——子智能体转播
//! （`plugins/agent/host/subagent.rs`）与 CLI（`cli/src/client.rs`）——需要等价的一份，
//! 于是收在本模块（`symbio_core` 是两者唯一可达的共同层）。
//!
//! ## 两个方向为什么都要显式折算
//!
//! - **节点 → 消息**：节点把「结构」放在 `attributes`、把「正文」放在内容里
//!   （正向投影见 `plugins/session/plugin/nodes.rs::message_node`）。反向必须它同源，
//!   否则同一份数据在两个消费者眼里长成两个样子。
//! - **全量 → 增量**：VDFS 的 `created` / `updated` 载荷是**全量**（"现在是什么"），
//!   而 `StreamEvent::Update` 与 `ChatMessage::apply_patch` 是**增量**语义
//!   （`Text` / `Reasoning` 的 `content` 是"多了什么"）。把全量当增量拼，
//!   每来一次 `updated` 正文就翻一倍（叠字）。[`TranscriptPatchBuilder`] 因此
//!   记住每个 id 已知的正文，把全量折算成新增的那一段。
//!
//! ## 组合节点（Turn / ToolCall）的正文是「整条消息的 JSON 视图」
//!
//! `nodes.rs::message_text` 对组合节点给的是 `serde_json::to_string_pretty(msg)`，
//! 而不是 `msg.content`（工具参数）。这是**读**路径需要的（在同一地址上取到完整结构），
//! 因此逆投影就是把它读回来——[`message_from_vdfs_node`] 对这两个类型走
//! `serde_json::from_str::<ChatMessage>`，于是工具参数不会在进程内消费者手里
//! 退化成「包了一层的 JSON」。

use serde_json::Value;
use std::collections::HashMap;

use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
};
use crate::symbio_core::vdfs_provider::{
    VdfsChange, VdfsNode, VDFS_CHANGE_APPENDED, VDFS_CHANGE_CREATED, VDFS_CHANGE_DELETED,
    VDFS_CHANGE_TRUNCATED, VDFS_CHANGE_UPDATED,
};

/// 一条 VDFS 转写变更折算出的结果。
///
/// 为什么不是 `Option<ChatMessage>`：`created` / `updated` **未带节点视图**时，
/// 折算器**没有回读能力**（它是进程内纯函数，不持有路由句柄），只能跳过——
/// 这与「本来就没有补丁」（删除 / 空增量）是两件事，混成 `None` 会让调用方
/// 无法留痕，而静默跳过正是本仓库反复踩到的那类事故。
// 变体尺寸差异是刻意的：`ChatMessage` 是本模块的**产物**，而调用方拿到后立刻消费
// （渲染 / 转发到工具通道），生命周期只到下一个 `apply` 为止。把它装进 `Box` 能让
// 枚举变小，代价是热路径上每次变更多一次分配——拿分配换一个只在栈上待一瞬的尺寸，
// 不划算。
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum PatchOutcome {
    /// 增量补丁：转发 / 渲染它
    Patch(ChatMessage),
    /// 本次变更不产生补丁（删除 / 截断 / 空增量）
    Nothing,
    /// `created` / `updated` 缺节点视图，折算器无法处理——调用方应留痕
    MissingPayload,
}

/// VDFS 消息节点 + 正文 → 消息（`plugins/session/plugin/nodes.rs::message_node` 的逆）。
///
/// 结构字段取自节点 `attributes`（`role` / `type` / `parent_id` / `name` / `seq` /
/// `error` / `meta`），状态取自 `node.status`（消息状态词与 `MessageStatus` 同源）；
/// 正文按类型决定：组合节点解析整条消息 JSON，其余原样装箱。
pub fn message_from_vdfs_node(node: &VdfsNode, content: &str) -> ChatMessage {
    let attrs = &node.attributes;
    let attr = |k: &str| attrs.get(k);
    let role = attr("role").and_then(|v| serde_json::from_value::<MessageRole>(v.clone()).ok());
    let msg_type = attr("type").and_then(|v| serde_json::from_value::<MessageType>(v.clone()).ok());

    // 组合节点：正文即整条消息的 JSON 视图（见模块文档），读回来就得到了原消息。
    if matches!(
        msg_type,
        Some(MessageType::Turn) | Some(MessageType::ToolCall)
    ) {
        if let Ok(mut m) = serde_json::from_str::<ChatMessage>(content) {
            // id 以**地址**为准：节点名是权威寻址（JSON 视图可能来自合并后的视图）
            m.id = node.name.clone();
            return m;
        }
    }

    ChatMessage {
        id: node.name.clone(),
        role,
        msg_type,
        parent_id: attr("parent_id")
            .and_then(Value::as_str)
            .map(str::to_string),
        name: attr("name").and_then(Value::as_str).map(str::to_string),
        content: if content.is_empty() {
            None
        } else {
            Some(MessageContent::Text(content.to_string()))
        },
        // `active` 一类**非消息**状态词在这里解析失败 ⇒ `None`（会话节点就是这么被排除的）
        status: serde_json::from_value::<MessageStatus>(Value::String(node.status.clone())).ok(),
        error: attr("error").and_then(Value::as_str).map(str::to_string),
        meta: attr("meta").filter(|v| !v.is_null()).cloned(),
        timestamp: node.updated_at,
        seq: attr("seq").and_then(Value::as_i64),
        ..Default::default()
    }
}

/// 把一个会话的 VDFS 变更折算成**增量消息补丁**（`StreamEvent::Update` 语义）。
///
/// 除了产出补丁，它还维护一份**该会话消息的合并视图**（`id → ChatMessage`）：
/// 调用方不必自己再记一遍内容（[`Self::last_message`]），
/// 这正是「全量 → 增量」能算得出来的前提。
///
/// 只处理消息级地址；会话节点（`path` = 会话 id）不是消息，`apply` 视其为
/// [`PatchOutcome::Nothing`]——会话运行态由节点 `status` 承载，
/// 不折算成消息补丁（那是两张表，合并会让「谁在跑」有两处判据）。
#[derive(Default)]
pub struct TranscriptPatchBuilder {
    known: HashMap<String, ChatMessage>,
}

impl TranscriptPatchBuilder {
    /// 折算一条变更。
    pub fn apply(&mut self, change: &VdfsChange) -> PatchOutcome {
        let Some(id) = change
            .path
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .map(str::to_string)
        else {
            return PatchOutcome::Nothing;
        };

        match change.change.as_str() {
            VDFS_CHANGE_APPENDED => {
                let Some(delta) = change.delta.as_deref().filter(|d| !d.is_empty()) else {
                    return PatchOutcome::Nothing;
                };
                let entry = self.known.entry(id.clone()).or_insert_with(|| ChatMessage {
                    id: id.clone(),
                    ..Default::default()
                });
                if let Some(MessageContent::Text(buf)) = entry.content.as_mut() {
                    buf.push_str(delta);
                } else {
                    entry.content = Some(MessageContent::Text(delta.to_string()));
                }
                PatchOutcome::Patch(ChatMessage {
                    id,
                    content: Some(MessageContent::Text(delta.to_string())),
                    ..Default::default()
                })
            }

            VDFS_CHANGE_CREATED | VDFS_CHANGE_UPDATED => {
                let Some(node) = change.node.as_ref() else {
                    return PatchOutcome::MissingPayload;
                };
                let full = change.content.clone().unwrap_or_default();
                let prev = self
                    .known
                    .get(&id)
                    .and_then(|m| m.content.as_ref().map(MessageContent::to_text))
                    .unwrap_or_default();

                let merged = message_from_vdfs_node(node, &full);
                // 追加语义的类型（Text / Reasoning）只发**新增的那一段**；
                // 其余类型（ToolCall / Turn / UserPrompt …）本就是全量替换。
                let patch_content = if matches!(
                    merged.msg_type,
                    Some(MessageType::Text) | Some(MessageType::Reasoning)
                ) {
                    match full.strip_prefix(prev.as_str()) {
                        // 正文没变（纯状态迁移）：补丁不带 content，只带状态
                        Some("") => None,
                        Some(rest) => Some(MessageContent::Text(rest.to_string())),
                        // 前缀不成立（整段被改写）：按全量兜底，消费者不会丢内容
                        None => Some(MessageContent::Text(full.clone())),
                    }
                } else {
                    merged.content.clone()
                };

                let mut patch = merged.clone();
                patch.content = patch_content;
                // 合并视图存的是**全量**那一份（下次折算的基准）
                self.known.insert(id, merged);
                PatchOutcome::Patch(patch)
            }

            VDFS_CHANGE_DELETED => {
                self.known.remove(&id);
                PatchOutcome::Nothing
            }

            // 尾部截断：该节点及其之后全部没了。折算器的合并视图按 id 索引，
            // 无法知道「之后」是哪些（顺序在调用方的列表里），故整体作废 ——
            // 宁可让后续 `created` 走一次全量兜底，也不留下已被删除的残留。
            VDFS_CHANGE_TRUNCATED => {
                self.known.clear();
                PatchOutcome::Nothing
            }

            _ => PatchOutcome::Nothing,
        }
    }

    /// 折算器眼中的**合并视图**（该 id 目前的全量消息）。
    pub fn last_message(&self, id: &str) -> Option<&ChatMessage> {
        self.known.get(id)
    }
}

#[cfg(test)]
#[path = "transcript.test.rs"]
mod tests;
