//! 会话上下文压缩服务
//!
//! 职责：
//! - 检测何时需要压缩会话历史
//! - 准备压缩指令（在当前会话内处理，不递归）
//! - 触发 PreCompact Hook 事件

use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageType,
};

const COMPRESSION_TOKEN_THRESHOLD: f64 = 0.7;
const COMPRESSION_PRESERVE_THRESHOLD: f64 = 0.3;
const MIN_COMPRESSION_FRACTION: f64 = 0.05;

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

/// 找到压缩分割点
fn find_compress_split_point(messages: &[ChatMessage], fraction: f64) -> usize {
    if fraction <= 0.0 || fraction >= 1.0 {
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

    let last_content = messages.last();
    if last_content
        .map(|m| m.role == Some(MessageRole::Assistant))
        .unwrap_or(false)
    {
        return messages.len();
    }

    last_split_point
}

/// 估算 Token 数
fn estimate_token_count(messages: &[ChatMessage]) -> usize {
    let text: String = messages
        .iter()
        .filter_map(|m| {
            m.content
                .as_ref()
                .map(|c| c.to_text())
                .filter(|s| !s.is_empty())
        })
        .collect::<Vec<_>>()
        .join("\n");
    text.len() / 4
}

/// 检查是否需要压缩，返回是否应该开始压缩
pub fn should_start_compression(
    messages: &[ChatMessage],
    context_limit: usize,
    force: bool,
) -> bool {
    let original_token_est = estimate_token_count(messages);
    let threshold = (context_limit as f64 * COMPRESSION_TOKEN_THRESHOLD) as usize;

    !messages.is_empty() && (force || original_token_est >= threshold)
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
