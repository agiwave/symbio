use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageType,
};
use std::collections::HashSet;
use std::path::Path;

/// 自动清理历史会话的过程工具调用信息，只保留最后的文本结果。
pub async fn prune_historical_tool_calls(
    messages: &mut Vec<ChatMessage>,
    session_dir: Option<&Path>,
    keep_turns: usize,
) {
    let mut user_indices = Vec::new();
    for (idx, msg) in messages.iter().enumerate() {
        if msg.role == Some(MessageRole::User) {
            user_indices.push(idx);
        }
    }

    if user_indices.len() <= keep_turns {
        return;
    }

    let limit_idx = user_indices[user_indices.len() - keep_turns];
    let mut to_remove = HashSet::new();

    for msg in &messages[..limit_idx] {
        if msg.role == Some(MessageRole::Tool)
            || msg.msg_type == Some(MessageType::ToolCall)
            || msg.msg_type == Some(MessageType::Reasoning)
        {
            if let Some(dir) = session_dir {
                if let Some(rel_path) = msg
                    .meta
                    .as_ref()
                    .and_then(|m| m.get("archive_path"))
                    .and_then(|v| v.as_str())
                {
                    let full_path = dir.join(rel_path);
                    if full_path.exists() {
                        let _ = tokio::fs::remove_file(full_path).await;
                    }
                }
            }
            to_remove.insert(msg.id.clone());
        }
    }

    // 同时移除被剪除 ToolCall 的直接子节点（请求 Text / 响应 Text）
    let extra: HashSet<String> = messages[..limit_idx]
        .iter()
        .filter(|m| m.parent_id.as_ref().map(|p| to_remove.contains(p)).unwrap_or(false))
        .map(|m| m.id.clone())
        .collect();
    to_remove.extend(extra);

    for msg in &messages[..limit_idx] {
        if msg.role == Some(MessageRole::Assistant) {
            let content_text = msg
                .content
                .as_ref()
                .map(|c| c.to_text())
                .unwrap_or_default();
            let mut has_retained_children = false;
            for child in &messages[..limit_idx] {
                if child.parent_id.as_ref() == Some(&msg.id) && !to_remove.contains(&child.id) {
                    has_retained_children = true;
                    break;
                }
            }
            if content_text.trim().is_empty() && !has_retained_children {
                to_remove.insert(msg.id.clone());
            }
        }
    }

    messages.retain(|msg| !to_remove.contains(&msg.id));
}

/// 混合滑动窗口过滤历史工具调用 (Layered Sliding Window)
pub fn apply_layered_sliding_window(
    messages: &[ChatMessage],
    max_active_tool_calls: usize,
) -> Vec<ChatMessage> {
    let mut tool_call_ids = Vec::new();
    for msg in messages {
        if msg.msg_type == Some(MessageType::ToolCall) {
            tool_call_ids.push(msg.id.clone());
        }
    }

    let active_threshold = if tool_call_ids.len() > max_active_tool_calls {
        tool_call_ids.len() - max_active_tool_calls
    } else {
        0
    };

    let tool_call_ids_set: HashSet<String> = tool_call_ids.iter().cloned().collect();
    let active_ids: HashSet<String> = tool_call_ids
        .iter()
        .skip(active_threshold)
        .cloned()
        .collect();

    let mut filtered = Vec::with_capacity(messages.len());
    for msg in messages {
        let mut new_msg = msg.clone();

        // 父节点是被裁剪的 ToolCall？
        let parent_inactive = msg
            .parent_id
            .as_ref()
            .map(|pid| tool_call_ids_set.contains(pid) && !active_ids.contains(pid))
            .unwrap_or(false);

        if msg.msg_type == Some(MessageType::ToolCall) {
            if !active_ids.contains(&msg.id) {
                // ToolCall 组合节点本身无内容，仅清理可能遗留的 tool_calls meta
                if let Some(mut meta) = new_msg.meta.as_ref().and_then(|m| m.as_object()).cloned() {
                    meta.remove("tool_calls");
                    new_msg.meta = Some(serde_json::Value::Object(meta));
                }
            }
        } else if parent_inactive {
            // 请求 / 响应 Text 子节点 → 骨架化内容以节省上下文
            let preview = msg
                .content
                .as_ref()
                .map(|c| c.to_text())
                .unwrap_or_default();
            let success_status = if preview.contains("Error") || preview.contains("failed") {
                "failed"
            } else {
                "successfully"
            };
            let label = if msg.role == Some(MessageRole::Tool) {
                format!(
                    "[System Info: Tool result received ({}). Output skeletonized.]",
                    success_status
                )
            } else {
                "[System Info: Tool call input parameters skeletonized to save context.]".to_string()
            };
            new_msg.content = Some(MessageContent::Text(label));
        }

        filtered.push(new_msg);
    }

    filtered
}
