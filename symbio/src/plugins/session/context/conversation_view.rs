//! 对话线投影 —— 从**一份存储**里切出「用户与助手说过的话」。
//!
//! ## 一条存储，两条线，一个分界
//!
//! 存储只有一份（ADR-020：转写只有一个写入者），但有两种读者：
//!
//! | | **对话线**（本文件） | **工作线**（[`super::view::build_request_view`]） |
//! |---|---|---|
//! | 内容 | user 消息 + assistant **文本节点** | 推理 / 工具调用 / 工具结果 / Turn 容器 |
//! | 谁读 | `triage` / `reply` 的上下文 + 前端「对话」面板 | worker 的请求视图 + 前端「工作」面板 |
//!
//! **分界是一条纯函数，不是两处判断**：三处共用同一份规则（两个插件 + 前端），
//! 否则会出现「界面看得到、插件看不到」的错位——那类 bug 没有错误信号，
//! 只表现为「助手答得不对」，排查方向会被带偏到提示词上。
//!
//! ## 判据只有两条
//!
//! 1. `role = user`（根级）；
//! 2. `role = assistant` 且是**文本节点**（`msg_type` 缺省或 `Text`），且**根级**
//!    （`parent_id` 为空）。
//!
//! 第 2 条的「根级」是必须的：`Turn` 的子 `Text` 节点是**回复正文**，它属于工作线
//! 的一轮；而根级的 assistant 文本节点才是「助手单独说的一句话」（首响 / 汇报）。
//! 两者在节点树上长得一样，靠 `parent_id` 区分——这也正是「对话线不是节点类型，
//! 是位置」的由来。
//!
//! ## 它**不**看 `meta.exclude_from_context`
//!
//! 两个过滤器看着矛盾，其实各管一条线：
//!
//! - `exclude_from_context` 管的是**模型请求包**（工作线）：首响那句话不进
//!   provider 的消息数组，否则会出现连续两条 `assistant`（部分协议直接 400）；
//! - 本函数管的是**对话线**：首响是用户**真的看到过**的一句话，它当然属于对话——
//!   下一轮 `reply` 若不知道上一轮说过「好的，我去看看」，措辞就会重复或断裂。
//!
//! 把两者混成一个开关，会让「用户看得到但模型看不到」与「模型看得到但用户看不到」
//! 这两种**都错**的状态变成同一件事。

use crate::symbio_core::schemas::session::chat_message::{ChatMessage, MessageRole, MessageType};

/// 投影出对话线：按存储顺序保留最后 `limit` 条（`limit = 0` 表示不限制）。
///
/// 返回的是**新 `Vec`**（克隆节点），调用方可以自由改写而不影响存储。
pub fn conversation_view(messages: &[ChatMessage], limit: usize) -> Vec<ChatMessage> {
    let mut out: Vec<ChatMessage> = messages
        .iter()
        .filter(|m| is_conversation_node(m))
        .cloned()
        .collect();
    if limit > 0 && out.len() > limit {
        out.drain(..out.len() - limit);
    }
    out
}

/// 一条节点是否属于对话线（判据见模块文档，只有两条）。
fn is_conversation_node(m: &ChatMessage) -> bool {
    let is_text = matches!(m.msg_type, None | Some(MessageType::Text));
    if !is_text {
        return false;
    }
    match m.role {
        Some(MessageRole::User) => true,
        Some(MessageRole::Assistant) => m.parent_id.is_none(),
        // `tool` / `system` 都不是"说过的话"：前者是工具的产物，后者是注入的框架文本
        _ => false,
    }
}

#[cfg(test)]
#[path = "conversation_view.test.rs"]
mod tests;
