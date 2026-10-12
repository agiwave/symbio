//! `transcript` 投影——从事件网格读出**对话转写**（v2 会话运行时的历史侧）。
//!
//! [`crate::symbio_core::actors::Reasoner`] 组 prompt 的输入：多轮对话时模型
//! 需要看到自己说过什么（否则第二轮起失忆）。转写是**纯函数读视图**——
//! 历史来自同一份事实源，不另存副本（ADR-044：实测与判据同源的同族纪律）。
//!
//! 口径（**四格 → 三角色**）：
//! - `user.message`（`turn × opened`）→ `user` 行（载荷 `text`）；
//! - `chat.assistant.final`（`turn × closed`）→ `assistant` 行（载荷 `text`）；
//! - `chat.assistant.fallback`（`turn × closed`）→ `assistant` 行（载荷 `why`）——
//!   兜底话术是用户实际看到的回复，模型必须知道它「说」过这个；
//! - `artifact.added`（`artifact × asserted`）→ `tool` 行（载荷 `tool` + `text`）。
//!
//! 另有**两个变更格**（`message.edited` / `message.removed`）——它们**不产行**，
//! 而是改写 / 丢弃既有行；以及**一个取代格**（`session.consolidated`，压缩快照）——
//! 它也不产行，而是把此前整段历史**换基**成载荷里那份列表。三者都见 [`transcript`]
//! 的文档。
//!
//! ## 工具结果为什么也在转写里（[plan/10 批 3](../../../../docs/plan/PL-008-工具轮v2化实施方案.md "PL-008-工具轮v2化实施方案")）
//!
//! 工具轮**当轮**的交换由运行器就地追加（网格里还没有那部分历史，见
//! [`crate::symbio_core::actors::render_messages`] 的文档）；而**跨轮**的工具结果
//! 若不进投影，第二轮起的 prompt 就只剩「用户问过 X、助手答过 Y」——模型看不见
//! 自己调用过什么、拿到了什么，会**重复调用同一个工具**。所以 `artifact.added`
//! 进投影：多轮 prompt 因此能重建**含工具**的对话。
//!
//! 工具行投影成 `role: "tool"` 的**结构化消息**（带工具名与合成 `tool_call_id`，
//! 见 [`PromptMessage`]）——与轮内交换（`tool_exchange_messages`）**同一形态**，
//! 于是「历史里的工具轮」与「本轮的工具轮」在 prompt 里长得一样。

use super::super::event::{
    event_payload_node_id, event_produced_text_node_id, event_tool_result_node_id, Entity, Event,
    Verb, EVENT_ARTIFACT_ADDED, EVENT_ASSISTANT_FALLBACK, EVENT_ASSISTANT_FINAL,
    EVENT_ASSISTANT_OPENING, EVENT_ASSISTANT_PROGRESS, EVENT_ASSISTANT_REPORTED,
    EVENT_MESSAGE_ADDED, EVENT_MESSAGE_EDITED, EVENT_MESSAGE_REMOVED, EVENT_SESSION_CONSOLIDATED,
    EVENT_TURN_SUPPLEMENTED, EVENT_USER_MESSAGE,
};
use super::super::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageType,
};
use super::super::view::{Budget, View};
use super::Projection;

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// 转写条目：一句话及其角色。
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
    /// 线上格式保持不变（`skip_serializing_if`）：这个标记是**投影内部的判别位**，
    /// 不是线格式的一部分——落库那份 `ChatMessage` 上有它自己的 `meta` 字段
    /// （`exclude_from_context`），那份才是前端与轮次窗口读的东西。
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub exclude_from_context: bool,
}

/// 转写视图：按事件顺序（append-only ⇒ 时间序）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TranscriptView {
    pub entries: Vec<TranscriptEntry>,
}

