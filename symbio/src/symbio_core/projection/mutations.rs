//! 消息列表变更的**读侧索引**（ADR-053）。
//!
//! ## 它为什么存在
//!
//! 事实网格里除了「追加了什么」，还有「**改过什么**」。三类变更各有各的折法：
//!
//! | 变更格 | 折法 | 出口 |
//! |---|---|---|
//! | `message.edited`（这条现在读作 Y） | 按节点身份换正文 / 角色 | [`MessageMutations`] |
//! | `message.removed`（这一段没了） | 按节点身份丢条目 | [`MessageMutations`] |
//! | `session.consolidated`（整段被摘要取代） | **重置**成快照那一份列表 | [`mutations_latest_snapshot`] |
//!
//! 前两类**不产 prompt entry**（它们不是新的对话内容），也**不是**「按设计不进请求包」
//! 的界面文本（那是汇报 / 首响）——它们是第三类：对既有内容的**改写**。
//!
//! ## 为什么必须是共享的
//!
//! 「怎么认出一次变更、它作用在哪个节点上」如果有两份实现，两份迟早给出不同的
//! 答案，而后果是**静默**的：一份折叠了、一份没折叠，表现为「prompt 与转写对同一条
//! 消息读法不同」——没有任何东西会红。
//!
//! ## 纯函数
//!
//! [`MessageMutations::collect`] 与 [`mutations_latest_snapshot`] 只读事件切片、不碰
//! 任何状态：同一份切片恒得同一份索引。这是投影纯函数性（N1）的一部分，也是
//! 「连读两次逐字相同」那条判据的前提。

use std::collections::{HashMap, HashSet};

use crate::symbio_core::event::{
    Entity, Event, Verb, EVENT_MESSAGE_EDITED, EVENT_MESSAGE_REMOVED, EVENT_SESSION_CONSOLIDATED,
};
use crate::symbio_core::schemas::session::chat_node::{ChatNode, MessageContent};

/// 一条消息被编辑后**在对话里读作什么**。
///
/// 只留 `role` / `text` 两格而不是整条 [`ChatNode`]：读侧（转写行、prompt 复核）
/// 只按这两格读对话。把整条图语义（`parent_id` / `tool_calls` / `seq`）搬进来，
/// core 就被某个 provider 的线格式绑住了——那条边界写在 `PromptMessage` 的文档里。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditedEntry {
    /// 载荷给了 `role` 才是 `Some`（补丁未提供 ⇒ 保持不变，与 `patch_message` 同义）。
    pub role: Option<String>,
    /// 载荷给了 `content` 才是 `Some`（同上）。
    pub text: Option<String>,
}

/// 一次切片里所有变更格折出来的索引。
#[derive(Debug, Clone, Default)]
pub struct MessageMutations {
    /// 节点 id → 编辑后的读法。
    edited: HashMap<String, EditedEntry>,
    /// 被删掉的节点 id（多次 `message.removed` 累加）。
    removed: HashSet<String>,
}

impl MessageMutations {
    /// 从事件切片收集（按事件顺序：同一节点被编辑多次 ⇒ **最后一条说了算**）。
    ///
    /// 载荷不是合法 `ChatNode`（旧版本写下的 / 手工改过的 WAL）⇒ 跳过那条：
    /// 读侧必须对任意历史事件容错，一条坏载荷不该让整次投影失败。
    pub fn collect(events: &[Event]) -> Self {
        let mut edited = HashMap::new();
        let mut removed = HashSet::new();
        for e in events {
            match (e.entity, e.verb, e.kind.as_str()) {
                (Entity::Message, Verb::Asserted, EVENT_MESSAGE_EDITED) => {
                    let Some(m) = parse_edited(e) else { continue };
                    edited.insert(m.0, m.1);
                }
                (Entity::Message, Verb::Closed, EVENT_MESSAGE_REMOVED) => {
                    removed.extend(ids_of(e));
                }
                _ => {}
            }
        }
        Self { edited, removed }
    }

