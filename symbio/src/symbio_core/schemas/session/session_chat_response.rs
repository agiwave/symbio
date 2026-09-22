use super::chat_message::ChatMessage;
use serde::{Deserialize, Serialize};

/// `session/chat` 的响应：本轮定稿后的消息副本。
///
/// ## 这里曾经住着 `NodeOp` / `NodeChange`
///
/// 消息变更的线协议一度是独立的一层「显式操作」枚举（`upsert` / `append` /
/// `remove` / `reset` / `warn`），后来收敛成只剩一个变元的 `Change`。两者都已
/// 删除：**帧就是一条 [`ChatMessage`]**（见
/// [`crate::symbio_core::transcript_stream::NodeEvent`]）。
///
/// 原来那层枚举只是在重复 `ChatMessage` 已经有的字段：
///
/// | 原枚举里的字段 | 现在的落点 |
/// |---|---|
/// | `Change.message_id` | `ChatMessage.id` |
/// | `Change.delta` | `ChatMessage.delta`（增量；与 `content` 互斥） |
/// | `Change.status` / `error` / `meta` / 身份字段 | `ChatMessage` 的同名字段 |
/// | `Remove { message_id }` | `status = removed`（一次状态迁移） |
/// | `Reset` | 不存在——协议里没有「清空重读」的消息形态，删除逐条下发 |
/// | `Warn { warning }` | 会话节点状态（`TranscriptWriter::warn`），不是消息帧 |
///
/// 于是「消息变更」只剩一种形状：消费端不必再维护第二套解析与第二套合并规则，
/// 「这条消息现在是什么样」与「这帧说了什么」也不再是两份需要对齐的数据。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Response {
    pub message: ChatMessage,
}
