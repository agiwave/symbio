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
//! ## 工具结果为什么也在转写里（[plan/10 批 3](../../../../docs/plan/10-工具轮v2化实施方案.md)）
//!
//! 工具轮**当轮**的交换由运行器就地追加（`render_tool_exchange`：网格里还没有
//! 那部分历史，见 `actors::Reasoner::render_prompt` 的文档）；而**跨轮**的工具结果
//! 若不进投影，第二轮起的 prompt 就只剩「用户问过 X、助手答过 Y」——模型看不见
//! 自己调用过什么、拿到了什么，会**重复调用同一个工具**。所以 `artifact.added`
//! 进投影：多轮 prompt 因此能重建**含工具**的对话。
//!
//! 工具行渲染成 `工具结果(<tool>): <text>`——与轮内交换（`render_tool_exchange`）
//! **同一形态**，于是「历史里的工具轮」与「本轮的工具轮」在 prompt 里长得一样。

use super::super::event::{
    Entity, Event, Verb, EVENT_ARTIFACT_ADDED, EVENT_ASSISTANT_FALLBACK, EVENT_ASSISTANT_FINAL,
    EVENT_ASSISTANT_REPORTED, EVENT_TURN_SUPPLEMENTED, EVENT_USER_MESSAGE,
};
use super::super::view::{Budget, View};
use super::Projection;

use serde::{Deserialize, Serialize};

/// 转写条目：一句话及其角色。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TranscriptEntry {
    /// `"user"` / `"assistant"` / `"tool"`。
    pub role: String,
    pub text: String,
    /// 工具结果行的**工具名**（`role == "tool"` 时才有；其余角色为 `None`）。
    ///
    /// 单独成字段而不是拼进 `text`：工具名是**事实**（载荷里就有），渲染
    /// （`工具结果(<tool>): ` 前缀）是消费方的事。`skip_serializing_if` 让
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

/// 一条转写条目的渲染行（角色前缀 + 正文）。
///
/// 工具行用 `工具结果(<tool>): ` 前缀——与轮内交换（`render_tool_exchange`）
/// 逐字同形；用户 / 助手行沿用 `用户: ` / `助手: `。
fn render_line(e: &TranscriptEntry) -> String {
    match e.role.as_str() {
        "user" => format!("用户: {}", e.text),
        "tool" => format!("工具结果({}): {}", e.tool.as_deref().unwrap_or(""), e.text),
        _ => format!("助手: {}", e.text),
    }
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
/// `to_prompt()` 把 `role: tool` **降级**成 user 消息里的纯文本，于是模型收到的是
/// 「一整段散文里夹着工具结果」，看不到 `role: tool` 也拿不到 `tool_call_id`。而
/// `ModelProvider::execute_turn` 的签名本身就是 `(system_prompt, messages: &[ChatMessage], …)`
/// —— `role` + `tool_call_id` 是**跨栈契约**不是排版。拍平是已判定的退步。
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
    /// 平铺成单 prompt（多轮历史 + 当前消息）。单轮 ⇒ 裸文本（与无历史形态等价）。
    pub fn to_prompt(&self) -> String {
        match self.entries.as_slice() {
            [] => String::new(),
            [only] => only.text.clone(),
            many => {
                let history = &many[..many.len() - 1];
                let current = &many[many.len() - 1];
                let mut out = String::from("<对话历史>\n");
                for e in history {
                    out.push_str(&render_line(e));
                    out.push('\n');
                }
                out.push_str("</对话历史>\n");
                out.push_str(&render_line(current));
                out
            }
        }
    }

    /// 投影成**消息数组**（ADR-048a 的结构化出口）。
    ///
    /// 与 [`Self::to_prompt`] 的关系：两者读**同一份** `entries`，所以「唯一真源」
    /// 与「结构化协议」不冲突——**排版是消费方的事，事实不是**。
    /// `to_prompt` 留给「只需要一段文本」的调用方（诊断、断言、日志）；
    /// **送给模型的路径必须走这里**，否则 `role` 会退化成纯文本。
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
pub fn transcript() -> Projection<TranscriptView> {
    Projection::new(|events: &[Event], now, _budget: Budget| {
        let mut entries = Vec::new();
        for e in events {
            if e.ts > now {
                continue;
            }
            match (e.entity, e.verb, e.kind.as_str()) {
                (Entity::Turn, Verb::Opened, EVENT_USER_MESSAGE) => {
                    let text = e.payload.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    entries.push(TranscriptEntry {
                        role: "user".into(),
                        text: text.to_string(),
                        tool: None,
                        exclude_from_context: false,
                    });
                }
                (Entity::Turn, Verb::Closed, EVENT_ASSISTANT_FINAL) => {
                    let text = e.payload.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    entries.push(TranscriptEntry {
                        role: "assistant".into(),
                        text: text.to_string(),
                        tool: None,
                        exclude_from_context: false,
                    });
                }
                (Entity::Turn, Verb::Closed, EVENT_ASSISTANT_FALLBACK) => {
                    let why = e.payload.get("why").and_then(|v| v.as_str()).unwrap_or("");
                    entries.push(TranscriptEntry {
                        role: "assistant".into(),
                        text: why.to_string(),
                        tool: None,
                        exclude_from_context: false,
                    });
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
                        entries.push(TranscriptEntry {
                            role: "user".into(),
                            text: text.to_string(),
                            tool: None,
                            exclude_from_context: false,
                        });
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
                        entries.push(TranscriptEntry {
                            role: "assistant".into(),
                            text: text.to_string(),
                            tool: None,
                            exclude_from_context: true,
                        });
                    }
                }
                // 工具结果（`artifact × asserted`）：跨轮可见的「我调用过什么、拿到了什么」。
                (Entity::Artifact, Verb::Asserted, EVENT_ARTIFACT_ADDED) => {
                    let tool = e.payload.get("tool").and_then(|v| v.as_str()).unwrap_or("");
                    let text = e.payload.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    entries.push(TranscriptEntry {
                        role: "tool".into(),
                        text: text.to_string(),
                        tool: Some(tool.to_string()),
                        exclude_from_context: false,
                    });
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