/// 一条**结构化**的 prompt 消息：角色与正文分开，工具关联单独成字段。
///
/// ## 为什么 core 自己定义这个类型，而不是直接用 provider 的 `ChatMessage`
///
/// `ChatMessage` 是**图语义**（有 `id` / `tool_calls` / `status` / 帧类型），
/// 住在 `plugins/session`；core 不认识那套（见 `DispatchOutcome` 的文档：core 只
/// 认识「拿什么回填给模型」这段文本）。若 core 的出口就是 `ChatMessage`，core 就
/// 被某个 provider 的线格式绑住了。
///
/// 所以 core 出**最小结构化形态**，由 adapter 边界翻译成 provider 的消息
/// （`ProviderLlmAdapter` 原样交给 `execute_turn`）。**结构在 core、线格式在
/// adapter**——与 ADR-043 的域边界一致。
///
/// ## 为什么必须有它（ADR-048a）
///
/// `ModelProvider::execute_turn` 的签名本身就是
/// `(system_prompt, messages: &[ChatMessage], …)`——`role` + `tool_call_id` 是**跨栈契约**
/// 不是排版。把消息拍平成一段文本会把 `role: tool` 降级成 user 消息里的纯文本，模型就
/// 看不到「这是工具结果」、也拿不到 `tool_call_id`——那是已判定的退步。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptMessage {
    /// `"user"` / `"assistant"` / `"tool"`。
    pub role: String,
    pub text: String,
    /// 工具消息关联的**调用 id**（`role == "tool"` 时有；其余为 `None`）。
    ///
    /// **缺失时的行为是已定的**，不是待办：模型侧协议要求 `tool` 消息带
    /// `tool_call_id`，而历史里的工具格没有调用 id（`artifact.added` 载荷只记了
    /// 工具名与正文——那**就是**网格里存在的事实）。所以 adapter 边界按工具名合成
    /// 一个**稳定 id**（见 `provider_adapter`），而不是让消息裸奔——裸奔会让
    /// 「拒绝未知 tool_call_id」的 provider 整条请求失败，那比拍平成一条 user 消息
    /// **更糟**（拍平至少还能答）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    /// 工具名（`role == "tool"` 时有；其余为 `None`）。是**事实**（载荷里就有）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    /// 本条消息声明的**工具调用**（`role == "assistant"` 且模型请求了工具时）。
    ///
    /// ## 为什么不能只靠正文说「调用工具 x」
    ///
    /// 请求包清洗会**丢弃「无对应 `tool_call` 的 tool 结果」**——provider 要求
    /// `role: tool` 消息的 `tool_call_id` 必须匹配前文某条 assistant 的
    /// `tool_calls`（`message_builder::flatten_chat_messages` 末尾的清洗段，
    /// 理由是「否则直接 400」）。而若 assistant 那条只把调用写在**正文**里
    /// （"调用工具 vdfs_read {...}"），它在协议层就不是一条带 `tool_calls` 的
    /// assistant 消息，于是**所有 tool 结果都成了孤儿**并被清掉——模型永远收不到
    /// 工具结果，于是**无限工具循环**。
    ///
    /// 这个坑是实测撞出来的（9905 次请求、CLI 撞 120s 超时），而症状（无限循环）
    /// 指不到「请求包清洗」这一层。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<PromptToolCall>>,
}

/// 一次工具调用的声明（core 侧的最小形态；线格式由 adapter 翻译）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptToolCall {
    /// 调用 id（`role: tool` 消息用它回指本次调用）。
    pub id: String,
    pub name: String,
    /// 参数（**结构化**，不是拼进正文的字符串）。
    pub arguments: serde_json::Value,
}

impl From<&TranscriptEntry> for PromptMessage {
    fn from(e: &TranscriptEntry) -> Self {
        // `role == "tool"` **必须**带 `tool_call_id`：模型侧协议要求 tool 消息
        // 关联到具体那次调用，缺了会被 provider 整条拒绝——那比拍平成 user 消息
        // **更糟**（拍平至少还能答）。
        //
        // 网格里的 `artifact.added` 没有调用 id（那**才是事实**：调用 id 是本次运行的
        // 句柄，不是跨轮事实）。所以按工具名合成一个**稳定 id**：
        // - 稳定 = 同一格每次投影出同一个 id，跨轮不会漂；
        // - 按名字而不是随机 = 同一工具的多次调用得到同一个 id，于是**诚实**地
        //   表示「无法区分是第几次」——而虚假的唯一 id 会让模型以为它能对上，
        //   配错时产生**看起来正确**的错误。
        let tool_call_id = e.tool.as_deref().map(|tool| format!("call_{tool}_0"));
        PromptMessage {
            role: e.role.clone(),
            text: e.text.clone(),
            tool_call_id,
            tool: e.tool.clone(),
            tool_calls: None,
        }
    }
}

