//! 会话上下文压缩服务
//!
//! 职责：
//! - 检测何时需要压缩会话历史
//! - 准备压缩指令（在当前会话内处理，不递归）
//! - 触发 PreCompact Hook 事件

use std::sync::Arc;

use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
};
use crate::symbio_core::{InvokeRequest, InvokeRequestExt};

const COMPRESSION_TOKEN_THRESHOLD: f64 = 0.7;
const COMPRESSION_PRESERVE_THRESHOLD: f64 = 0.3;
const MIN_COMPRESSION_FRACTION: f64 = 0.05;

/// 主动压缩工具：水位提醒阈值（相对有效上限）。
pub const CONTEXT_NUDGE_THRESHOLD: f64 = 0.55;
/// 迟滞系数：快照后估算再增长不足 (post_tokens × 1.15) 时跳过被动压缩。
pub const COMPACT_HYSTERESIS_FACTOR: f64 = 1.15;
/// 主动压缩最小收益：待压缩历史低于该 token 数时不值得一次 LLM 调用。
pub const MIN_COMPACT_TOKENS: usize = 4000;

/// 获取压缩提示词
pub fn get_compression_prompt() -> String {
    r#"You are the component that summarizes internal chat history into a given structure.

When the conversation history grows too large, you will be invoked to distill the entire history into a concise, structured XML snapshot. This snapshot is CRITICAL, as it will become the agent's *only* memory of the past. The agent will resume its work based solely on this snapshot. All crucial details, plans, errors, and user directives MUST be preserved.

First, you will think through the entire history in a private <scratchpad>. Review the user's overall goal, the agent's actions, tool outputs, file modifications, and any unresolved questions. Identify every piece of information that is essential for future actions.

After your reasoning is complete, generate the final <state_snapshot> XML object. Be incredibly dense with information. Omit any irrelevant conversational filler.

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

    <open_questions>
        Questions that remain unanswered or issues that need to be addressed.
    </open_questions>
</state_snapshot>"#.to_string()
}

/// 找到压缩分割点（仅在 User 边界切分，保证保留区从一轮对话的开头开始）。
///
/// 返回值语义：
/// - `> 0`：`messages[..n]` 为待压缩历史，`messages[n..]` 为保留区；
/// - `0`：找不到安全切分点，本轮放弃压缩。
///
/// 安全规则：
/// 1. **禁止全量压缩**：旧版"末尾是 Assistant 就压缩全部"分支已移除——它违背
///    30% 保留语义，且会把进行中的 tool_calls 压成孤儿（结果子节点失去父节点）。
/// 2. 尾部若存在未配对的 ToolCall（结果子节点尚未落库，典型于 continuation 场景），
///    任何切分都可能拆散配对，返回 0 放弃本轮压缩。
fn find_compress_split_point(messages: &[ChatMessage], fraction: f64) -> usize {
    if fraction <= 0.0 || fraction >= 1.0 || messages.is_empty() {
        return 0;
    }

    // 尾部未配对 ToolCall 检查：从末尾往前扫，遇到 Tool 结果即配对完成；
    // 若先遇到 Assistant 的 ToolCall 节点（其结果不在其后），说明有悬空调用。
    let mut pending_tool_call = false;
    for m in messages.iter().rev() {
        match m.role {
            Some(MessageRole::Tool) => {
                pending_tool_call = false;
                break;
            }
            Some(MessageRole::Assistant) if m.msg_type == Some(MessageType::ToolCall) => {
                pending_tool_call = true;
                break;
            }
            Some(MessageRole::Assistant) => {
                // 纯文本/推理结尾：无悬空调用
                break;
            }
            _ => continue,
        }
    }
    if pending_tool_call {
        return 0;
    }

    let char_counts: Vec<usize> = messages
        .iter()
        .map(|m| serde_json::to_string(m).map(|s| s.len()).unwrap_or(0))
        .collect();
    let total_char_count: usize = char_counts.iter().sum();
    let target_char_count = total_char_count as f64 * fraction;

    let mut last_split_point = 0;
    let mut cumulative_char_count = 0;

    for (i, msg) in messages.iter().enumerate() {
        if msg.role == Some(MessageRole::User) {
            let has_content = msg
                .content
                .as_ref()
                .map(|c| !c.to_text().is_empty())
                .unwrap_or(false);
            if has_content {
                if cumulative_char_count >= target_char_count as usize {
                    return i;
                }
                last_split_point = i;
            }
        }
        cumulative_char_count += char_counts[i];
    }

    last_split_point
}

