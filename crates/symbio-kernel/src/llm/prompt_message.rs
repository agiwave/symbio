//! `PromptMessage` —— 送给模型的一组消息的**最小结构化形态**。
//!
//! 它住在内核，理由与 [`TurnOutput`](crate::llm::turn::TurnOutput) 同一条：这是模型接缝
//! 的**输入词汇**。`LlmAdapter::generate` 的入参就是 `&[PromptMessage]`（core 的 port），
//! 而投影（core）、措辞与判决（`compose` / `classify`）、工具循环（`loop`）、以及测试替身
//! （`testkit`）都要能命名它——跨 crate 共享的**纯数据**，没有行为、没有机制。
//! 语法进内核、机制留 core（D-188 / D-255）这条线，把它划在前者。
//!
//! 线的这一侧只放形状：`TranscriptEntry → PromptMessage` 这类**从 core 的投影构造**的
//! 转换留在 core（那是投影的知识，不是模型的词汇）。

use serde::{Deserialize, Serialize};

/// 一条**结构化**的 prompt 消息：角色与正文分开，工具关联单独成字段。
///
/// ## 为什么定义它，而不是直接用 `ChatNode`
///
/// `ChatNode` 是**图语义**（有 `id` / `tool_calls` / `status` / 帧类型），服务于会话与
/// 界面；模型接缝只需要「谁说的、说了什么、关联哪次调用」三个维度。若 adapter 的入口
/// 就是 `ChatNode`，每个 provider 适配器都被那套图语义绑住——它要么实现一堆用不上的
/// 字段，要么在边界处现编一份「只取三个维度」的私有类型（那正是本类型要消除的重复）。
///
/// 所以内核出**最小结构化形态**，由 adapter 边界翻译成 provider 的消息。
/// **结构在词汇层、线格式在 adapter**——与 D-043 的域边界一致。
///
/// ## 为什么必须有它（D-048a）
///
/// `ModelProvider::round_trip` 的签名本身就是消息数组
/// `(system_prompt, messages: &[ChatNode], …)`——`role` + `tool_call_id` 是**跨栈契约**
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
    /// 一个**稳定 id**，而不是让消息裸奔——裸奔会让「拒绝未知 tool_call_id」的
    /// provider 整条请求失败，那比拍平成一条 user 消息**更糟**（拍平至少还能答）。
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
    /// `tool_calls`（adapter 侧 `flatten_chat_messages` 末尾的清洗段，理由是
    /// 「否则直接 400」）。而若 assistant 那条只把调用写在**正文**里
    /// （"调用工具 vdfs_read {...}"），它在协议层就不是一条带 `tool_calls` 的
    /// assistant 消息，于是**所有 tool 结果都成了孤儿**并被清掉——模型永远收不到
    /// 工具结果，于是**无限工具循环**。
    ///
    /// 这个坑是实测撞出来的（9905 次请求、CLI 撞 120s 超时），而症状（无限循环）
    /// 指不到「请求包清洗」这一层。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<PromptToolCall>>,
}

/// 一次工具调用的声明（最小形态；线格式由 adapter 翻译）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptToolCall {
    /// 调用 id（`role: tool` 消息用它回指本次调用）。
    pub id: String,
    pub name: String,
    /// 参数（**结构化**，不是拼进正文的字符串）。
    pub arguments: serde_json::Value,
}