impl TranscriptView {
    /// 投影成**消息数组**（ADR-048a 的结构化出口，**唯一出口**）。
    ///
    /// 不裁剪、不改写、不补分隔符：**它就是网格里的那些条**，按事件顺序。
    /// 任何「为了 prompt 好看」的加工都会让模型看到网格没有的东西——那正是
    /// `prompt_fidelity` 要报的「两条真源」。
    pub fn to_messages(&self) -> Vec<PromptMessage> {
        self.entries
            .iter()
            // 界面文本（汇报 / 首响）**不进请求包**——它们是给人看的，模型要是看见了
            // 会当成「我刚说过这句」，于是线上出现连续两条 assistant。滤在这里而不是
            // 在投影里不收，是因为落库与前端仍需要它们（用户得看得见自己被汇报了）。
            .filter(|e| !e.exclude_from_context)
            .map(PromptMessage::from)
            .collect()
    }

    /// 视图里**按设计不进请求包**的条目（诊断 / 判据用）。
    ///
    /// 为什么要有这个出口：过滤之后「它们去哪了」就只剩投影实现这一处内部知识——
    /// 测试与门禁无法从外面验证「汇报真的落了格、且真的没进请求包」。
    pub fn excluded(&self) -> Vec<&TranscriptEntry> {
        self.entries
            .iter()
            .filter(|e| e.exclude_from_context)
            .collect()
    }
}