/// 估算单条消息的 token 数。
///
/// 统一使用 `CalibratedTokenizer`（启发式 × provider 用量反馈校准），
/// **禁止**裸 `text.len()/4`：UTF-8 下中文每字 3 字节，`len()/4` 会低估约 2 倍，
/// 导致压缩触发过晚、请求撞上 provider 的 context-length 400。
///
/// 计入：正文内容 + ToolCall 节点参数（content 即 JSON 文本）+ 每消息固定结构开销。
pub fn estimate_message_tokens(m: &ChatMessage) -> usize {
    use crate::symbio_core::{default_tokenizer, Tokenizer};

    const PER_MESSAGE_OVERHEAD: usize = 8; // role / 框架 / 分隔符等固定开销
    let tok = default_tokenizer();

    let mut total = PER_MESSAGE_OVERHEAD;
    if let Some(c) = &m.content {
        let text = c.to_text();
        if !text.is_empty() {
            total += tok.count(&text);
        }
    }
    if let Some(name) = &m.name {
        if !name.is_empty() {
            total += tok.count(name);
        }
    }
    total
}

/// 估算消息列表的总 token 数。
///
/// `overhead_tokens` 为请求级固定开销（system prompt + 工具定义），由调用方传入；
/// 压缩触发判断必须计入这部分，否则阈值虚高、触发过晚。
pub fn estimate_context_tokens(messages: &[ChatMessage], overhead_tokens: usize) -> usize {
    messages.iter().map(estimate_message_tokens).sum::<usize>() + overhead_tokens
}

/// 估算请求级固定开销：system prompt + 全部工具定义。
pub async fn estimate_request_overhead(
    system_prompt: &str,
    ctx: &Arc<dyn InvokeRequest>,
) -> usize {
    use crate::symbio_core::{default_tokenizer, CAPABILITY_MANAGER, Tokenizer};

    let tok = default_tokenizer();
    let mut total = tok.count(system_prompt);

    if let Some(tool_manager) = ctx.get(CAPABILITY_MANAGER) {
        for cap in tool_manager.list_capability().await {
            let schema_json = cap.input_schema.to_string();
            total += tok.count(&format!("{} {} {}", cap.name, cap.description, schema_json));
        }
    }
    total
}

/// 检查是否需要压缩，返回是否应该开始压缩。
///
/// `overhead_tokens`：system prompt + 工具定义等请求级固定开销，必须计入。
/// `force`：跳过阈值判断（用户 /compact 或主动压缩工具）。
pub fn should_start_compression(
    messages: &[ChatMessage],
    context_limit: usize,
    force: bool,
    overhead_tokens: usize,
) -> bool {
    if messages.is_empty() {
        return false;
    }
    if force {
        return true;
    }
    let current = estimate_context_tokens(messages, overhead_tokens);
    let threshold = (context_limit as f64 * COMPRESSION_TOKEN_THRESHOLD) as usize;

    if current < threshold {
        return false;
    }

    // 迟滞：若最近一次快照后已增长不足 15%，说明上一轮压缩收益被挥霍，
    // 再次压缩大概率是"快照过小"导致的循环调用，跳过并依赖确定性层消化。
    // 例外：force（主动压缩 / 用户指令）不受此限制。
    if let Some(last) = messages
        .iter()
        .find(|m| m.meta.as_ref().map(|meta| meta.get("compacted") == Some(&serde_json::json!(true))).unwrap_or(false))
    {
        if let Some(post) = last
            .meta
            .as_ref()
            .and_then(|meta| meta.get("post_tokens"))
            .and_then(|v| v.as_u64())
        {
            let hysteresis_floor = (post as f64 * COMPACT_HYSTERESIS_FACTOR) as usize;
            if current < hysteresis_floor {
                return false;
            }
        }
    }

    true
}

