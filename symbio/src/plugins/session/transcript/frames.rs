//! session 侧的**状态帧 / 删除帧**：构造与发射的唯一落点。
//!
//! 它们原住 [`symbio_core::llm::turn`](crate::symbio_core::llm::turn)，按
//! ADR-023 的「依赖方数量」判据（只被一个模块依赖的内容下沉回该模块，见
//! ADR-038）整组迁下：`llm_emit_state` / `llm_state_frame` / `llm_emit_removed`
//! 的**生产消费方只有 session**（`tool_executor` · `chat_loop` · `resume` ·
//! `orchestrator::failure`）。
//!
//! ## 帧家族的另一半仍在 core —— 且必须留在 core
//!
//! | 符号 | 消费方 | 为什么位置相反 |
//! |---|---|---|
//! | `llm_emit_message` / `llm_message_frame` | model（`stream.rs`）· session（`chat_loop` · `plugin` · `vdfs_provider`） | 两侧共用；且 `llm_message_frame` 是 core 自己 `llm_emit_message` 的帧派生（「完整消息必然带状态」只实现一次），两者**同进退** |
//! | [`llm_removed_frame`] | `plugins/agent`（`subagent.rs`）· session | 两个插件共用的构造点 |
//! | `llm_emit_delta` | model（`stream.rs` 的流循环热路径） | 唯一调用方是 model，随调用点住在 `plugins/model/stream.rs` |
//!
//! 反向依赖（core → 本模块）是被禁止的：core 不认识任何插件。所以 core 侧的
//! 帧原语**不能**调用这里，允许的方向只有 session → core（下面的
//! [`llm_emit_removed`] 就是这样调 [`llm_removed_frame`] 的）。
//! 插件之间同样禁止互引（`plugin-entry-audit` E-009）——本插件之外没人能引用本文件。

use crate::symbio_core::schemas::session::chat_message::{ChatMessage, MessageStatus};
use crate::symbio_core::{llm_removed_frame, ExecEventSink};

/// 发送一帧**状态**：身份 + 状态 + 元数据 + 错误，**不带正文**。
///
/// 流式节点的正文已由 `llm_emit_delta`（model 的流循环，住
/// `plugins/model/stream.rs`）逐帧上线，这里再带一次只是把同一段文字二次传输
/// （且会把权威副本的完整正文重新发一遍）。`content` / `delta` 一律剥掉，
/// 免得调用方传了一条「内容齐全的副本」就顺手把它送上热路径。
pub async fn llm_emit_state(sink: &ExecEventSink, msg: ChatMessage) {
    sink.emit(llm_state_frame(&msg)).await;
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

/// 发送一帧**删除**：`status = removed`。
///
/// 协议里没有 `remove` 操作——删除就是一次状态迁移，与出现、增长、完成同走
/// 一条消息帧，接收端据此就地移除节点。
///
/// 帧本身由 core 的 [`llm_removed_frame`] 构造（`plugins/agent` 与本插件共用，
/// 构造点只能有一处）；本函数只是它的 session 侧发射口。
pub async fn llm_emit_removed(sink: &ExecEventSink, message_id: &str) {
    sink.emit(llm_removed_frame(message_id)).await;
}