    /// 该节点被编辑过吗？编辑后读作什么？
    pub fn edited(&self, node: &str) -> Option<&EditedEntry> {
        self.edited.get(node)
    }

    /// 该节点被删掉了吗？
    pub fn is_removed(&self, node: &str) -> bool {
        self.removed.contains(node)
    }
}

/// `message.edited` 的载荷 → `(节点 id, 读法)`。
fn parse_edited(e: &Event) -> Option<(String, EditedEntry)> {
    let m: ChatNode = serde_json::from_value(e.payload.get("message")?.clone()).ok()?;
    if m.id.is_empty() {
        return None;
    }
    let role = m.role.as_ref().map(role_word);
    let text = m.content.as_ref().map(MessageContent::to_text);
    Some((m.id, EditedEntry { role, text }))
}

/// 角色 → 线格式词形（与 `NodeRole` 的 `serde(rename_all = "snake_case")` 逐字
/// 相等；`transcript` 的条目角色与 `PromptMessage::role` 用的是同一批词）。
fn role_word(r: &crate::symbio_core::schemas::session::chat_node::NodeRole) -> String {
    use crate::symbio_core::schemas::session::chat_node::NodeRole as R;
    match r {
        R::User => "user",
        R::Assistant => "assistant",
        R::Tool => "tool",
        R::System => "system",
    }
    .to_string()
}

/// `message.removed` 的载荷 → 被删节点 id 列表。
fn ids_of(e: &Event) -> Vec<String> {
    e.payload
        .get("ids")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// **最近一次压缩快照**：`(快照事件在本切片里的下标, 压缩后的整份消息列表)`。
///
/// ## 为什么返回下标而不是 seq
///
/// 折方要问的是「从哪一条起接着折」——那是**切片内**的位置。用 seq 要求事件
/// 都带 seq（旧版本写下的 WAL 行 `seq: null`），而位置对任意切片都成立
/// （切片被裁过、手工改过也照折）。与 [`MessageMutations`] 的「先收索引、再投影」
/// 同一条纪律：读侧不得依赖呈现细节。
///
/// ## 为什么只认**最近**一次
///
/// 快照取代它之前的一切：更早的快照连它自己的消息列表都已被后来的快照取代。
/// 取最近一条即可，前面那几条的作用只剩「历史上有过几次压缩」（审计读归档段）。
///
/// 载荷不是合法列表（旧版本 / 手工改过的 WAL）⇒ **跳过那条**继续往前找：读侧必须
/// 对任意历史事件容错，一条坏载荷不该让整次投影失败——但也不该让一个坏快照
/// 把此前有效的那一份吃掉。
pub fn mutations_latest_snapshot(events: &[Event]) -> Option<(usize, Vec<ChatNode>)> {
    events
        .iter()
        .enumerate()
        .rev()
        .find_map(|(i, e)| snapshot_messages(e).map(|msgs| (i, msgs)))
}

/// `session.consolidated` 的载荷 → 压缩后的整份消息列表。
fn snapshot_messages(e: &Event) -> Option<Vec<ChatNode>> {
    if !(e.entity == Entity::Message && e.verb == Verb::Progressed) {
        return None;
    }
    if e.kind != EVENT_SESSION_CONSOLIDATED {
        return None;
    }
    let arr = e.payload.get("messages")?.as_array()?;
    // 逐条解：**一条坏条目不该吃掉整份快照**（与 `parse_edited` 同一条容错口径）。
    let msgs: Vec<ChatNode> = arr
        .iter()
        .filter_map(|v| serde_json::from_value::<ChatNode>(v.clone()).ok())
        .collect();
    // 空列表不是一份快照：压缩恒产出「至少那条摘要消息」。空数组若被当成有效快照，
    // 后果是**整段历史被抹掉**——容错的方向必须是「少折」而不是「多抹」。
    (!msgs.is_empty()).then_some(msgs)
}

#[cfg(test)]
#[path = "mutations.test.rs"]
mod tests;