/// 水位提醒判断：估算用量是否达到提醒阈值（55%）。
/// 主动压缩工具（context_compact）配合此机制，让模型在安全时机自行压缩。
pub fn should_emit_context_nudge(messages: &[ChatMessage], context_limit: usize, overhead_tokens: usize) -> bool {
    if messages.is_empty() {
        return false;
    }
    let current = estimate_context_tokens(messages, overhead_tokens);
    current >= (context_limit as f64 * CONTEXT_NUDGE_THRESHOLD) as usize
}

/// 准备压缩：将要压缩的历史提取出来，生成压缩指令
/// 返回 (压缩指令, 要压缩的历史, 要保留的历史)
pub fn prepare_compression(
    messages: &[ChatMessage],
) -> Option<(ChatMessage, Vec<ChatMessage>, Vec<ChatMessage>)> {
    let split_point = find_compress_split_point(messages, 1.0 - COMPRESSION_PRESERVE_THRESHOLD);
    if split_point == 0 {
        return None;
    }

    let history_to_compress = messages[..split_point].to_vec();
    let history_to_keep = messages[split_point..].to_vec();

    // 检查压缩比例
    let compress_char_count: usize = history_to_compress
        .iter()
        .map(|m| serde_json::to_string(m).map(|s| s.len()).unwrap_or(0))
        .sum();
    let total_char_count: usize = messages
        .iter()
        .map(|m| serde_json::to_string(m).map(|s| s.len()).unwrap_or(0))
        .sum();

    if total_char_count > 0
        && (compress_char_count as f64) / (total_char_count as f64) < MIN_COMPRESSION_FRACTION
    {
        return None;
    }

    // 生成压缩指令
    let history_json = serde_json::to_string(&history_to_compress).unwrap_or_default();
    let prompt = format!(
        "{}\n\n## Chat History to Summarize:\n{}",
        get_compression_prompt(),
        history_json
    );

    let compression_msg = ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: Some(MessageRole::User),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text(prompt)),
        ..Default::default()
    };

    Some((compression_msg, history_to_compress, history_to_keep))
}

/// 老化淡化：当对话轮次过多时，把较早的工具结果（`role=Tool`、`msg_type=Text`）做 head/tail 摘要，
/// 保留最近 `keep_recent_turns` 个 user turn 起的原文，以及**全部** assistant 文本 / 推理节点
/// （绝不改动 assistant 消息，最大限度保护思维链）。
///
/// 这是对用户"不要 max_tool_rounds 硬性上限、而是有效淡化历时轮次"诉求的核心落地：
/// 上下文不会无限膨胀，但近期交互与推理全程保留。被淡化的结果会标记 `tool_result_faded`，
/// 原始全文仍存于 `archive_path`（可用 local/file_read 取回），符合"压缩可回溯"原则。
pub fn fade_aged_tool_results(messages: &mut [ChatMessage], keep_recent_turns: usize) {
    use crate::symbio_core::{default_tokenizer, Tokenizer};

    // 以 user 消息为边界，定位"最近 keep_recent_turns 个 user turn"的起始下标；
    // 该下标之前的消息视为"历史"，对其中的工具结果做淡化。
    let user_positions: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, m)| m.role == Some(MessageRole::User))
        .map(|(i, _)| i)
        .collect();
    let keep_from = if user_positions.len() > keep_recent_turns {
        user_positions[user_positions.len() - keep_recent_turns]
    } else {
        0
    };

    const FADE_BUDGET: usize = 2048; // 老化工具结果压到约 2k token
    let tok = default_tokenizer();
    for m in messages.iter_mut().take(keep_from) {
        if m.role == Some(MessageRole::Tool) && m.msg_type == Some(MessageType::Text) {
            if let Some(MessageContent::Text(t)) = &m.content {
                if tok.count(t) > FADE_BUDGET {
                    let g = super::tool_result_guard::guard_tool_result(t, FADE_BUDGET);
                    m.content = Some(MessageContent::Text(g.text));
                    let mut meta = m.meta.clone().unwrap_or_else(|| serde_json::json!({}));
                    meta["tool_result_faded"] = serde_json::json!(true);
                    if let Some(p) = g.archive_path {
                        meta["archive_path"] = serde_json::json!(p);
                    }
                    m.meta = Some(meta);
                }
            }
        }
    }
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

