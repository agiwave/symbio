//! **跨轮降级**（三段之一）：
//! 树上缺东西时**补什么**，以及补完还要**扔什么**。
//!
//! 这一段是「继续会话」中断可见性的全部落点：模型在上下文里看到的一定是
//! 「上次输出 → 中断说明 → 用户新消息」的连贯序列，而不是半截输出或一条
//! 没有结果的 `tool_call`。事故历史（e2e `t27-approval-resume` 的恢复轮、
//! `t38-transcript-tool-projection` 的跨轮工具结果）都落在这里，它们的具名判据
//! 走 `flatten_chat_messages` 这个入口测（`message_builder.test.rs`）——
//! 补件与扔件都只在整包形状上才看得出对错。
//!
//! 与 [`super::shape`] 的分界：形状换算只管「有这个东西时长成什么样」，
//! 这里管「这个东西缺了怎么办」。搬动判据时不要把它们混进同一个函数——
//! 混了以后，「降级没生效」与「聚合写错了」在读面上是同一条红。

use std::collections::HashSet;

use crate::types::*;
use symbio_core::chat_node::{ChatNode, NodeRole, NodeStatus};

/// ToolCall 没有对应结果（执行被打断，未产生结果）时合成的占位 tool 结果。
///
/// **为什么必须有**：请求包出现「有 `tool_call` 无 `tool_result`」会让 provider 直接 400，
/// 一整轮恢复不了；占位文本让模型看得见「这个工具被打断了」，而不是让它自己猜。
/// 调用点必须在聚合遍内把它收进 `tool_results`（先于 [`clean_request`] 的孤儿剔除），
/// 否则它会被当作孤儿 tool 结果误删——这条顺序是 e2e `t27-approval-resume` 依赖的形状。
pub(super) fn placeholder_tool_result(tool_name: &str, wire_id: Option<String>) -> NativeMessage {
    NativeMessage {
        role: NodeRole::Tool,
        tool_call_id: wire_id,
        content: Some(MessageContent::Text(format!(
            "[工具 {tool_name} 的执行被中断，未产生结果。]"
        ))),
        ..Default::default()
    }
}

/// 失败的工具结果推导 `success=false`
///
/// 触发 Anthropic tool_result 的 `is_error=true`，工具失败信息才能被模型感知；
/// **跨轮工具失败必须可见，不得被过滤丢失**。非失败的节点不动这一格（留 `None`，
/// 由各协议自己按默认值处理）。
pub(super) fn mark_failed_result(native: &mut NativeMessage, result: &ChatNode) {
    if result.status == Some(NodeStatus::Failed) {
        native.success = Some(false);
    }
}

/// 失败 Turn 的**中断说明**段落：半截输出原样聚合后附加在正文尾部。
///
/// 模型在上下文中看到「上次输出 → 中断说明 → 用户新消息」的连贯序列，避免思维链
/// 断裂（等价于用户打断了模型说话，然后继续）。有 `error` 就把原因带进说明——
/// 「为什么断」比「断了」信息量大，而它已经在树上（`ChatNode::error`），没有理由不印。
pub(super) fn append_interrupt_note(native: &mut NativeMessage, turn: &ChatNode) {
    let note = match turn.error.as_deref() {
        Some(err) if !err.trim().is_empty() => format!(
            "[本轮回复被中断（原因：{err}）。以上是中断前的部分输出，仅供参考；请结合用户接下来的消息继续。]"
        ),
        _ => "[本轮回复被中断。以上是中断前的部分输出，仅供参考；请结合用户接下来的消息继续。]"
            .to_string(),
    };
    native.content = Some(match native.content.take() {
        Some(MessageContent::Text(text)) if !text.is_empty() => {
            MessageContent::Text(format!("{text}\n\n{note}"))
        }
        _ => MessageContent::Text(note),
    });
}

/// 请求包清洗（避免把脏消息发到 provider 触发反序列化失败）
///
/// 两条扔法各自的后果：
/// 1. **丢「无对应 tool_call 的 tool 结果」**——provider 要求 tool 角色的 content 必须
///    关联到前文某个 assistant 的 `tool_calls`，否则直接 400；
/// 2. **丢「空 assistant 回合」**——既无文本/推理内容、也无工具调用，发过去是无意义且
///    可能触发校验错误的占位消息。
pub(super) fn clean_request(result: Vec<NativeMessage>) -> Vec<NativeMessage> {
    // 1. 收集本批所有 tool_call id（用于识别孤儿 tool 结果）。
    let mut tool_call_ids: HashSet<String> = HashSet::new();
    for nm in &result {
        if let Some(calls) = &nm.tool_calls {
            for tc in calls {
                if let Some(id) = &tc.id {
                    tool_call_ids.insert(id.clone());
                }
            }
        }
    }
    let mut cleaned: Vec<NativeMessage> = Vec::with_capacity(result.len());
    for nm in result {
        if nm.role == NodeRole::Tool {
            // 2. 孤儿 tool 结果（没有对应 tool_call，或压根没带 id）不发
            match &nm.tool_call_id {
                Some(tid) if tool_call_ids.contains(tid) => {}
                _ => continue,
            }
        }
        // 3. 空 assistant 回合不发
        if nm.role == NodeRole::Assistant
            && nm.content.as_ref().map(|c| c.is_empty()).unwrap_or(true)
            && nm.reasoning_content.is_none()
            && nm.tool_calls.is_none()
        {
            continue;
        }
        cleaned.push(nm);
    }
    cleaned
}
