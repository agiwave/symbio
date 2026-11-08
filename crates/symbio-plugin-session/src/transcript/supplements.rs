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
//! | `parent_id` | `None` | 顶层：与用户直接发言同形——折进的是**输入**，不是某个回复的子树 |
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

use super::super::active::{ActiveSessionState, InboxItem};
use std::sync::Arc;
use symbio_sdk::chat::chat_node as cm;
use symbio_sdk::vdfs::host::VdfsChangeSubscriptions;

/// 合并标记的三个键名（唯一真源在 core 词汇表；重建侧与 loop 侧也读它们）。
pub use symbio_sdk::turn::facts::{META_SUPPLEMENT, META_SUPPLEMENT_COUNT, META_SUPPLEMENT_IDS};

/// 合并 n 条补充时正文之间的连接符。
///
/// 取空行（`\n\n`）而不是分隔线或编号：**不引入用户没写过的字符**。
const JOINER: &str = "\n\n";

/// 把一批收件箱条目合并成一条用户消息；空批次返回 `None`。
///
/// `n == 1` 原样返回该条目的消息（**与今天的行为逐字一致**，见模块头）。
pub fn merge_supplements(items: &[InboxItem]) -> Option<cm::ChatNode> {
    match items {
        [] => None,
        // 一条不算"补充整合"：原样返回，不多加任何字段。
        [only] => Some(only.message.clone()),
        many => Some(merge_many(many)),
    }
}

fn merge_many(items: &[InboxItem]) -> cm::ChatNode {
    cm::ChatNode {
        // 地址末段即身份：沿用第一条的 id，前端的权威帧合并因此天然对齐
        // （`useChatConnection.ts`：后端沿用调用方给的 id，前端按 id 合并）。
        id: items[0].id.clone(),
        parent_id: None,
        principal: Some(symbio_sdk::chat::principal::Principal::from_legacy(
            symbio_sdk::authz::PRINCIPAL_USER,
        )),
        msg_type: Some(cm::NodeKind::Body),
        content: Some(merge_content(items)),
        status: Some(cm::NodeStatus::Completed),
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

/// 补充整合在**批次边界**的落点：抽干整队并合并成一条补充（[D-086](../../../../docs/003-design/014-单环一表.md#d-086-loopsession-切片契约turndispatch-进程内交付turnspec-纯值面内部动作走地址面 "DE-014-单环一表") §6）。
///
/// ## 为什么住 session 而不随驱动器迁走
///
/// 抽干要读**取批那三样**（上界、队列、通知表，见
/// [`super::inbox::take_busy_supplement_batch_sync`]），它们全是会话状态；
/// 批次边界本身（工具结果落格）也在会话的**事实写口**上，两条判据因此都不需要
/// 驱动器伸手向会话要数据。
///
/// ## 它只做一件事
///
/// 取批 + 合并 + 落格都在本域内完成。落格由会话事实源在 `artifact.added`
/// （工具结果）之后调用 [`Self::land_merged_sync`]，经消息列表的**唯一写入口**
/// `apply` 落一条 `message.added`——合并消息因此必然排在工具结果**之后**
/// （写臂那一刻落格会插在 `tool_calls` 与结果之间，违反协议顺序）。
pub(crate) struct SupplementDrain {
    config: Arc<tokio::sync::RwLock<super::super::config::SessionConfig>>,
    change_subs: Arc<VdfsChangeSubscriptions>,
    state: Arc<ActiveSessionState>,
    /// 消息列表的唯一写入口 `apply` 需要它（落格不能只写事实，见 [`Self::land_merged_sync`]）。
    store: Arc<crate::store::SessionStore>,
}

impl SupplementDrain {
    pub(crate) fn new(
        config: Arc<tokio::sync::RwLock<super::super::config::SessionConfig>>,
        change_subs: Arc<VdfsChangeSubscriptions>,
        state: Arc<ActiveSessionState>,
        store: Arc<crate::store::SessionStore>,
    ) -> Self {
        Self {
            config,
            change_subs,
            state,
            store,
        }
    }

    /// 忙窗批次边界：抽干并合并成**一条**用户消息（**只取件、不落格**）。
    ///
    /// 只有本会话正在跑时才取（空闲那批归轮收尾的下一轮）。返回完整节点——
    /// 落格要的是 `message.added` 的载荷。
    ///
    /// ⚠️ **调用方负责 `block_in_place`**：委托的入口内部用 `blocking_read`，
    /// 单线程 runtime 的 async 上下文里直接调会 panic。
    fn take_merged_node_sync(&self) -> Option<cm::ChatNode> {
        let batch = super::inbox::take_busy_supplement_batch_sync(
            &self.config,
            &self.change_subs,
            &self.state,
        );
        merge_supplements(&batch)
    }

    /// **批次边界落格**（同步外壳）：取件、合并、经唯一写入口 `apply` 落一条
    /// `message.added`。
    ///
    /// 为什么走 `apply` 而不是直接写事实：消息列表的「事实 + `messages.json`」必须
    /// 由同一个入口一起折（LWF-001）；只落事实会让读缓存与重建结果分叉。
    ///
    /// ⚠️ **调用方负责 `block_in_place` 且当前必须是多线程 runtime**：取件用
    /// `blocking_write`，`apply` 经 `Handle::block_on` 走存储的异步写口。
    pub(crate) fn land_merged_sync(&self, handle: &tokio::runtime::Handle) {
        use crate::chat_session::{Carrier, MessageChange, PersistentChatSession};

        let Some(node) = self.take_merged_node_sync() else {
            return;
        };
        let session = PersistentChatSession::new(
            self.state.session_id.clone(),
            Arc::clone(&self.config),
            Arc::clone(&self.store),
        );
        let outcome = handle.block_on(session.apply(MessageChange::Born {
            nodes: vec![node],
            carrier: Carrier::Here,
        }));
        if let Err(why) = outcome {
            symbio_plugin_api::plugin_warn!(
                "session",
                "[补充] 合并消息落格失败（{why:?}；会话 {}）——条目已出队，轮收尾不会再抽到它",
                self.state.session_id
            );
        }
    }
}

#[cfg(test)]
#[path = "supplements.test.rs"]
mod tests;