/// 压缩快照的载荷（压缩后的整份消息列表）→ 转写条目（含节点身份）。
///
/// ## 为什么必须有且只有这一处翻译
///
/// 快照载荷是**图语义**（[`ChatMessage`]：`parent_id` / `type` / 工具调用的父子），
/// 而转写是**扁平三角色**。两者之间必须翻译一次。若 [`transcript`] 与
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
pub(crate) fn snapshot_entries(msgs: &[ChatMessage]) -> Vec<(String, TranscriptEntry)> {
    // 工具名在**父 ToolCall 节点**上：结果子节点只记 `parent_id`（事件投影里工具名
    // 来自 `artifact.added` 的载荷，两条路径取的是同一件事）。
    let tool_names: HashMap<&str, &str> = msgs
        .iter()
        .filter(|m| m.msg_type == Some(MessageType::ToolCall))
        .filter_map(|m| m.name.as_deref().map(|n| (m.id.as_str(), n)))
        .collect();
    msgs.iter()
        .filter_map(|m| {
            let role = m.role.as_ref()?;
            let text = m
                .content
                .as_ref()
                .map(MessageContent::to_text)
                .unwrap_or_default();
            let entry = match (role, &m.msg_type) {
                (MessageRole::User, None | Some(MessageType::Text)) => TranscriptEntry {
                    role: "user".into(),
                    text,
                    tool: None,
                    exclude_from_context: false,
                },
                (MessageRole::Assistant, Some(MessageType::Text)) => TranscriptEntry {
                    role: "assistant".into(),
                    text,
                    tool: None,
                    exclude_from_context: meta_flag(m, "exclude_from_context"),
                },
                (MessageRole::Tool, None | Some(MessageType::Text)) => TranscriptEntry {
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

/// 诞生格（`message.added`）的载荷 → 节点列表（载荷 `{ messages: [...] }`）。
///
/// 与 `snapshot_messages` 同一条容错口径：**一条坏条目不吃掉整批**——逐条解，
/// 解不出就跳过。空列表不是一次诞生：落一条空事实等于在事实源上留下「什么都没发生」
/// 的痕迹。
fn added_messages(e: &Event) -> Vec<ChatMessage> {
    e.payload
        .get("messages")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| serde_json::from_value::<ChatMessage>(v.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// `meta` 里的布尔标记（缺失 / 非布尔一律 `false`——容错方向是「少标记」而不是
/// 「多标记」：多标一个 `exclude_from_context` 会让界面文本**静默**漏出请求包）。
fn meta_flag(m: &ChatMessage, key: &str) -> bool {
    m.meta
        .as_ref()
        .and_then(|v| v.get(key))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// `transcript` 投影：从事件切片读出对话转写。
///
/// ## 折叠：不只是追加（S2.7-b1，ADR-053）
///
/// 事实网格里除了「追加了什么」，还有「**改过什么**」——`message.edited` /
/// `message.removed`。少了这一段，一次截断在 prompt 面上等于**没发生过**：模型
/// 下一轮仍看得见被删掉的那几句；编辑过的发言也仍以**旧**正文出现（prompt 从
/// 事实网格投影，而网格里那条事件还是旧的）。
///
/// | 变更格 | 折叠动作 |
/// |---|---|
/// | `message.edited` | 按节点身份**换正文 / 角色**（位置不变） |
/// | `message.removed` | 按节点身份**丢条目** |
/// | `session.consolidated` | **换基**：此前整段历史由载荷里那份列表取代（S2.7-b2） |
///
/// ## 换基（压缩快照，S2.7-b2）
///
/// `session.consolidated` 的载荷是**压缩后的整份消息列表**。折叠动作是：先把它
/// 经 [`snapshot_entries`] 收成基线，再从它**下标之后**接着折事实；它之前的事件
/// 一律不参与。少了这一段，一次压缩在 prompt 面上等于**没发生过**——被压掉的
/// 历史会全部复活，而压缩的全部意义就是让它们不再进入上下文。
///
/// **只认最近一次**：更早的快照连它自己的列表都已被后来的快照取代。
///
/// ## 折叠是「先收索引、再投影」，不是「边投影边回头改」
///
/// 变更格在事件序列里排在它改的那条**之后**（先有事实，才有「它变了」），所以一趟
/// 顺序回放没法在处理原事件时就知道它后来被改了。做法是先把两个变更格收成一份索引
/// （[`super::mutations::MessageMutations::collect`]），再投影——每条条目**出生时就
/// 带着最终读法**，于是折叠与事件顺序无关（切片被裁过、手工改过也不会漏），也不
/// 需要「先 push 再回头改」那套下标维护。快照同理：先定位它，再从它之后折。
///
/// 索引本体与 `prompt_fidelity` 的复核**共用同一份**：两个出口因此不可能对同一条
/// 消息给出两种读法（而那种分叉是静默的）。
pub fn transcript() -> Projection<TranscriptView> {
    Projection::new(|events: &[Event], now, _budget: Budget| {
        let mutations = super::mutations::MessageMutations::collect(events);
        let mut entries: Vec<TranscriptEntry> = Vec::new();

        // ── 换基：先收下快照那份列表，再从其下标之后接着折（见函数文档）────────
        //
        // 快照事件自身也受 `now` 约束（as-of 读视图：晚于 `now` 的压缩在那一刻
        // 还没发生，历史就该按压缩**前**的样子读）。
        let (start, snapshot) = match super::mutations::mutations_latest_snapshot(events) {
            Some((i, msgs)) if events[i].ts <= now => (i + 1, msgs),
            _ => (0, Vec::new()),
        };
        let baseline = snapshot_entries(&snapshot);
        // 基线里已经占了的节点身份：快照之后再来同 id 的事实不再产第二行。见 `push`。
        let superseded: std::collections::HashSet<String> =
            baseline.iter().map(|(node, _)| node.clone()).collect();

        // 收一条条目：**出生时就按索引定稿**（被删的不收；被编辑过的用新读法）。
        //
        // `node` = 该条目对应的节点身份（与 `rebuild` / 变更格同一套 id，见
        // [`crate::symbio_core::event_payload_node_id`]）。它**不进线格式**——`TranscriptEntry`
        // 的字段一个没多，折叠键只在投影内部存在。
        //
        // **换基边界不重复产行**（`from_snapshot` 为假时才查 `superseded`）：自动压缩把
        // **本轮**的用户发言原文留在快照载荷尾部，而那条发言同时是本轮的开场格
        // （`turn × opened`，落在快照之后）。于是同一个节点被两条来源各产一次——每次压缩
        // 之后网格里多一条重复的用户发言。这是「两个出口对同一段历史给两种读法」那种
        // **静默**分叉：`rebuild` 的 `at` 先被快照播种、后来的同 id 事件不认（「同 id 只认
        // 首次」），而这里此前没有那一步。
        //
        // 判据**只作用在换基边界上**（事实事件 ↔ 快照载荷之间），不在事实事件彼此之间：
        // 兜底 / 汇报那几格的节点身份是按 `turn_id` 派生的合成 id（见
        // [`crate::symbio_core::event_produced_text_node_id`]），同一轮里两条不同事件可能
        // 撞到同一个合成 id，而它们在 `rebuild` 里是两个不同的节点——把它们去重就是另一种
        // 分叉。播种集合只装基线里那份，正是这个边界。
        let mut push = |mut entry: TranscriptEntry, node: String, from_snapshot: bool| {
            if mutations.is_removed(&node) {
                return;
            }
            if !from_snapshot && superseded.contains(&node) {
                return;
            }
            if let Some(ed) = mutations.edited(&node) {
                if let Some(r) = &ed.role {
                    entry.role = r.clone();
                }
                if let Some(t) = &ed.text {
                    entry.text = t.clone();
                }
            }
            entries.push(entry);
        };

        for (node, entry) in baseline {
            push(entry, node, true);
        }

        for e in &events[start..] {
            if e.ts > now {
                continue;
            }
            match (e.entity, e.verb, e.kind.as_str()) {
                // ── 变更格：**不产条目**——它们改的是既有条目（见函数文档）──────
                (Entity::Message, Verb::Asserted, EVENT_MESSAGE_EDITED)
                | (Entity::Message, Verb::Closed, EVENT_MESSAGE_REMOVED) => {}
                // ── 取代格（压缩快照）：**不产条目**——它的那份列表已在上面换基
                //    收下（见函数文档「换基」）。在这里再产一次就是同一段历史读两遍。
                (Entity::Message, Verb::Progressed, EVENT_SESSION_CONSOLIDATED) => {}
                (Entity::Turn, Verb::Opened, EVENT_USER_MESSAGE) => {
                    let text = e.payload.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    push(
                        TranscriptEntry {
                            role: "user".into(),
                            text: text.to_string(),
                            tool: None,
                            exclude_from_context: false,
                        },
                        event_payload_node_id(e, "id"),
                        false,
                    );
                }
                (Entity::Turn, Verb::Closed, EVENT_ASSISTANT_FINAL) => {
                    let text = e.payload.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    // 空正文的收束格不进转写：空行不是对话内容（空正文跳过，
                    // 与补充 / 汇报两条臂同一判据）。
                    if !text.trim().is_empty() {
                        push(
                            TranscriptEntry {
                                role: "assistant".into(),
                                text: text.to_string(),
                                tool: None,
                                exclude_from_context: false,
                            },
                            event_produced_text_node_id(e),
                            false,
                        );
                    }
                }
                // 进展格（`turn × progressed`）：等待用户时模型说过的话——它是**助手
                // 说过的话**（role=assistant）且进请求包（下一轮模型看得见自己在等什么）。
                // 等待是事实的**状态**（载荷 `status: "pending"`），不是另一种事实：
                // kind 仍是对话事实网格里的进展格，这里照常投影。
                (Entity::Turn, Verb::Progressed, EVENT_ASSISTANT_PROGRESS) => {
                    let text = e.payload.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    if !text.trim().is_empty() {
                        push(
                            TranscriptEntry {
                                role: "assistant".into(),
                                text: text.to_string(),
                                tool: None,
                                exclude_from_context: false,
                            },
                            event_produced_text_node_id(e),
                            false,
                        );
                    }
                }
                (Entity::Turn, Verb::Closed, EVENT_ASSISTANT_FALLBACK) => {
                    let why = e.payload.get("why").and_then(|v| v.as_str()).unwrap_or("");
                    push(
                        TranscriptEntry {
                            role: "assistant".into(),
                            text: why.to_string(),
                            tool: None,
                            exclude_from_context: false,
                        },
                        event_produced_text_node_id(e),
                        false,
                    );
                }
                // 轮边界折进的补充（缺口 3）：它**是用户说的话**，所以 role=user——
                // 不是为了「让它看起来像用户发言」，而是因为它字面上就是。
                //
                // ⚠️ 这条修的是**静默丢失**：补充进了 `context.messages`、会落库、
                // 会出现在 `messages.json` 里，而 prompt 从事实网格投影 ⇒ 模型看不见。
                // 落进投影之后它对下一轮可见，且**是这一轮**就该看见（不是下一轮）。
                (Entity::Turn, Verb::Asserted, EVENT_TURN_SUPPLEMENTED) => {
                    let text = e.payload.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    if !text.trim().is_empty() {
                        push(
                            TranscriptEntry {
                                role: "user".into(),
                                text: text.to_string(),
                                tool: None,
                                exclude_from_context: false,
                            },
                            event_payload_node_id(e, "id"),
                            false,
                        );
                    }
                }
                // 轮边界汇报（缺口 4）：它是**助手说的话**（role=assistant），但**不是**
                // 模型的对话内容——它是给人看的界面文本。
                //
                // 所以 `exclude_from_context: true` ⇒ 进转写（用户看得见、落库留得住）、
                // 不进 `to_messages()`（模型看不见，否则线上会出现连续两条 assistant）。
                //
                // ⚠️ 它与 `chat.assistant.final` **不是**同一格：那格是 N3「每轮至多一条」，
                // 而一轮可以汇报多次。挤进去会让「一轮一次最终答复」这条不变量失效。
                (Entity::Turn, Verb::Asserted, EVENT_ASSISTANT_REPORTED) => {
                    let text = e.payload.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    if !text.trim().is_empty() {
                        push(
                            TranscriptEntry {
                                role: "assistant".into(),
                                text: text.to_string(),
                                tool: None,
                                exclude_from_context: true,
                            },
                            event_payload_node_id(e, "id"),
                            false,
                        );
                    }
                }
                // 对话面开场白（T-056，`Escalate` 首响）：与轮边界汇报同一条纪律
                // ——助手说的界面文本，`exclude_from_context = true`（进转写、
                // 不进 `to_messages()`）。理由码不同只影响 `meta.reason`，转写这层
                // 只关心「谁说了一句给人看的话」。
                (Entity::Turn, Verb::Asserted, EVENT_ASSISTANT_OPENING) => {
                    let text = e.payload.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    if !text.trim().is_empty() {
                        push(
                            TranscriptEntry {
                                role: "assistant".into(),
                                text: text.to_string(),
                                tool: None,
                                exclude_from_context: true,
                            },
                            event_payload_node_id(e, "id"),
                            false,
                        );
                    }
                }
                // 工具结果（`artifact × asserted`）：跨轮可见的「我调用过什么、拿到了什么」。
                (Entity::Artifact, Verb::Asserted, EVENT_ARTIFACT_ADDED) => {
                    let tool = e.payload.get("tool").and_then(|v| v.as_str()).unwrap_or("");
                    let text = e.payload.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    push(
                        TranscriptEntry {
                            role: "tool".into(),
                            text: text.to_string(),
                            tool: Some(tool.to_string()),
                            exclude_from_context: false,
                        },
                        event_tool_result_node_id(e),
                        false,
                    );
                }
                // 诞生格（`message × opened`，ADR-054
                // 决策 1）：**没有别的格可搭车**的节点在这里出生——今天只有压缩记录节点
                // （「已压缩上下文」那张卡）。
                //
                // `exclude_from_context` **恒为 `true`**：本格的节点按定义就是界面文本
                // （见 `EVENT_MESSAGE_ADDED` 的边界——承载对话内容的节点必须由它自己的
                // 事实落）。让它进请求包会让模型看见一条没有对话语义的记录行。
                (Entity::Message, Verb::Opened, EVENT_MESSAGE_ADDED) => {
                    for (node, mut entry) in snapshot_entries(&added_messages(e)) {
                        entry.exclude_from_context = true;
                        push(entry, node, false);
                    }
                }
                _ => {}
            }
        }
        View {
            value: TranscriptView { entries },
            degraded: false,
            used: Budget::new(0, 0),
        }
    })
}

#[cfg(test)]
#[path = "transcript.test.rs"]
mod tests;
