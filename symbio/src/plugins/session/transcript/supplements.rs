//! 补充整合：把一批收件箱条目合并成**一条**用户消息。
//!
//! ## 为什么需要合并（而不是逐条追加）
//!
//! 用户在会话运行中连发多条消息时，今天的行为是"一条消息 = 一轮"。本模块让
//! n 条补充在**同一轮**里被整体处理。合并成**一条**而不是逐条追加，理由是
//! **存储裁剪按 User 消息计数**：
//!
//! `chat_session/write.rs::prune_historical_tool_calls` 用 `role = User` 的消息数
//! 计算保留分水岭（`keep_turns = SessionConfig::context_messages`）。逐条追加会让
//! n 条补充吃掉 n 份保留预算，把更早的轮次提前挤出窗口——用户只是多说了两句话，
//! 历史却悄悄少了两轮。合并成一条，n 条在计数上仍是一轮，与用户心智一致。
//!
//! > 注意另一处窗口**不**受影响：`context/window.rs::apply_layered_sliding_window`
//! > 按 **ToolCall** 计数，与补充条数无关。两处口径不同，不要混。
//!
//! ## n = 1 时**不合并**
//!
//! 只有一条时原样返回该条目的消息——**与今天的行为逐字一致**（同一个 id、同一份
//! 正文、不多一个字段）。因此 `supplements_enabled = true` 也不会改变"只发一条"
//! 这种最常见情况的任何字节，`meta.supplement*` 只在真的发生了合并时出现。
//!
//! ## 合并结果的形状
//!
//! | 字段 | 取值 | 理由 |
//! |---|---|---|
//! | `id` | 第一条条目的 id | 地址末段即身份；前端按 id 合并权威帧，沿用第一条的 id 让其中一条乐观副本天然被替换（见下） |
//! | `parent_id` | `None` | 顶层，与普通用户消息同形（`find_turn_user_split_idx` 要求根级 User 才认） |
//! | `role` / `msg_type` | `User` / `Text` | 它就是一条用户消息 |
//! | `meta.supplement_ids` | 全部原始条目 id，按入队顺序 | **可追溯性**：合并后原始条目只此一处留痕 |
//! | `meta.supplement_count` | n | 让模型知道这是几条的合并 |
//!
//! ## 正文拼接：**不改写用户原文**
//!
//! 用 `"\n\n"` 连接各条正文，不加编号、不加分隔线、不做摘要。改写用户原文是
//! **不可逆的信息损失**——模型看到的必须是用户真正说过的话。需要条目边界时读
//! `meta.supplement_count`。
//!
//! 内容形态**保持无损**：全部条目都是纯文本时产出纯文本；只要有一条携带
//! 非文本片段（如图片），就按顺序把所有片段拼成 `Parts`，不把图片丢掉。

use super::super::active::InboxItem;
use crate::symbio_core::schemas::session::chat_message as cm;

/// 合并标记的三个键名（唯一真源：写入方与消费方都从这里取）。
///
/// 放在常量里而不是各处写字面量：`meta` 是自由 JSON，编译器管不到键名拼写，
/// 散写必然出现"写的键与读的键差一个字母"这种**编译期看不见**的分叉。
pub const META_SUPPLEMENT: &str = "supplement";
pub const META_SUPPLEMENT_IDS: &str = "supplement_ids";
pub const META_SUPPLEMENT_COUNT: &str = "supplement_count";

/// 合并 n 条补充时正文之间的连接符。
///
/// 取空行（`\n\n`）而不是分隔线或编号：**不引入用户没写过的字符**。
const JOINER: &str = "\n\n";

/// 把一批收件箱条目合并成一条用户消息；空批次返回 `None`。
///
/// `n == 1` 原样返回该条目的消息（**与今天的行为逐字一致**，见模块头）。
pub fn merge_supplements(items: &[InboxItem]) -> Option<cm::ChatMessage> {
    match items {
        [] => None,
        // 一条不算"补充整合"：原样返回，不多加任何字段。
        [only] => Some(only.message.clone()),
        many => Some(merge_many(many)),
    }
}

fn merge_many(items: &[InboxItem]) -> cm::ChatMessage {
    cm::ChatMessage {
        // 地址末段即身份：沿用第一条的 id，前端的权威帧合并因此天然对齐
        // （`useChatConnection.ts`：后端沿用调用方给的 id，前端按 id 合并）。
        id: items[0].id.clone(),
        parent_id: None,
        role: Some(cm::MessageRole::User),
        msg_type: Some(cm::MessageType::Text),
        content: Some(merge_content(items)),
        status: Some(cm::MessageStatus::Completed),
        meta: Some(serde_json::json!({
            META_SUPPLEMENT: true,
            META_SUPPLEMENT_IDS: items.iter().map(|i| i.id.clone()).collect::<Vec<_>>(),
            META_SUPPLEMENT_COUNT: items.len(),
        })),
        ..Default::default()
    }
}

/// 按入队顺序拼接正文，**保持内容形态无损**。
///
/// 全部为纯文本 ⇒ 纯文本（最常见路径）；任一条带非文本片段 ⇒ `Parts`，
/// 让图片之类的内容按顺序保留，而不是被 `to_text()` 悄悄吃掉。
fn merge_content(items: &[InboxItem]) -> cm::MessageContent {
    let all_text = items
        .iter()
        .all(|i| matches!(i.message.content, Some(cm::MessageContent::Text(_)) | None));

    if all_text {
        let joined = items
            .iter()
            .map(|i| {
                i.message
                    .content
                    .as_ref()
                    .map(|c| c.to_text())
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join(JOINER);
        return cm::MessageContent::Text(joined);
    }

    let mut parts: Vec<cm::ContentPart> = Vec::new();
    for (idx, item) in items.iter().enumerate() {
        if idx > 0 {
            parts.push(cm::ContentPart::Text {
                text: JOINER.to_string(),
            });
        }
        match item.message.content.as_ref() {
            Some(cm::MessageContent::Text(s)) => {
                parts.push(cm::ContentPart::Text { text: s.clone() })
            }
            Some(cm::MessageContent::Parts(ps)) => parts.extend(ps.iter().cloned()),
            None => {}
        }
    }
    cm::MessageContent::Parts(parts)
}

#[cfg(test)]
#[path = "supplements.test.rs"]
mod tests;
