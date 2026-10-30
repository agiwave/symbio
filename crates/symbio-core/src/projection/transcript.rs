//! `transcript` 投影——从事件网格读出**对话转写**（会话运行时的历史侧）。
//!
//! [`crate::actors::Reasoner`] 组 prompt 的输入：多轮对话时模型
//! 需要看到自己说过什么（否则第二轮起失忆）。转写是**纯函数读视图**——
//! 历史来自同一份事实源，不另存副本（D-044：实测与判据同源的同族纪律）。
//!
// doc-link-allow D-004: 「四格 → 三角色」是事件形状到角色的静态映射，非版本改造
//! 口径（**四格 → 三角色**）：
//! - `user.message`（`turn × opened`）→ `user` 行（载荷 `text`）；
//! - `chat.assistant.final`（`turn × closed`）→ `assistant` 行（载荷 `text`）；
//! - `chat.assistant.fallback`（`turn × closed`）→ `assistant` 行（载荷 `detail`）——
//!   兜底话术是用户实际看到的回复，模型必须知道它「说」过这个；
//! - `artifact.added`（`artifact × asserted`）→ `tool` 行（载荷 `tool` + `text`）。
//!
//! 另有**两个变更格**（`message.edited` / `message.removed`）——它们**不产行**，
//! 而是改写 / 丢弃既有行；以及**一个取代格**（`session.consolidated`，压缩快照）——
//! 它也不产行，而是把此前整段历史**换基**成载荷里那份列表。三者都见 [`transcript`]
//! 的文档。
//!
//! ## 工具结果为什么也在转写里
//!
//! 工具轮**当轮**的交换由运行器就地追加（网格里还没有那部分历史，见
//! [`crate::actors::render_messages`] 的文档）；而**跨轮**的工具结果
//! 若不进投影，第二轮起的 prompt 就只剩「用户问过 X、助手答过 Y」——模型看不见
//! 自己调用过什么、拿到了什么，会**重复调用同一个工具**。所以 `artifact.added`
//! 进投影：多轮 prompt 因此能重建**含工具**的对话。
//!
//! 工具行投影成 `role: "tool"` 的**结构化消息**（带工具名与合成 `tool_call_id`，
//! 见 [`PromptMessage`]）——与轮内交换（`tool_exchange_messages`）**同一形态**，
//! 于是「历史里的工具轮」与「本轮的工具轮」在 prompt 里长得一样。

use super::super::event::{event_payload_node_id, event_tool_result_node_id, Event};
use super::vocab::{slot_node_of, slot_of, ChatSlot};
use super::Projection;
use crate::{ChatVocab, MessageVocab};

use serde::{Deserialize, Serialize};

// 行类型与快照读法住在**消息读侧**（`super::mutations`，D-053）：
// `transcript` 折叠与 `prompt_fidelity` 复核共用同一份「一条消息被读作什么」——
// 投影 A 不读投影 B（L2）：mutations 不含 `Projection::new`，是可共享的读侧普通函数
// （`projection-purity.mjs` 静态守：产投影的文件不得导入产投影的兄弟模块）。
use super::elision::{is_replacement, Elisions};
use super::mutations::{added_messages, snapshot_entries, OutputIndex, TranscriptEntry};

/// 转写视图：按事件顺序（append-only ⇒ 时间序）。
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TranscriptView {
    pub entries: Vec<TranscriptEntry>,
}

// 形状本身住内核（它是模型接缝的输入词汇，与 `TurnOutput` 同一类，D-255）；本模块只留
// **从转写行构造**它的转换——那是投影的知识，不是模型的词汇。
pub use symbio_kernel::llm::prompt_message::{PromptMessage, PromptToolCall};

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
    /// 投影成**消息数组**（D-048a 的结构化出口，**唯一出口**）。
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

