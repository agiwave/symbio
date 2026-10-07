//! 压缩提示词与 `<state_snapshot>` 快照协议。
//!
//! 提示词文本 → 指纹 → 快照 meta，构成「提示词变更可从产物侧观测」的溯源链；
//! `context_compact` 工具元也在本模块——它只是给模型看的入口说明。

use super::*;
use crate::symbio_core::chat_message::{MessageContent, MessageStatus};

/// 压缩协议版本（P2-3）：快照 meta 记录此版本，用于从产物侧验证协议演进是否生效。
/// 语义：v2 = L0 会话目录存档 + 三层统一取回协议 + JSON 语义摘要 + 快照指纹。
pub const COMPRESSION_PROTOCOL_VERSION: &str = "v2";

/// 当前压缩提示词的指纹（FNV-1a 64，取高 32 位十六进制）。
///
/// 提示词文本变更 → 指纹变更 → 新快照 meta 可观测；用于回答
/// "提示词强化是否生效"——对比历史快照 meta 即可确认该轮压缩用的提示词版本。
pub fn compression_prompt_fingerprint() -> String {
    fn fnv1a(data: &[u8]) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in data {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }
    let prompt = get_compression_prompt();
    format!("{:08x}", (fnv1a(prompt.as_bytes()) >> 32) as u32)
}

/// 获取压缩提示词
pub fn get_compression_prompt() -> String {
    r#"You are the component that summarizes internal chat history into a given structure.

When the conversation history grows too large, you will be invoked to distill the entire history into a concise, structured XML snapshot. This snapshot is CRITICAL, as it will become the agent's *only* memory of the past. The agent will resume its work based solely on this snapshot. All crucial details, plans, errors, and user directives MUST be preserved.

First, you will think through the entire history in a private <scratchpad>. Review the user's overall goal, the agent's actions, tool outputs, file modifications, and any unresolved questions. Identify every piece of information that is essential for future actions.

After your reasoning is complete, generate the final <state_snapshot> XML object. Be incredibly dense with information. Omit any irrelevant conversational filler.

Signal-to-noise rules (apply while writing the snapshot):
- Reconcile before output: a previous snapshot found in the history is unverified INPUT, not ground truth. When newer findings contradict an inherited item, correct or drop it; never copy old <key_knowledge> forward unchecked — an error that enters a snapshot survives every later generation.
- Each fact appears exactly once across ALL sections. If the same fact fits multiple sections, place it in the most relevant one and do not repeat it.
- Record conclusions and outcomes, not process metrics. Drop line numbers, byte counts, read ranges, raw dumps, and step-by-step command transcripts; keep the final state and what it implies for future work.
- <key_knowledge> holds only facts and constraints that still bind FUTURE work. Drop test-writing trivia, closed-issue aftermath notes, and volatile numbers (test counts, file sizes); state invariants instead ("full suite + clippy clean", not a passing count). Prefix each decision with "Decision:" — state the chosen option, the alternatives rejected, and why they were rejected.
- Distinguish REGENERABLE from CONVERSATION-ONLY before writing a fact down. A fact is regenerable when the repo or environment can be asked again for it — inventories of modules, directories, endpoints, dependencies, counts, layouts, and anything an auto-generated reference already states. For those, name the source and how to refresh it instead of copying entries: the source is regenerated from truth and is always fresher than a summary of a summary, while a copied entry silently rots yet still reads as authoritative. Spend the freed space on what is not regenerable.
- What ONLY this conversation could reveal MUST be kept in full: user preferences and explicit directives, decisions together with the alternatives rejected and why, dead ends and why they failed, tooling quirks learned the hard way, and the reasoning behind an in-flight choice. Pointer-ising regenerable facts is never a licence to lose judgement.
- <completed_items>: one line per item — the outcome and where it landed. If an item leaves a lasting constraint, that constraint belongs in <key_knowledge>; the how-it-was-done narrative stays in the transcript.
- Compress each error to ONE line: the conclusion plus its root cause. Keep errors ONLY if they still constrain future actions (an unresolved failure, a known pitfall); drop errors that were already fixed and whose fix is recorded in <completed_items>.
- Before listing a question in <open_questions>, if the retained history already answers it, record the confirmed answer instead. You have no tool access during compaction; if verification requires an external check, say so explicitly.
- In <open_questions>, distinguish waiting-on-input items (blocked on the user or an external dependency — name what is awaited) from items needing investigation (state what to check first).
- If a todo list exists in the conversation, reference its item IDs/titles in <in_progress_items> instead of restating full descriptions; the agent retains live access to the list.

The structure MUST be as follows:

<state_snapshot>
    <overall_goal>
        A single, concise sentence describing the user's high-level objective.
    </overall_goal>

    <key_knowledge>
        Crucial facts, conventions, and constraints the agent must remember based on the conversation history and interaction with the user. Use bullet points.
    </key_knowledge>

    <completed_items>
        Items that have been completed, including:
        - Files created, modified, or deleted
        - Commands executed and their results
        - User approvals or confirmations received
        - Problems solved or resolved
    </completed_items>

    <in_progress_items>
        Items currently in progress or pending completion.
    </in_progress_items>

    <next_step>
        A single line describing the immediate action the agent should take first upon resuming.
    </next_step>

    <open_questions>
        Questions that remain unanswered or issues that need to be addressed.
    </open_questions>
</state_snapshot>"#.to_string()
}