/// 快照校验失败时的兜底：把纯文本输出包裹为极简快照。
/// 有总比无好——但明确标注这是降级产物，下一轮压缩时模型可据此补全结构。
pub fn fallback_snapshot(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(format!(
        "<state_snapshot>\n<key_knowledge>\n[降级快照：上次压缩时模型未按结构输出，以下为原始摘要，内容可能不完整]\n\n{trimmed}\n</key_knowledge>\n</state_snapshot>"
    ))
}

/// `context_compact` 工具名（chat_loop 拦截分发用）。
pub const CONTEXT_COMPACT_TOOL_NAME: &str = "context_compact";

/// 构建主动压缩请求（`context_compact` 工具执行体）。
///
/// 与被动压缩（`prepare_compression`）的差异：显式指定待压缩历史，
/// 并把模型的 `hints`（必须保留的关键信息）追加进提示词——
/// 让模型"亲手"决定快照里必须留下什么。
pub fn build_compression_request(history: &[ChatMessage], hints: Option<&str>) -> ChatMessage {
    let history_json = serde_json::to_string(history).unwrap_or_else(|_| "[]".to_string());
    let hints_section = match hints {
        Some(h) if !h.trim().is_empty() => {
            format!("\n\n## Must-Preserve Hints (from the agent, MUST be kept in the snapshot):\n{h}")
        }
        _ => String::new(),
    };
    let prompt = format!(
        "{}\n\n## Chat History to Summarize:\n{}{}",
        get_compression_prompt(),
        history_json,
        hints_section
    );
    ChatMessage {
        id: uuid::Uuid::new_v4().to_string(),
        role: Some(MessageRole::User),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text(prompt)),
        status: Some(MessageStatus::Completed),
        ..Default::default()
    }
}

