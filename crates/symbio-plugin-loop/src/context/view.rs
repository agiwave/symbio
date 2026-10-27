//! 请求级注入段 —— 只影响**单次请求、不落库**的置顶文本。
//!
//! 唯一入口 [`build_request_sections`]：把「本轮要注入哪些段」的**数据**变成节点。
//! **对话体不在这里加工**：发给模型的历史是事实网格的纯投影（[D-048a](../../../symbio-plugin-session/README.md#d-048a-v2-的-prompt-形状必须回到结构化消息不把工具结果拍平成纯文本 "crates/symbio-plugin-session/src/README")），
//! 请求视图级的内容淡化 / 轮次淡化 / 分层滑窗骨架化三层已按
//! [D-078](../../../symbio-plugin-session/README.md#d-078-请求视图三层淡化退役请求体是事实网格的纯投影注入段只追加不改写 "crates/symbio-plugin-session/src/README") 退役。
//!
//! 各段都是**追加的读视图**，不是对既有消息的改写——所以它们与「请求体 = 投影」不冲突：
//! 它们以一条 `system` 前缀消息进入请求（见 `chat_loop::request_view_prefix`），
//! 而 `prompt_fidelity` 把这一条当作**声明过的例外**（它不是事件，是本轮临时投影）。
//!
//! ## 段的身份从枚举退回宿主词汇（DE-025 D-215）
//!
//! 这里原先有一个 `SectionKind` 枚举，理由是「生产方与消费方各写一份字面量会静默
//! 拼错」。端口化之后那个理由不再成立：**生产方就是 provider 自己**，而消费方
//! （拼接）按视图次序取**全部**段、不再按名字查表——两处拼写变成一处，枚举因此没有
//! 要防的东西。
//!
//! 名字没有散落：已知的段身份登记在 `symbio_plugin_api::context::slots`，与
//! `behaviors` 同一条纪律（宿主词汇、开放集合、新名字由声明方自带）。这与
//! 「新领域走 `Entity::Domain`」是同一种取舍：**闭集保护的是两处拼写，不是名字本身**。

use super::*;
use symbio_core::chat_node::{MessageContent, NodeStatus};
use symbio_plugin_api::context::section_meta;

/// 构建本次请求的**注入段**（请求级唯一入口）。
///
/// 入参是「要注入哪些段」的**数据**：`(段身份, 正文)`。返回按**入参次序**排列的节点
/// ——段位置就是模型读到的上下文次序，而这次序由收集方声明（provider 的 owner 序），
/// 不由本函数的代码行序隐含。
///
/// 正文的**空值判定不在这里**：空串照样产出一个节点，由取值点
/// （`chat_loop::request_view_prefix`，它 trim 后过滤）决定拼不拼。判据只有一个
/// 出处——两处都判会让「空段算不算注入」取决于改的是哪一处。
///
/// ## 各段的语义位置（收集方按此顺序给）
///
/// 1. `slots::RECALL`：记忆是**背景事实**。置顶而不是置尾——置尾会把「最后一条 user
///    消息」从用户的问题换成记忆，mock 场景匹配与轮次窗口都会读错；也不进系统提示词
///    ——那里有「唯一真源 = 注册段」的纪律（`plugins/session/README.md`）。
/// 2. `slots::READYSET`：`readyset` 投影的**候选集**，不是替模型决定做哪件事
///    （`projection::readyset` 模块头的边界）。
/// 3. `slots::DELEGATE`：委派者的身份目录。
pub fn build_request_sections(sections: &[(&'static str, String)]) -> Vec<ChatNode> {
    sections
        .iter()
        .map(|(slot, text)| ChatNode {
            id: uuid::Uuid::new_v4().to_string(),
            principal: Some(symbio_core::principal::Principal::from_legacy(
                symbio_core::PRINCIPAL_USER,
            )),
            msg_type: Some(NodeKind::Body),
            content: Some(MessageContent::Text(text.clone())),
            status: Some(NodeStatus::Completed),
            meta: Some(section_meta(slot)),
            ..Default::default()
        })
        .collect()
}

#[cfg(test)]
#[path = "view.test.rs"]
mod tests;