/// 从模型输出中提取 `<state_snapshot>` XML 块（容错空白与转义）。
/// 模型偶发会把 scratchpad 也吐出来，或被 max_tokens 截断——必须解析校验，
/// 残缺内容不能原样成为唯一记忆。
pub fn extract_snapshot(text: &str) -> Option<String> {
    const OPEN: &str = "<state_snapshot>";
    const CLOSE: &str = "</state_snapshot>";
    let start = text.find(OPEN)?;
    let after_open = start + OPEN.len();
    let end = text[after_open..].find(CLOSE)? + after_open;
    let inner = text[after_open..end].trim();
    if inner.is_empty() {
        return None;
    }
    Some(format!("{OPEN}\n{inner}\n{CLOSE}"))
}

/// `context_compact` 工具名（chat_loop 拦截分发用）。
pub const CONTEXT_COMPACT_TOOL_NAME: &str = "context_compact";

/// 把模型输出的 XML 快照渲染为纯文本分节（用于落库，诉求3）。
///
/// 快照会以 assistant 消息长期驻留上下文——若原样保留 `<state_snapshot>`/
/// `<key_knowledge>` 等 XML 标签，模型会把历史里的这条消息当作"期望输出格式"
/// 来模仿，在正常对话中频繁输出同类标签总结。落库前把标签替换为可读分节标记：
/// 信息不丢，但切断"XML 标签 → 格式模仿"的泄漏链。
/// 压缩子系统内部（提取/校验/纠正重试）仍统一使用 XML。
///
/// 头部注入时点声明：快照是"截至某时刻"的状态切片，其【进行中】/【待确认】
/// 分节会随后续轮次自然过期。没有时点标记，读者（模型或人）无法区分
/// "快照说未完成"与"实际早已完成"——只能靠最新消息反推，易误判旧状态为现役。
pub fn render_snapshot_for_history(snapshot_text: &str) -> String {
    let rendered = snapshot_text
        .replace("<state_snapshot>", "")
        .replace("</state_snapshot>", "")
        .replace("<overall_goal>", "【目标】")
        .replace("</overall_goal>", "")
        .replace("<key_knowledge>", "【关键知识】")
        .replace("</key_knowledge>", "")
        .replace("<completed_items>", "【已完成】")
        .replace("</completed_items>", "")
        .replace("<in_progress_items>", "【进行中】")
        .replace("</in_progress_items>", "")
        .replace("<next_step>", "【下一步】")
        .replace("</next_step>", "")
        .replace("<open_questions>", "【待确认问题】")
        .replace("</open_questions>", "")
        .trim()
        .to_string();
    let snapshot_at = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_default();
    format!("[快照时点：{snapshot_at}，其后消息未纳入本快照，【进行中】/【待确认问题】以最新消息为准]\n{rendered}")
}

