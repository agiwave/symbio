//! 单轮 LLM 的**共享帧原语**与 id 原语；类型面已迁入 `symbio-kernel`（D-188 C2b）。

use crate::symbio_core::chat_node::{ChatNode, NodeStatus};
use crate::symbio_core::ExecEventSink;

pub use symbio_kernel::llm::turn::*;

// 执行期出口（共享的完整消息帧）

/// 发送一条**完整消息**（`content` = 整条替换，幂等）。
///
/// 用在正文对接收端是**新的权威副本**的帧上：一次性节点（工具结果 / 用户消息
/// 回填）的单帧完成、存储回执、压缩快照。流式节点的正文已由 `llm_emit_delta`
/// （`plugins/model/stream.rs`）逐帧传过，它的终态走 `llm_emit_state`
/// （`plugins/session/transcript/frames.rs`），不在这里重发。
pub async fn llm_emit_message(sink: &ExecEventSink, msg: ChatNode) {
    sink.emit(llm_message_frame(&msg)).await;
}

/// 由一条完整消息派生**消息帧**（`content` = 整条替换）：缺省补 `completed`。
///
/// 直接写转写（`Transcript::apply`，不经出口）的发布路径也用它——「完整消息必然
/// 带状态」这条约定只在这里实现一次。
pub fn llm_message_frame(m: &ChatNode) -> ChatNode {
    let mut frame = m.clone();
    if frame.status.is_none() {
        frame.status = Some(NodeStatus::Completed);
    }
    frame
}

/// 删除帧：`status = removed`（发射方与收口路径的唯一构造点，避免各写一份）。
pub fn llm_removed_frame(message_id: &str) -> ChatNode {
    ChatNode {
        id: message_id.to_string(),
        status: Some(NodeStatus::Removed),
        ..Default::default()
    }
}
// id 原语（消息节点 id 的统一格式，两侧共用）

/// 生成长度短的 ID（8 字符，取 UUID v4 前缀）
pub fn llm_short_id() -> String {
    uuid::Uuid::new_v4().to_string()[..8].to_string()
}
