//! 消息变更的**读侧索引**（[ADR-053](../../../../docs/decisions/session.md)）。
//!
//! ## 它为什么存在
//!
//! 事实网格里除了「追加了什么」，还有「**改过什么**」：`message.edited`（这条现在
//! 读作 Y）与 `message.removed`（这一段没了）。它们既**不产 prompt entry**（它们不是
//! 新的对话内容），也**不是**「按设计不进请求包」的界面文本（那是汇报 / 首响）——
//! 它们是**第三类**：对既有内容的**改写**。
//!
//! 本模块把这两个格子的载荷收成一张可查的索引，供两个读侧出口共用：
//!
//! | 出口 | 怎么用它 |
//! |---|---|
//! | [`super::transcript`] | 条目按节点身份查：编辑 ⇒ 换正文；删除 ⇒ 丢条目 |
//! | [`super::prompt_fidelity`] | 同上——判据必须**看见**折叠，否则编辑过的正文会被报成「网格→prompt 丢事实」（事件里还是旧正文，prompt 里已是新正文） |
//!
//! ## 为什么必须是共享的
//!
//! 「怎么认出一次变更、它作用在哪个节点上」如果有两份实现，两份迟早给出不同的
//! 答案，而后果是**静默**的：一份折叠了、一份没折叠，表现为「prompt 与转写对同一条
//! 消息读法不同」——没有任何东西会红。
//!
//! ## 纯函数
//!
//! [`MessageMutations::collect`] 只读事件切片、不碰任何状态：同一份切片恒得同一份
//! 索引。这是投影纯函数性（N1）的一部分，也是「连读两次逐字相同」那条判据的前提。

use std::collections::{HashMap, HashSet};

use crate::symbio_core::event::{Entity, Event, Verb, EVENT_MESSAGE_EDITED, EVENT_MESSAGE_REMOVED};
use crate::symbio_core::schemas::session::chat_message::{ChatMessage, MessageContent};

/// 一条消息被编辑后**在对话里读作什么**。
///
/// 只留 `role` / `text` 两格而不是整条 [`ChatMessage`]：读侧（转写行、prompt 复核）
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
    /// 载荷不是合法 `ChatMessage`（旧版本写下的 / 手工改过的 WAL）⇒ 跳过那条：
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
    let m: ChatMessage = serde_json::from_value(e.payload.get("message")?.clone()).ok()?;
    if m.id.is_empty() {
        return None;
    }
    let role = m.role.as_ref().map(role_word);
    let text = m.content.as_ref().map(MessageContent::to_text);
    Some((m.id, EditedEntry { role, text }))
}

/// 角色 → 线格式词形（与 `MessageRole` 的 `serde(rename_all = "snake_case")` 逐字
/// 相等；`transcript` 的条目角色与 `PromptMessage::role` 用的是同一批词）。
fn role_word(r: &crate::symbio_core::schemas::session::chat_message::MessageRole) -> String {
    use crate::symbio_core::schemas::session::chat_message::MessageRole as R;
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

#[cfg(test)]
#[path = "mutations.test.rs"]
mod tests;
