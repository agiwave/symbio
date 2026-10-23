//! 消息列表的**读侧**（D-053）——变更索引、压缩快照、行翻译的唯一读法。
//!
//! 本模块**不含 `Projection::new`**（是可共享的读侧普通函数，与 `rebuild`
//! / `turns` 同族）：投影 A 不得读投影 B 的输出（L2，DE-001 §13.2），
//! 而「变更怎么折、快照怎么读」是多个投影（`transcript` / `rebuild` / `turns`）与
//! 复核（`prompt_fidelity`）**共用**的读法。这条不变量由
//! `scripts/projection-purity.mjs` 静态守——**产投影的文件不得导入产投影的兄弟模块**，
//! 共享读法不产投影，因此谁都可以导它。
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

use crate::symbio_core::chat_node::{ChatNode, MessageContent, NodeRole};
use crate::symbio_core::event::{
    Entity, Event, Verb, EVENT_MESSAGE_EDITED, EVENT_MESSAGE_REMOVED, EVENT_SESSION_CONSOLIDATED,
};

use serde::{Deserialize, Serialize};

/// 转写条目：一句话及其角色（读侧**行类型**——`transcript` 折叠与
/// `prompt_fidelity` 复核共用的「一条消息被读作什么」）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptEntry {
    /// `"user"` / `"assistant"` / `"tool"`。
    pub role: String,
    pub text: String,
    /// 工具结果行的**工具名**（`role == "tool"` 时才有；其余角色为 `None`）。
    ///
    /// 单独成字段而不是拼进 `text`：工具名是**事实**（载荷里就有），怎么呈现
    /// （消息的 `tool` 字段）是消费方的事。`skip_serializing_if` 让
    /// 非工具行的线格式保持 `{role, text}` 不变——新增角色不改旧角色的形状。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// **按设计不进请求包**（界面文本：轮边界汇报 / `Escalate` 首响一类）。
    ///
    /// 为什么投影**收**它而 `to_messages()` **滤**它：用户看得见它、落库要留住它，
    /// 但它不是模型的对话内容——把它发进请求包会让线上出现连续两条 assistant
    /// （模型会以为那是它自己刚说的）。
    ///
    /// 线上格式保持不变（`skip_serializing_if`）：这个标记是**读侧的判别位**，
    /// 不是线格式的一部分——落库那份 `ChatNode` 上有它自己的 `meta` 字段
    /// （`exclude_from_context`），那份才是前端与轮次窗口读的东西。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub exclude_from_context: bool,
}

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
    let role = m.node_role().map(role_word);
    let text = m.content.as_ref().map(MessageContent::to_text);
    Some((m.id, EditedEntry { role, text }))
}

/// 角色 → 线格式词形（与 `NodeRole` 的 `serde(rename_all = "snake_case")` 逐字
/// 相等；`transcript` 的条目角色与 `PromptMessage::role` 用的是同一批词）。
fn role_word(r: NodeRole) -> String {
    match r {
        NodeRole::User => "user",
        NodeRole::Assistant => "assistant",
        NodeRole::Tool => "tool",
        NodeRole::System => "system",
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

/// 压缩快照的载荷（压缩后的整份消息列表）→ 转写条目（含节点身份）。
///
/// ## 为什么必须有且只有这一处翻译
///
/// 快照载荷是**图语义**（[`ChatNode`]：`parent_id` / `type` / 工具调用的父子），
/// 而转写是**扁平三角色**。两者之间必须翻译一次。若 `transcript` 与
/// `prompt_fidelity` 各译一遍，两份迟早对同一条给出两种读法——而那种分叉是
/// **静默**的（一份折了、一份没折，没有任何东西会红），正是本仓反复栽的那个坑。
///
/// ## 翻译规则（与事件投影**同源**）
///
/// | 载荷里的节点 | 条目 |
/// |---|---|
/// | `role=user`（快照摘要那条 / 折进的补充 / 普通发言） | `user` 行 |
/// | `role=assistant, type=text` | `assistant` 行（`meta.exclude_from_context` ⇒ 不进请求包） |
/// | `role=tool, type=text` | `tool` 行（工具名取**父 ToolCall 节点**的 `name`） |
/// | 其余（`Turn` / `ToolCall` / `Reasoning` 组合或非对话节点） | **不产行** |
///
/// 跳过 `Turn` / `ToolCall` / `Reasoning` **不是丢事实**：事件投影里它们同样不产行
/// ——收束格产的那条 `assistant` 行**就是**它的 Text 子节点，工具格产的那条 `tool`
/// 行**就是**结果子节点。组合节点只是承载结构。
pub(crate) fn snapshot_entries(msgs: &[ChatNode]) -> Vec<(String, TranscriptEntry)> {
    // 工具名在**父 ToolCall 节点**上：结果子节点只记 `parent_id`（事件投影里工具名
    // 来自 `artifact.added` 的载荷，两条路径取的是同一件事）。
    let tool_names: HashMap<&str, &str> = msgs
        .iter()
        .filter(|m| m.is_tool_call())
        .filter_map(|m| m.tool_name().map(|n| (m.id.as_str(), n)))
        .collect();
    msgs.iter()
        .filter_map(|m| {
            let role = m.node_role()?;
            let text = m
                .content
                .as_ref()
                .map(MessageContent::to_text)
                .unwrap_or_default();
            // 「文本型正文」= 显式 `Text` 或未标类型（历史数据）——谓词只看
            // `msg_type`，None 兼容在此显式表达。
            let textish = m.msg_type.is_none() || m.is_text();
            let entry = match role {
                NodeRole::User if textish => TranscriptEntry {
                    role: "user".into(),
                    text,
                    tool: None,
                    exclude_from_context: false,
                },
                NodeRole::Assistant if m.is_text() => TranscriptEntry {
                    role: "assistant".into(),
                    text,
                    tool: None,
                    exclude_from_context: meta_flag(m, "exclude_from_context"),
                },
                NodeRole::Tool if textish => TranscriptEntry {
                    role: "tool".into(),
                    text,
                    tool: m
                        .parent_id
                        .as_deref()
                        .and_then(|p| tool_names.get(p))
                        .map(|s| (*s).to_string()),
                    exclude_from_context: false,
                },
                _ => return None,
            };
            Some((m.id.clone(), entry))
        })
        .collect()
}

/// `meta` 里的布尔标记（缺失 / 非布尔一律 `false`——容错方向是「少标记」而不是
/// 「多标记」：多标一个 `exclude_from_context` 会让界面文本**静默**漏出请求包）。
fn meta_flag(m: &ChatNode, key: &str) -> bool {
    m.meta
        .as_ref()
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

#[cfg(test)]
#[path = "mutations.test.rs"]
mod tests;
