//! 请求级注入段 —— 只影响**单次请求、不落库**的三段置顶文本。
//!
//! 唯一入口 [`build_request_sections`]：长期记忆 / 就绪任务 / 委派者真源，按固定次序
//! 插入。**对话体不在这里加工**：发给模型的历史是事实网格的纯投影（[ADR-048a](../../../../../docs/010-adr/048a.md)），
//! 请求视图级的内容淡化 / 轮次淡化 / 分层滑窗骨架化三层已按
//! [ADR-078](../../../../../docs/010-adr/078.md) 退役。
//!
//! 三段都是**追加的读视图**，不是对既有消息的改写——所以它们与「请求体 = 投影」不冲突：
//! 它们以一条 `system` 前缀消息进入请求（见 `chat_loop::request_view_prefix`），
//! 而 `prompt_fidelity` 把这一条当作**声明过的例外**（它不是事件，是本轮临时投影）。

use super::*;
use crate::symbio_core::chat_node::{MessageContent, NodeStatus};

/// 构建本次请求的**注入段**（请求级唯一入口）。
///
/// 次序与位置是判据的一部分（段位置就是模型读到的上下文次序）：
///
/// 1. 长期记忆（S5 步 12）：`recall_section` 非空时插一条 `meta.kind = recall_context`。
///    **置顶**而不是置尾：记忆是背景事实，置尾会把「最后一条 user 消息」从用户的问题
///    换成记忆——mock 场景匹配与轮次窗口都会读错；也不进系统提示词——那里有
///    「唯一真源 = 注册段」的纪律（`plugins/session/README.md`）。
/// 2. 任务调度段（S7 步 16–17，[04 §3.1 批⑨](../../../../../docs/004-plan/004-工程落地.md "PL-004-工程落地")）：
///    `ready_section` 非空时插一条 `meta.kind = readyset`。它给的是 `readyset` 投影的
///    **候选集**，不是替模型决定做哪件事（`projection::readyset` 模块头的边界）。
/// 3. 委派者真源段（ADR-047）：`delegate_section` 非空时插一条
///    `meta.kind = delegate_context`，落 **index 2**。
///
/// 插入次序是「委派 → 就绪 → 记忆」，每段都插在 0，于是三段并存时读作
/// 记忆 0 / 就绪 1 / 委派 2（单独存在时各自在 0）。顺序反了会把三段的位置全部改掉，
/// 而段位置是模型读到的上下文次序的一部分——换一段就换一次上下文次序，
/// 同一会话的两次请求不再可比。
pub fn build_request_sections(
    recall_section: Option<&str>,
    ready_section: Option<&str>,
    delegate_section: Option<&str>,
) -> Vec<ChatNode> {
    let mut sections: Vec<ChatNode> = Vec::new();
    let section = |kind: &str, text: &str| ChatNode {
        id: uuid::Uuid::new_v4().to_string(),
        role: Some(NodeRole::User),
        msg_type: Some(NodeKind::Body),
        content: Some(MessageContent::Text(text.to_string())),
        status: Some(NodeStatus::Completed),
        meta: Some(serde_json::json!({ "kind": kind })),
        ..Default::default()
    };
    // 3) 委派者真源段落 index 2（ADR-047）——**必须最先插**：本段先插 0，
    //    ready 段再插 0 把本段挤到 1，最后记忆段插 0 把两者依次推到 1 / 2。
    if let Some(text) = delegate_section {
        sections.insert(0, section("delegate_context", text));
    }
    // 2) 任务调度段置顶（S7 步 16–17，批⑨）——记忆段随后再插一次 0 把它挤到 1。
    if let Some(text) = ready_section {
        sections.insert(0, section("readyset", text));
    }
    // 1) 长期记忆置顶（S5 步 12）——最后插 0，于是恒在 index 0。
    if let Some(text) = recall_section {
        sections.insert(0, section("recall_context", text));
    }
    sections
}

#[cfg(test)]
#[path = "view.test.rs"]
mod tests;
