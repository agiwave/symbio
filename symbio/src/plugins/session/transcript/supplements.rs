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
use crate::symbio_core::chat_node as cm;
use crate::symbio_core::VdfsChangeSubscriptions;
use std::sync::Arc;

/// 合并标记的三个键名（唯一真源在 core 词汇表；重建侧与 loop 侧也读它们）。
pub(crate) use crate::symbio_core::{META_SUPPLEMENT, META_SUPPLEMENT_COUNT, META_SUPPLEMENT_IDS};

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
        role: Some(cm::NodeRole::User),
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

/// 抽干结果（类型已随 D-086 §7 上移 core：loop 的补充注入与 session 的抽干口
/// 都要命名它；这里只留 session 侧的短路径别名）。
pub(crate) use crate::symbio_core::DrainedSupplement;

/// 「取批 + 合并 + 成形」的**同步单一入口**（[D-081](../../../../../docs/003-design/014-单环一表.md "D-081") 裁决三）。
///
/// 补充消息的**到达**归存储（队列与变更通知都在本域），驱动器只掌握**要的时机**
/// （工具循环的批次边界）——于是这三步整体住在本域，`chat_loop::SupplementDrain`
/// 不再在驱动器侧自拼两步、手写成形段，退化为「持着取批那三样、调一下」。
///
/// 判定与取件逻辑不复制：内部走 [`super::inbox::take_inbox_batch_sync`]
/// （`supplements_enabled = false` 时它退化为"取一条"，与 [`merge_supplements`] 叠加后
/// 恰好等于"一条消息 = 一轮"的今天行为）。
///
/// 返回 `None` = 队列为空（或合并结果为空批次）——调用方据此**什么都不做**，
/// 不推进 `last_saved`、不落库。
///
/// ⚠️ **调用方负责 `block_in_place`**：本函数内部用 `blocking_read`，若在
/// 单线程 runtime 的 async 上下文里直接调会 panic。`turn_exec` 的注入闭包里有
/// flavor 判定（多线程才 `block_in_place`，否则裸调）。
pub(crate) fn take_supplement_batch_sync(
    config: &tokio::sync::RwLock<super::super::config::SessionConfig>,
    change_subs: &VdfsChangeSubscriptions,
    state: &Arc<ActiveSessionState>,
) -> Option<DrainedSupplement> {
    let batch = super::inbox::take_inbox_batch_sync(config, change_subs, state);
    let merged = merge_supplements(&batch)?;
    let count = merged
        .meta
        .as_ref()
        .and_then(|m| m.get(META_SUPPLEMENT_COUNT))
        .and_then(|v| v.as_u64())
        .unwrap_or(1);
    Some(DrainedSupplement {
        text: merged
            .content
            .as_ref()
            .map(|c| c.to_text())
            .unwrap_or_default(),
        count,
        id: merged.id,
        ids: merged
            .meta
            .as_ref()
            .and_then(|m| m.get(META_SUPPLEMENT_IDS))
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default(),
    })
}

/// 补充整合在**批次边界**的落点：抽干整队并合并成一条补充（[D-086](../../../../../docs/003-design/014-单环一表.md "D-086") §6）。
///
/// ## 为什么住 session 而不随驱动器迁走
///
/// 抽干要读**取批那三样**（上界、队列、通知表，见 [`take_supplement_batch_sync`]），
/// 它们全是会话状态；驱动器经 `session/<sid>/_run/take-supplements` 动作使用它。
///
/// ## 它只做一件事
///
/// 委托本域的单一入口 [`take_supplement_batch_sync`]（取批 + 合并 + 成形三步整体
/// 住在本域，见 [D-081](../../../../../docs/003-design/014-单环一表.md "D-081") 裁决三），
/// **不在驱动器侧自拼步骤、不写任何地址、不落格、不落库**：抽干发生在工具循环的
/// **批次边界**，落格与「进下一次请求」都在 loop 的 `turn_exec.rs` 的
/// `supplement_into_request`（由 `turn_runner.rs` 的 2f 触发）。
pub(crate) struct SupplementDrain {
    config: Arc<tokio::sync::RwLock<super::super::config::SessionConfig>>,
    change_subs: Arc<VdfsChangeSubscriptions>,
    state: Arc<ActiveSessionState>,
}

impl SupplementDrain {
    pub(crate) fn new(
        config: Arc<tokio::sync::RwLock<super::super::config::SessionConfig>>,
        change_subs: Arc<VdfsChangeSubscriptions>,
        state: Arc<ActiveSessionState>,
    ) -> Self {
        Self {
            config,
            change_subs,
            state,
        }
    }

    /// 抽干该会话队首的一批补充并合并成**一条**用户消息（缺口 3：v2 工具循环里的
    /// 注入挂点）。
    ///
    /// ⚠️ **调用方负责 `block_in_place`**：委托的入口内部用 `blocking_read`，若在
    /// 单线程 runtime 的 async 上下文里直接调会 panic。动作面（`run_actions`）与
    /// loop 的注入闭包都有 flavor 判定（多线程才 `block_in_place`，否则裸调）。
    pub(crate) fn take_inbox_batch_sync(&self) -> Option<DrainedSupplement> {
        take_supplement_batch_sync(&self.config, &self.change_subs, &self.state)
    }
}

#[cfg(test)]
#[path = "supplements.test.rs"]
mod tests;
