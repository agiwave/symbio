//! 上下文窗口策略纯函数 —— 历史工具调用的分层滑窗骨架化。
//!
//! Phase sink：自 `symbio_core/context_window.rs` 下沉至 session 插件——E-② 后
//! 该纯函数的唯一消费者是 session 请求视图构建（`compression.rs::build_request_view`），
//! "跨插件共享"的前提（model 构建 request view）已随 Phase E-② 循环族下沉消失，
//! 属单一模块私有设施，不再置于 core 共享层。

use crate::symbio_core::ToolContextRetention;
use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageType,
};
use std::collections::{HashMap, HashSet};

/// 混合滑动窗口过滤历史工具调用 (Layered Sliding Window)
///
/// 两级压缩规则（只骨架化、不删除，保持 tool_call/tool_result 配对合法）：
/// 1. 全局窗口：超出 `max_active_tool_calls` 的历史 ToolCall —— 自身参数骨架化、
///    其结果子节点骨架化；
/// 2. 工具级保留策略：`retention` 按工具 CapabilityMeta.name（裸名，如
///    "todo_write"）→ 声明策略映射。声明了 `LastOnly` / `LastN(n)` 的工具，其更早
///    的调用即使仍在全局窗口内，参数与结果同样骨架化（同工具"重复全量写入"的
///    历史对后续推理无参考价值）。映射由调用方在运行时按工具声明动态构建
///    （session 会话循环内直接用 CapabilityManager），**不持久化**、
///    不写入任何消息 meta。
///
/// ToolCall 节点的 name 是 LLM 可见全名（如 "local/todo_write"），此处匹配时
/// 取最后一个 '/' 后的短名。
pub fn apply_layered_sliding_window(
    messages: &[ChatMessage],
    max_active_tool_calls: usize,
    retention: &HashMap<String, ToolContextRetention>,
) -> Vec<ChatMessage> {
    // ── 收集全部 ToolCall（保序），计算两级"失效"判定 ──────────────────────
    // info:         ToolCall id → (全局序号, 工具短名, 自声明保留策略)
    // per_tool_seq: 工具短名 → 该工具的调用 id 序列（工具级保留策略用）
    let mut info: HashMap<String, (usize, String, Option<ToolContextRetention>)> = HashMap::new();
    let mut per_tool_seq: HashMap<String, Vec<String>> = HashMap::new();
    for msg in messages {
        if msg.msg_type == Some(MessageType::ToolCall) {
            let idx = info.len();
            let full_name = msg.name.clone().unwrap_or_default();
            // "local/todo_write" → "todo_write"（与 CapabilityMeta.name 对齐）
            let name = full_name
                .rsplit('/')
                .next()
                .unwrap_or(&full_name)
                .to_string();
            let declared = retention.get(&name).copied();
            info.insert(msg.id.clone(), (idx, name.clone(), declared));
            per_tool_seq.entry(name).or_default().push(msg.id.clone());
        }
    }

    let active_threshold = info.len().saturating_sub(max_active_tool_calls);

    // 判定某个 ToolCall 是否"失效"（需骨架化）：
    // - 全局失效：在全局滑动窗口之外（较新的 max_active_tool_calls 条保留）
    // - 策略失效：该工具声明了保留策略，且此调用不在该工具的最近 N 次内
    let is_stale = |tc_id: &str| -> bool {
        let Some((idx, name, declared)) = info.get(tc_id) else {
            return false;
        };
        if *idx < active_threshold {
            return true;
        }
        let Some(ret) = declared else {
            return false;
        };
        let keep = ret.keep_count() as usize;
        let Some(seq) = per_tool_seq.get(name) else {
            return false;
        };
        match seq.iter().position(|id| id == tc_id) {
            Some(pos) => pos + keep < seq.len(),
            None => false,
        }
    };

    let tool_call_ids_set: HashSet<&String> = info.keys().collect();

    let mut filtered = Vec::with_capacity(messages.len());
    for msg in messages {
        let mut new_msg = msg.clone();

        if msg.msg_type == Some(MessageType::ToolCall) {
            if is_stale(&msg.id) {
                // ToolCall 组合节点自身 content = 请求参数 JSON → 骨架化，
                // 并清理可能遗留的 legacy tool_calls meta。
                new_msg.content = Some(MessageContent::Text(
                    "[System Info: Tool call input parameters skeletonized to save context.]"
                        .to_string(),
                ));
                if let Some(mut meta) = new_msg.meta.as_ref().and_then(|m| m.as_object()).cloned() {
                    meta.remove("tool_calls");
                    new_msg.meta = Some(serde_json::Value::Object(meta));
                }
            }
        } else {
            // 父节点是失效的 ToolCall → 子节点（工具结果/请求参数）骨架化
            let parent_stale = msg
                .parent_id
                .as_ref()
                .map(|pid| tool_call_ids_set.contains(pid) && is_stale(pid))
                .unwrap_or(false);

            if parent_stale {
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
                    "[System Info: Tool call input parameters skeletonized to save context.]"
                        .to_string()
                };
                new_msg.content = Some(MessageContent::Text(label));
            }
        }

        filtered.push(new_msg);
    }

    filtered
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::schemas::session::chat_message::MessageStatus;

    fn tc_msg(id: &str, name: &str, args: &str) -> ChatMessage {
        ChatMessage {
            id: id.to_string(),
            role: Some(MessageRole::Assistant),
            msg_type: Some(MessageType::ToolCall),
            name: Some(name.to_string()),
            content: Some(MessageContent::Text(args.to_string())),
            status: Some(MessageStatus::Completed),
            ..Default::default()
        }
    }

    fn tool_result(parent: &str, text: &str) -> ChatMessage {
        ChatMessage {
            id: format!("{parent}-result"),
            parent_id: Some(parent.to_string()),
            role: Some(MessageRole::Tool),
            msg_type: Some(MessageType::Text),
            content: Some(MessageContent::Text(text.to_string())),
            status: Some(MessageStatus::Completed),
            ..Default::default()
        }
    }

    fn text(content: &str) -> ChatMessage {
        ChatMessage {
            id: format!("t-{}", content),
            role: Some(MessageRole::User),
            msg_type: Some(MessageType::Text),
            content: Some(MessageContent::Text(content.to_string())),
            status: Some(MessageStatus::Completed),
            ..Default::default()
        }
    }

    /// 构建 短工具名 → 保留策略 映射（模拟会话循环运行时从 CapabilityManager 动态解析）
    fn retention_map(entries: &[(&str, ToolContextRetention)]) -> HashMap<String, ToolContextRetention> {
        entries
            .iter()
            .map(|(n, r)| (n.to_string(), *r))
            .collect()
    }

    fn param_of(msg: &ChatMessage) -> String {
        msg.content.as_ref().map(|c| c.to_text()).unwrap_or_default()
    }

    /// LastOnly：同名工具多次调用，仅最新一次参数/结果完整，旧的骨架化
    #[test]
    fn last_only_keeps_only_latest_call_of_same_tool() {
        let messages = vec![
            text("第一轮"),
            tc_msg("tc1", "local/todo_write", r#"{"todos":"v1 很长很长"}"#),
            tool_result("tc1", "ok"),
            tc_msg("tc2", "local/todo_write", r#"{"todos":"v2 很长很长"}"#),
            tool_result("tc2", "ok"),
            text("第二轮"),
        ];
        let ret = retention_map(&[("todo_write", ToolContextRetention::LastOnly)]);
        let out = apply_layered_sliding_window(&messages, 15, &ret);

        let tc1 = out.iter().find(|m| m.id == "tc1").unwrap();
        let tc2 = out.iter().find(|m| m.id == "tc2").unwrap();
        assert!(
            param_of(tc1).contains("skeletonized"),
            "旧调用参数应被骨架化: {:?}",
            tc1.content
        );
        assert_eq!(param_of(tc2), r#"{"todos":"v2 很长很长"}"#, "最新调用参数应原样保留");

        let r1 = out.iter().find(|m| m.parent_id.as_deref() == Some("tc1")).unwrap();
        let r2 = out.iter().find(|m| m.parent_id.as_deref() == Some("tc2")).unwrap();
        assert!(param_of(r1).contains("skeletonized"), "旧结果应被骨架化");
        assert_eq!(param_of(r2), "ok", "最新结果应原样保留");
    }

    /// LastN(2)：保留最近 2 次，更早的骨架化
    #[test]
    fn last_n_keeps_n_latest_calls() {
        let messages = vec![
            tc_msg("a1", "local/search", r#"{"q":"1"}"#),
            tool_result("a1", "r1"),
            tc_msg("a2", "local/search", r#"{"q":"2"}"#),
            tool_result("a2", "r2"),
            tc_msg("a3", "local/search", r#"{"q":"3"}"#),
            tool_result("a3", "r3"),
        ];
        let ret = retention_map(&[("search", ToolContextRetention::LastN(2))]);
        let out = apply_layered_sliding_window(&messages, 15, &ret);

        assert!(param_of(&out.iter().find(|m| m.id == "a1").unwrap()).contains("skeletonized"));
        assert_eq!(param_of(&out.iter().find(|m| m.id == "a2").unwrap()), r#"{"q":"2"}"#);
        assert_eq!(param_of(&out.iter().find(|m| m.id == "a3").unwrap()), r#"{"q":"3"}"#);
    }

    /// 未声明策略的工具不受影响（回归保护：默认行为与旧版一致）
    #[test]
    fn tools_without_retention_are_unaffected() {
        let messages = vec![
            tc_msg("x1", "local/file_read", r#"{"path":"a.rs"}"#),
            tool_result("x1", "content"),
            tc_msg("x2", "local/file_read", r#"{"path":"b.rs"}"#),
            tool_result("x2", "content2"),
        ];
        let ret = HashMap::new();
        let out = apply_layered_sliding_window(&messages, 15, &ret);
        assert_eq!(param_of(&out.iter().find(|m| m.id == "x1").unwrap()), r#"{"path":"a.rs"}"#);
        assert_eq!(param_of(&out.iter().find(|m| m.id == "x2").unwrap()), r#"{"path":"b.rs"}"#);
    }

    /// 全局窗口：超出的 ToolCall 自身参数也骨架化（压缩 tool_call 参数占比）
    #[test]
    fn global_window_skeletonizes_tool_call_params() {
        let mut messages = Vec::new();
        for i in 0..6 {
            messages.push(tc_msg(&format!("g{i}"), "local/file_read", &format!(r#"{{"path":"f{i}.rs","blob":"x".repeat(50)}}"#)));
            messages.push(tool_result(&format!("g{i}"), "done"));
        }
        // 全局窗口只保留最近 2 个
        let ret = HashMap::new();
        let out = apply_layered_sliding_window(&messages, 2, &ret);
        assert!(param_of(&out.iter().find(|m| m.id == "g0").unwrap()).contains("skeletonized"));
        assert!(param_of(&out.iter().find(|m| m.id == "g3").unwrap()).contains("skeletonized"));
        assert!(!param_of(&out.iter().find(|m| m.id == "g4").unwrap()).contains("skeletonized"));
        assert!(!param_of(&out.iter().find(|m| m.id == "g5").unwrap()).contains("skeletonized"));
        // 结果子节点同样骨架化
        let r0 = out.iter().find(|m| m.parent_id.as_deref() == Some("g0")).unwrap();
        assert!(param_of(r0).contains("skeletonized"));
        // 状态保持 Completed（不产生 Failed/孤儿，配对合法）
        assert_eq!(out.iter().find(|m| m.id == "g0").unwrap().status, Some(MessageStatus::Completed));
    }

    /// 工具级 LastOnly 优先于全局窗口（即使仍在全局窗口内也骨架化旧调用）
    #[test]
    fn retention_applies_even_within_global_window() {
        let messages = vec![
            tc_msg("p1", "local/todo_write", r#"{"v":1}"#),
            tool_result("p1", "ok"),
            tc_msg("p2", "local/todo_write", r#"{"v":2}"#),
            tool_result("p2", "ok"),
        ];
        // 全局窗口 15 足够大，但 LastOnly 仍骨架化 p1
        let ret = retention_map(&[("todo_write", ToolContextRetention::LastOnly)]);
        let out = apply_layered_sliding_window(&messages, 15, &ret);
        assert!(param_of(&out.iter().find(|m| m.id == "p1").unwrap()).contains("skeletonized"));
        assert_eq!(param_of(&out.iter().find(|m| m.id == "p2").unwrap()), r#"{"v":2}"#);
    }
}