/// `context_compact` 工具定义：暴露给模型，由模型在任务阶段间隙主动调用。
///
/// 不进 CapabilityManager 分发——由 chat_loop 拦截执行（需要编排器内部的
/// 压缩链路：LLM 摘要 + 上下文替换 + 会话持久化）。
pub fn context_compact_tool_meta() -> crate::symbio_core::CapabilityMeta {
    crate::symbio_core::CapabilityMeta {
        name: CONTEXT_COMPACT_TOOL_NAME.to_string(),
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
        keywords: vec!["compact".to_string(), "压缩".to_string(), "上下文".to_string()],
        category: Some(crate::symbio_core::CapabilityCategory::Core),
        examples: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_msg(text: &str) -> ChatMessage {
        ChatMessage {
            id: uuid::Uuid::new_v4().to_string(),
            role: Some(MessageRole::User),
            msg_type: Some(MessageType::Text),
            content: Some(MessageContent::Text(text.to_string())),
            ..Default::default()
        }
    }

    fn assistant_msg(text: &str) -> ChatMessage {
        ChatMessage {
            id: uuid::Uuid::new_v4().to_string(),
            role: Some(MessageRole::Assistant),
            msg_type: Some(MessageType::Text),
            content: Some(MessageContent::Text(text.to_string())),
            ..Default::default()
        }
    }

    fn tool_call_msg() -> ChatMessage {
        ChatMessage {
            id: uuid::Uuid::new_v4().to_string(),
            role: Some(MessageRole::Assistant),
            msg_type: Some(MessageType::ToolCall),
            name: Some("local/file_read".to_string()),
            content: Some(MessageContent::Text(r#"{"path":"a.rs"}"#.to_string())),
            ..Default::default()
        }
    }

    fn tool_result_msg() -> ChatMessage {
        ChatMessage {
            id: uuid::Uuid::new_v4().to_string(),
            role: Some(MessageRole::Tool),
            msg_type: Some(MessageType::Text),
            content: Some(MessageContent::Text("file content".to_string())),
            ..Default::default()
        }
    }

    #[test]
    fn test_estimate_tokens_counts_tool_calls_and_chinese() {
        // 中文 3 字节/字：旧的 len()/4 会严重低估；tokenizer 应给出合理值
        let m = user_msg(&"你好世界。".repeat(100)); // 500 字 ≈ 500~1000 tok
        let est = estimate_message_tokens(&m);
        assert!(est >= 400, "中文估算不应低估: {est}");

        // ToolCall 参数（content 即 JSON）必须被计入
        let tc = tool_call_msg();
        assert!(estimate_message_tokens(&tc) > 8);
    }

    #[test]
    fn test_estimate_context_tokens_includes_overhead() {
        let msgs = vec![user_msg("hi")];
        let est = estimate_context_tokens(&msgs, 5000);
        assert!(est >= 5000, "请求级固定开销必须计入");
    }

    #[test]
    fn test_split_point_never_full_compress_on_assistant_tail() {
        // U A U A：末尾是 Assistant（旧版会全量压缩）。保留 30% ⇒ 应在早期 User 边界切分。
        let msgs = vec![
            user_msg(&"x".repeat(1000)),
            assistant_msg(&"y".repeat(1000)),
            user_msg(&"z".repeat(1000)),
            assistant_msg(&"w".repeat(1000)),
        ];
        let split = find_compress_split_point(&msgs, 0.7);
        assert!(split > 0 && split < msgs.len(), "不得全量压缩, split={split}");
        assert_eq!(msgs[split].role, Some(MessageRole::User));
    }

    #[test]
    fn test_split_point_zero_on_pending_tool_call() {
        // 尾部 ToolCall 无结果子节点（continuation 场景）⇒ 必须放弃压缩
        let msgs = vec![
            user_msg(&"x".repeat(1000)),
            assistant_msg(&"y".repeat(1000)),
            tool_call_msg(),
        ];
        assert_eq!(find_compress_split_point(&msgs, 0.7), 0);
    }

    #[test]
    fn test_split_point_ok_when_tool_call_paired() {
        // 配对完整的 ToolCall/Tool 不阻塞压缩
        let msgs = vec![
            user_msg(&"x".repeat(1000)),
            assistant_msg(&"y".repeat(1000)),
            tool_call_msg(),
            tool_result_msg(),
            user_msg(&"z".repeat(1000)),
        ];
        let split = find_compress_split_point(&msgs, 0.7);
        assert!(split > 0, "配对完整时不应放弃压缩");
    }

    #[test]
    fn test_should_start_compression_hysteresis() {
        // 压缩后 post_tokens=1000，当前估算 1100（< 1150）⇒ 即使超 70% 阈值也跳过
        let mut snapshot = assistant_msg("snapshot");
        snapshot.meta = Some(serde_json::json!({
            "compacted": true,
            "post_tokens": 1000
        }));
        // 构造一条足够大的历史让 current 估算落在 (1000, 1150) 区间
        let msgs = vec![snapshot, user_msg(&"中文字符填充".repeat(50))];
        let small_limit = estimate_context_tokens(&msgs, 0); // ≈ current
        // 阈值设为远小于 current ⇒ 无迟滞时会触发
        let threshold_limit = (small_limit as f64 / 0.7 * 0.99) as usize;
        // current/threshold ≈ 0.7/0.99 > 1 ⇒ 超阈值，但 current < 1150 ⇒ 迟滞应拦截
        assert!(!should_start_compression(&msgs, threshold_limit, false, 0));
        // force 不受迟滞限制
        assert!(should_start_compression(&msgs, threshold_limit, true, 0));
    }

    #[test]
    fn test_extract_snapshot() {
        let out = "thinking... <state_snapshot>\n  <overall_goal>done</overall_goal>\n</state_snapshot> trailing";
        let s = extract_snapshot(out).unwrap();
        assert!(s.starts_with("<state_snapshot>"));
        assert!(s.contains("done"));
        assert!(extract_snapshot("no xml here").is_none());
        assert!(extract_snapshot("<state_snapshot></state_snapshot>").is_none());
    }

    #[test]
    fn test_fallback_snapshot() {
        let s = fallback_snapshot("plain summary").unwrap();
        assert!(s.contains("state_snapshot"));
        assert!(s.contains("plain summary"));
        assert!(fallback_snapshot("  \n ").is_none());
    }

    #[test]
    fn test_prepare_compression_rejects_when_no_split() {
        // 单条 User 消息（巨型单轮场景）：无切分点 ⇒ prepare_compression 返回 None
        let msgs = vec![user_msg(&"x".repeat(100_000))];
        assert!(prepare_compression(&msgs).is_none());
    }
}