/// 构建主动压缩请求（`context_compact` 工具执行体）。
///
/// 与被动压缩（`prepare_compression`）的差异：显式指定待压缩历史，
/// 并把模型的 `hints`（必须保留的关键信息）追加进提示词——
/// 让模型"亲手"决定快照里必须留下什么。
/// 构造压缩 LLM 请求的消息序列：**待压缩历史原样 + 末尾一条压缩指令**。
///
/// # 为什么不再把历史序列化成 JSON
///
/// 旧写法是 `serde_json::to_string(history)` 塞进单条 user 消息的 content。
/// 每条 [`ChatMessage`] 都带着 `id` / `parent_id` / `seq` / `status` / `timestamp` /
/// `meta`（token 统计、工具名、转存路径……）——这些是**存储与前端**需要的，
/// 模型一个都不关心，却要为它们的 JSON 语法原样付费：字段名、引号、转义、
/// 嵌套括号全是纯开销，且随历史长度与消息条数线性放大。
///
/// 改为把历史消息**原样**交给 provider：它的 `flatten_chat_messages` 会把消息树
/// 投影成模型熟悉的对话形态（`Turn` → assistant 聚合、`ToolCall` → tool_calls、
/// 工具结果 → `role=tool`，并裁掉陈旧思考链）。模型看到的是**对话**，不是数据转储。
///
/// 附带好处：快照校验失败时的纠正重试（`context.messages.push(retry_msg)`）天然
/// 变成一段正常的多轮对话（历史 → 指令 → 纠正），而不是「JSON 之后追加一句话」。
pub fn build_compression_request(history: &[ChatMessage], hints: Option<&str>) -> Vec<ChatMessage> {
    let mut out = history.to_vec();
    out.push(compression_instruction(hints));
    out
}

/// 压缩指令：历史以对话形态排在它**之前**，因此这里只需说"把上面这段蒸馏成快照"。
///
/// 输出结构的全部约束在 system 提示词里，此处不重复——重复会诱导模型模仿格式。
fn compression_instruction(hints: Option<&str>) -> ChatMessage {
    let hints_section = match hints {
        Some(h) if !h.trim().is_empty() => {
            format!(
                "\n\n## Must-Preserve Hints (from the agent, MUST be kept in the snapshot):\n{h}"
            )
        }
        _ => String::new(),
    };
    ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: Some(MessageRole::User),
        msg_type: Some(MessageType::Text),
        // 措辞刻意**不提** `<state_snapshot>` 标签名：结构定义只在 system 侧出现
        // 一次。在对话里重复格式指令会诱导模型模仿模板而非按 system 输出。
        content: Some(MessageContent::Text(format!(
            "Distill the conversation above into the structured snapshot that your \
             instructions define. Output the snapshot only.{hints_section}"
        ))),
        status: Some(MessageStatus::Completed),
        ..Default::default()
    }
}

/// `context_compact` 工具定义：暴露给模型，由模型在任务阶段间隙主动调用。
///
/// 不进 CapabilityVisitor 分发——由 chat_loop 拦截执行（需要编排器内部的
/// 压缩链路：LLM 摘要 + 上下文替换 + 会话持久化）。
pub fn context_compact_tool_meta() -> crate::symbio_core::CapabilityMeta {
    crate::symbio_core::CapabilityMeta {
        name: CONTEXT_COMPACT_TOOL_NAME.to_string(),
        context_retention: None,
        description: "Compact the conversation history: distill older messages into a \
            structured state snapshot and keep only recent context in the session. \
            Call this when you have just finished a major subtask, when the context is \
            filled with intermediate outputs you no longer need, or when a system note \
            warns that context usage is high. The session continues seamlessly from the \
            snapshot. Pass `hints` describing information that MUST be preserved \
            (file paths, plans, constraints, open questions)."
            .to_string(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {
                "hints": {
                    "type": "string",
                    "description": "Key facts/plans/constraints that must be preserved in the snapshot"
                }
            }
        }),
        keywords: vec![
            "compact".to_string(),
            "压缩".to_string(),
            "上下文".to_string(),
        ],
        category: Some(crate::symbio_core::CapabilityCategory::Core),
        examples: None,
        ..Default::default()
    }
}

#[cfg(test)]
#[path = "prompt.test.rs"]
mod tests;
