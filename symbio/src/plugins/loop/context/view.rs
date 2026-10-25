//! 请求级注入段 —— 只影响**单次请求、不落库**的置顶文本。
//!
//! 唯一入口 [`build_request_sections`]：长期记忆 / 就绪任务 / 委派者真源，按**调用方
//! 给定的次序**装配。**对话体不在这里加工**：发给模型的历史是事实网格的纯投影（[D-048a](../../session/README.md#d-048a-v2-的-prompt-形状必须回到结构化消息不把工具结果拍平成纯文本 "symbio/src/plugins/session/README")），
//! 请求视图级的内容淡化 / 轮次淡化 / 分层滑窗骨架化三层已按
//! [D-078](../../session/README.md#d-078-请求视图三层淡化退役请求体是事实网格的纯投影注入段只追加不改写 "symbio/src/plugins/session/README") 退役。
//!
//! 各段都是**追加的读视图**，不是对既有消息的改写——所以它们与「请求体 = 投影」不冲突：
//! 它们以一条 `system` 前缀消息进入请求（见 `chat_loop::request_view_prefix`），
//! 而 `prompt_fidelity` 把这一条当作**声明过的例外**（它不是事件，是本轮临时投影）。
//!
//! ## 段的身份是枚举，不是字符串
//!
//! `meta.kind` 此前是一对**各写一份**的字面量：生产方在 [`build_request_sections`]
//! 里写 `"recall_context"`，消费方在 `chat_loop::request_view_prefix` 的 `KINDS`
//! 数组里再写一遍。两处任何一处拼错都不报错——生产方拼错则该段落库为无人认领的
//! kind（模型看不见），消费方拼错则 `find` 落空（同样静默）。[`SectionKind`] 把
//! 它收成闭集枚举后，`name()` 是这段字符串的**唯一出处**，拼错是编译错误。
//!
//! 两条判据仍然分属两处，这是刻意的：[`SectionKind`] 只回答「这一段叫什么」；
//! **视图内位置**由调用方给的数组次序决定，**前缀优先级**（模型读到的先后）由
//! `request_view_prefix` 自己的数组决定。两者今天恰好一致，但不是同一条判据——
//! 把「谁先谁后」绑死在装配次序上，下次调装配次序就会连带改掉模型看到的优先级。

use super::*;
use crate::symbio_core::chat_node::{MessageContent, NodeStatus};

/// 请求级注入段的种类 —— `meta.kind` 字符串的**唯一出处**。
///
/// 加一段 = 加一个变体（`name()` 的 `match` 会编译错误驱动你补全）+ 在调用点的
/// 数组里加一行。不再是「生产方写一遍字面量、消费方再写一遍」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SectionKind {
    /// 长期记忆（S5 步 12）。
    Recall,
    /// 任务调度候选集（S7 步 16–17，[04 §3.1 批⑨](../../../../../docs/003-design/016-工程落地与验证.md "DE-016-工程落地与验证")）。
    ReadySet,
    /// 委派者真源（D-047）。
    Delegate,
}

impl SectionKind {
    /// `meta.kind` 的取值。**这是这段字符串在仓里的唯一出处。**
    pub fn name(self) -> &'static str {
        match self {
            Self::Recall => "recall_context",
            Self::ReadySet => "readyset",
            Self::Delegate => "delegate_context",
        }
    }
}

/// 构建本次请求的**注入段**（请求级唯一入口）。
///
/// 入参是「要注入哪些段」的**数据**：`(段的种类, 正文)`，`None` 的正文表示这一段
/// 本轮不注入。返回按**入参次序**排列的节点——段位置就是模型读到的上下文次序，
/// 而这次序由调用方声明，不由本函数的代码行序隐含。
///
/// 正文的**空值判定不在这里**：`Some("")` 照样产出一个节点，由取值点
/// （`chat_loop::request_view_prefix`，它 trim 后过滤）决定拼不拼。判据只有一个
/// 出处——两处都判会让「空段算不算注入」取决于改的是哪一处。
///
/// ## 各段的语义位置（调用方按此顺序给）
///
/// 1. [`SectionKind::Recall`]：记忆是**背景事实**。置顶而不是置尾——置尾会把「最后
///    一条 user 消息」从用户的问题换成记忆，mock 场景匹配与轮次窗口都会读错；也不
///    进系统提示词——那里有「唯一真源 = 注册段」的纪律（`plugins/session/README.md`）。
/// 2. [`SectionKind::ReadySet`]：`readyset` 投影的**候选集**，不是替模型决定做哪件事
///    （`projection::readyset` 模块头的边界）。
/// 3. [`SectionKind::Delegate`]：委派者的身份目录。
pub fn build_request_sections(sections: &[(SectionKind, Option<&str>)]) -> Vec<ChatNode> {
    sections
        .iter()
        .filter_map(|(kind, text)| {
            let text = (*text)?;
            Some(ChatNode {
                id: uuid::Uuid::new_v4().to_string(),
                principal: Some(crate::symbio_core::principal::Principal::from_legacy(
                    crate::symbio_core::PRINCIPAL_USER,
                )),
                msg_type: Some(NodeKind::Body),
                content: Some(MessageContent::Text(text.to_string())),
                status: Some(NodeStatus::Completed),
                meta: Some(serde_json::json!({ "kind": kind.name() })),
                ..Default::default()
            })
        })
        .collect()
}

#[cfg(test)]
#[path = "view.test.rs"]
mod tests;