/// `transcript` 投影：从事件切片读出对话转写。
///
/// ## 折叠：不只是追加（D-053）
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
/// | `session.consolidated` | **换基**：此前整段历史由载荷里那份列表取代 |
///
/// 同一趟折叠还认**分支格**（`turn.branch` / `turn.branch_selected`）：它们改的不是正文
/// 而是**读法**——不选中那一支的助手侧条目照样进转写（用户切回去还看得见），只是标成
/// 不进请求包。少了这一段，重试在模型眼里等于没发生：新旧两支的答复同时躺在请求包里，
/// 而下一轮读的还是被用户刚刚不要的那一份。
///
/// 以及**区间省略**（`message.added` 载荷的 `elision`，D-249）：一条事实同时说「这段
/// 历史被这份摘要取代」——载荷里的节点进转写且可见，被指的区间（按 seq 闭区间）照样进
/// 转写、只是标成不进请求包。分支与区间两种来源在 [`Elisions`] 上合流成**同一个判据**：
/// 「哪一段不读」只有一个答案，转写与 prompt 复核都用它。
///
/// ## 换基（压缩快照）
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
pub fn transcript(c: ChatVocab, m: MessageVocab) -> Projection<TranscriptView> {
    Projection::new(move |events: &[Event], now| {
        let mutations = super::mutations::MessageMutations::collect(events, &m);
        // 「这一段不读」的**唯一判据**（[`Elisions`]）：显式区间（`message.added` 载荷的
        // `elision`：这段被摘要取代）与未选中分支两种来源在同一个 `elides` 上合流。
        // 为什么必须合流见 `elision` 的模块文档——两个出口各判一遍，分叉是静默的。
        //
        // 未选中支的根 id（[D-173](../../../../docs/003-design/001-核心架构.md "DE-001-核心架构")）：
        // 同一个问题在请求包里只能有**一支**答复，否则模型读到的是自己没说过的话。
        // 这里问「还在不在」的证据取变更格（被删的支不在世），而不是「有没有节点」——
        // 重试那一幕的新支此刻还没有任何节点，而那正是该把旧支挤出请求包的时刻。
        // 两条问法与共享的挑法见 [`super::branch`] 的模块文档。
        let elisions = Elisions::collect(events, &c, &m, |id| mutations.is_removed(id));
        let mut entries: Vec<TranscriptEntry> = Vec::new();

        // ── 换基：先收下快照那份列表，再从其下标之后接着折（见函数文档）────────
        //
        // 快照事件自身也受 `now` 约束（as-of 读视图：晚于 `now` 的压缩在那一刻
        // 还没发生，历史就该按压缩**前**的样子读）。
        let (start, snapshot) = match super::mutations::mutations_latest_snapshot(events, &m) {
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
        // [`crate::event_payload_node_id`]）。它**不进线格式**——`TranscriptEntry`
        // 的字段一个没多，折叠键只在投影内部存在。
        //
        // **换基边界不重复产行**（`from_snapshot` 为假时才查 `superseded`）：自动压缩把
        // **本轮**的用户发言原文留在快照载荷尾部，而那条发言同时是本轮的开场格
        // （`turn × opened`，落在快照之后）。于是同一个节点被两条来源各产一次——每次压缩
        // 之后网格里多一条重复的用户发言。这是「两个出口对同一段历史给两种读法」那种
        // **静默**分叉：`rebuild` 的 `at` 先被快照播种、后来的同 id 事件不认（「同 id 只认
        // 首次」）；这里必须对齐那一步。
        //
        // 判据**只作用在换基边界上**（事实事件 ↔ 快照载荷之间），不在事实事件彼此之间：
        // 同一轮里两条不同事件可能撞到同一个合成 id，而它们在 `rebuild` 里是两个不同的
        // 节点——把它们去重就是另一种分叉。播种集合只装基线里那份，正是这个边界。
        //
        // 收束 / 进展格的内容节点身份取载荷 `answer_id`（引擎在落格前算定，与重建同源），
        // 缺失时按 `turn_id` 派生合成 id（见
        // [`crate::event_produced_text_node_id`]）。
        let produced_node = |e: &Event| slot_node_of(e, &c, &m).unwrap_or_default();
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

        // 收束 / 进展格的正文不在这两条事实里：它们只说结局，正文归那一次模型往返
        // （索引扫全量，快照之前的往返也要在里面——一轮可能跨压缩点）。
        let outputs = OutputIndex::collect(events);
        for e in &events[start..] {
            if e.ts > now {
                continue;
            }
            match slot_of(e, &c, &m) {
                // ── 变更格与取代格：**不产条目**——它们改的是既有条目（见函数文档）；
                //    压缩快照的那份列表已在上面换基收下，再产一次就是同一段历史读两遍。
                Some(
                    ChatSlot::MessageEdited | ChatSlot::MessageRemoved | ChatSlot::Consolidated,
                ) => {}
                Some(ChatSlot::UserMessage) => {
                    let text = e.payload.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    push(
                        TranscriptEntry {
                            role: "user".into(),
                            text: text.to_string(),
                            tool: None,
                            // 用户格不带 `turn_id` ⇒ 分支判据对它恒 false；但显式区间
                            // （摘要取代的那一段）照样适用——这正是「一个应用点」的意思。
                            exclude_from_context: elisions.elides(e),
                        },
                        event_payload_node_id(e, "id"),
                        false,
                    );
                }
                Some(ChatSlot::FinalReply) => {
                    let text = outputs.produced_text(e);
                    // 空正文的收束格不进转写：空行不是对话内容（空正文跳过，
                    // 与补充 / 汇报两条臂同一判据）。
                    if !text.trim().is_empty() {
                        push(
                            TranscriptEntry {
                                role: "assistant".into(),
                                text,
                                tool: None,
                                // 未选中支的正文**进转写、不进请求包**：切回那一支时用户
                                // 还看得见它，模型却同一时刻只该读一支。
                                exclude_from_context: elisions.elides(e),
                            },
                            produced_node(e),
                            false,
                        );
                    }
                }
                // 进展格（`turn × progressed`）：等待用户时模型说过的话——它是**助手
                // 说过的话**（role=assistant）且进请求包（下一轮模型看得见自己在等什么）。
                // 等待是事实的**状态**（载荷 `status: "pending"`），不是另一种事实：
                // kind 仍是对话事实网格里的进展格，这里照常投影。
                Some(ChatSlot::Progress) => {
                    let text = outputs.produced_text(e);
                    if !text.trim().is_empty() {
                        push(
                            TranscriptEntry {
                                role: "assistant".into(),
                                text,
                                tool: None,
                                exclude_from_context: elisions.elides(e),
                            },
                            produced_node(e),
                            false,
                        );
                    }
                }
                Some(ChatSlot::Fallback) => {
                    // 兜底那一行给用户看的就是失败原因本身（I3：到点必答——
                    // 失败也是一句话）。它读 `detail`（人话），**不是** `reason`
                    // （闭集）：闭集是给判据与统计的，读给用户的是原文。
                    let detail = e
                        .payload
                        .get("detail")
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    push(
                        TranscriptEntry {
                            role: "assistant".into(),
                            text: detail.to_string(),
                            tool: None,
                            exclude_from_context: elisions.elides(e),
                        },
                        produced_node(e),
                        false,
                    );
                }
                // 轮边界折进的补充：它**是用户说的话**，所以 role=user——
                // 不是为了「让它看起来像用户发言」，而是因为它字面上就是。
                //
                // ⚠️ 这条修的是**静默丢失**：补充进了 `context.messages`、会落库、
                // 会出现在 `messages.json` 里，而 prompt 从事实网格投影 ⇒ 模型看不见。
                // 落进投影之后它对下一轮可见，且**是这一轮**就该看见（不是下一轮）。
                Some(ChatSlot::Supplemented) => {
                    let text = e.payload.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    if !text.trim().is_empty() {
                        push(
                            TranscriptEntry {
                                role: "user".into(),
                                text: text.to_string(),
                                tool: None,
                                exclude_from_context: elisions.elides(e),
                            },
                            event_payload_node_id(e, "id"),
                            false,
                        );
                    }
                }
                // 轮边界汇报：它是**助手说的话**（role=assistant），但**不是**
                // 模型的对话内容——它是给人看的界面文本。
                //
                // 所以 `exclude_from_context: true` ⇒ 进转写（用户看得见、落库留得住）、
                // 不进 `to_messages()`（模型看不见，否则线上会出现连续两条 assistant）。
                //
                // ⚠️ 它与 `chat.assistant.final` **不是**同一格：那格是 N3「每轮至多一条」，
                // 而一轮可以汇报多次。挤进去会让「一轮一次最终答复」这条不变量失效。
                Some(ChatSlot::Reported) => {
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
                // 对话面开场白（`Escalate` 首响）：与轮边界汇报同一条纪律
                // ——助手说的界面文本，`exclude_from_context = true`（进转写、
                // 不进 `to_messages()`）。理由码不同只影响 `meta.reason`，转写这层
                // 只关心「谁说了一句给人看的话」。
                Some(ChatSlot::Opening) => {
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
                Some(ChatSlot::ArtifactAdded) => {
                    let tool = e.payload.get("tool").and_then(|v| v.as_str()).unwrap_or("");
                    let text = e.payload.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    push(
                        TranscriptEntry {
                            role: "tool".into(),
                            text: text.to_string(),
                            tool: Some(tool.to_string()),
                            exclude_from_context: elisions.elides(e),
                        },
                        event_tool_result_node_id(e),
                        false,
                    );
                }
                // 诞生格（`message × opened`，D-054 决策 1）：**没有别的格可搭车**的节点
                // 在这里出生。它有两副面孔，判别位是载荷带不带 `elision`
                // （[`is_replacement`]，D-249）：
                //
                // - **不带**（现状：压缩记录卡「已压缩上下文」）⇒ 节点是界面文本，
                //   `exclude_from_context` 恒为 `true`。让它进请求包会让模型看见一条
                //   没有对话语义的记录行。
                // - **带**（「这段历史被这份摘要取代」）⇒ 载荷里的节点就是**取代那段区间
                //   的正文**，可见性按节点自己的 `meta.exclude_from_context`（与压缩快照
                //   同一份翻译 [`snapshot_entries`]）；被省略区间的旧事件由
                //   [`Elisions::elides`] 判掉——同一条事实的两半，不各判一处。
                Some(ChatSlot::MessageAdded) => {
                    let replaces = is_replacement(e);
                    for (node, mut entry) in snapshot_entries(&added_messages(e)) {
                        entry.exclude_from_context = if replaces {
                            entry.exclude_from_context || elisions.elides(e)
                        } else {
                            true
                        };
                        push(entry, node, false);
                    }
                }
                None => {}
            }
        }
        TranscriptView { entries }
    })
}

#[cfg(test)]
#[path = "transcript.test.rs"]
mod tests;
