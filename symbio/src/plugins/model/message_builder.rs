//! 消息构造与会话持久化
//!
//! 负责：
//! - 构造 ChatMessage（assistant 消息、tool result 消息）
//! - 将对话轮次产生的新消息保存到 session

use std::collections::{HashMap, HashSet};

use super::tool_call::ToolCallInfo;
use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
};

use super::types::*;
use crate::symbio_core::ToolCall;

// Flatten ChatMessage to NativeMessage

/// 将存储的细粒度 ChatMessage 树扁平化，转换为 API 所需的 NativeMessage 列表。
///
/// 分型树结构（请求/响应由 MessageRole 区分，组合节点可选）：
/// - 根级交替：`User`(请求, `Text`) 与 `Turn`(响应, `Assistant` 组合) 互为兄弟
/// - `Turn` 子节点：`Reasoning` / `Text`(回复) / `ToolCall`(`Assistant` 组合)
/// - `ToolCall`：`Assistant` 组合节点，自身 `content` 携带请求参数（JSON 文本）；
///   子节点仅剩 `Text`(响应结果, `Tool`)
///   （响应结果也可包在 `Turn`(`Tool`) 内，视复杂度而定；组合节点可选）
///
/// 扁平化规则：
/// - `Turn` → 父 `assistant` native message（聚合 Reasoning / Text / tool_calls）
/// - `ToolCall` → 从其自身 `content` 读 args 拼入父 `assistant.tool_calls`
/// - 响应结果 `Text`(`Tool`) → 独立的 `role=tool` native message（tool_call_id = ToolCall id）
/// - `User` / `System` 等 → 原样输出
pub fn flatten_chat_messages(messages: &[ChatMessage]) -> Vec<NativeMessage> {
    let by_id: HashMap<&str, &ChatMessage> = messages.iter().map(|m| (m.id.as_str(), m)).collect();
    let mut children: HashMap<&str, Vec<&ChatMessage>> = HashMap::new();
    for m in messages {
        if let Some(pid) = m.parent_id.as_deref() {
            children.entry(pid).or_default().push(m);
        }
    }

    let is_root = |m: &ChatMessage| -> bool {
        m.parent_id
            .as_deref()
            .map(|p| by_id.contains_key(p))
            .unwrap_or(true)
    };

    let mut result: Vec<NativeMessage> = Vec::new();

    // 已聚合消费的节点 id 集合：每个根级 Turn 在聚合时会把自身及全部子孙合并进
    // 一条 assistant 消息。此处仅把「子孙」id 收集进来（不含 Turn 自身），这样：
    //  - 若同一节点在列表中以「裸根」形式（parent_id 为 None 或指向缺失节点）重复出现，
    //    会被跳过，避免 reasoning / tool_call 参数 / tool_result 在 LLM 请求中重复
    //    （即 request.json 中 #5/#6/#7 重复于 #3/#4 的问题）；
    //  - Turn 自身不会被误标记，保证它能被主循环正常聚合（修复此前把 Turn 自身 id
    //    也插入 consumed 导致根级 Turn 被跳过、永不聚合的回归）。
    let mut consumed: HashSet<&str> = HashSet::new();
    /// 递归收集某 Turn 的全部子孙 id（不含 Turn 自身）到 `acc`
    fn collect_descendants<'a>(
        children: &HashMap<&'a str, Vec<&'a ChatMessage>>,
        acc: &mut HashSet<&'a str>,
        id: &'a str,
    ) {
        // 从 Turn 的直接子节点开始递归收集全部子孙 id（不含 Turn 自身）
        let mut stack: Vec<&'a str> = Vec::new();
        if let Some(kids) = children.get(id) {
            for kid in kids {
                stack.push(kid.id.as_str());
            }
        }
        while let Some(cur) = stack.pop() {
            acc.insert(cur);
            if let Some(kids) = children.get(cur) {
                for kid in kids {
                    stack.push(kid.id.as_str());
                }
            }
        }
    }
    for m in messages {
        if is_root(m) && m.msg_type == Some(MessageType::Turn) {
            collect_descendants(&children, &mut consumed, m.id.as_str());
        }
    }

    for m in messages {
        // 已被聚合消费（作为某 Turn 的子节点）的节点不再单独发出
        if consumed.contains(m.id.as_str()) {
            continue;
        }
        if !is_root(m) {
            continue; // 非根节点会在其父节点处理时被合并
        }

        match m.msg_type {
            Some(MessageType::Turn) => {
                let mut native: NativeMessage = m.clone().into();
                native.role = MessageRole::Assistant;
                native.content = None;
                native.reasoning_content = None;
                native.tool_calls = None;

                let mut tool_results: Vec<NativeMessage> = Vec::new();

                if let Some(kids) = children.get(m.id.as_str()) {
                    for child in kids {
                        match child.msg_type {
                            Some(MessageType::Reasoning) => {
                                native.reasoning_content =
                                    child.content.as_ref().map(|c| c.to_text());
                            }
                            Some(MessageType::Text) => {
                                native.content = child.content.clone();
                            }
                            Some(MessageType::ToolCall) => {
                                // 请求参数直接来自 ToolCall 节点自身的 content（JSON 文本）
                                let args_val = child
                                    .content
                                    .as_ref()
                                    .map(|c| c.to_text())
                                    .unwrap_or_default();
                                let args: serde_json::Value = serde_json::from_str(&args_val)
                                    .unwrap_or(serde_json::json!({}));
                                let tc = ToolCall {
                                    id: Some(child.id.clone()),
                                    kind: Some("function".to_string()),
                                    name: child.name.clone().unwrap_or_default(),
                                    arguments: args,
                                };
                                match &mut native.tool_calls {
                                    Some(calls) => calls.push(tc),
                                    None => native.tool_calls = Some(vec![tc]),
                                }

                                // 响应结果：ToolCall 下的 Text(Tool) 直接子节点，或 Turn(Tool) 内 Text
                                if let Some(res) = find_tool_result(&children, child.id.as_str()) {
                                    let mut tool_native: NativeMessage = res.clone().into();
                                    tool_native.role = MessageRole::Tool;
                                    tool_native.tool_call_id = Some(child.id.clone());
                                    tool_results.push(tool_native);
                                }
                            }
                            _ => {}
                        }
                    }
                }

                // 注：本 Turn 的全部子孙 id 已在预处理遍（consumed 预扫描）中收集，
                // 主循环开头的 `if consumed.contains(m.id) { continue; }` 会跳过它们，
                // 因此此处无需再单独标记，避免重复代码与潜在误标记。

                result.push(native);
                result.extend(tool_results);
            }
            _ => {
                // User / System 等根级内容节点
                let mut native: NativeMessage = m.clone().into();
                if native.role == MessageRole::Tool {
                    native.tool_call_id = m.parent_id.clone();
                }
                result.push(native);
            }
        }
    }

    result
}

/// 在 ToolCall 下查找响应结果节点：
/// 1. 直接的 `Text`(role=Tool) 子节点（非空内容优先）
/// 2. 或被 `Turn`(role=Tool) 包装的 `Text` 子节点（组合节点可选时的兼容形式，
///    子 agent 会话即以此种方式返回完整会话）
/// 3. 兜底：任意直接的 `Text`(role=Tool) 子节点
fn find_tool_result<'a>(
    children: &HashMap<&str, Vec<&'a ChatMessage>>,
    tool_call_id: &str,
) -> Option<&'a ChatMessage> {
    let kids = children.get(tool_call_id)?;
    // 1. 优先直接 Text(Tool) 且内容非空
    if let Some(direct) = kids.iter().find(|c| {
        c.role == Some(MessageRole::Tool)
            && c.msg_type == Some(MessageType::Text)
            && c.content.as_ref().map(|x| !x.is_empty()).unwrap_or(false)
    }) {
        return Some(direct);
    }
    // 2. 被 Turn(Tool) 包装（子 agent 会话）
    if let Some(turn) = kids
        .iter()
        .find(|c| c.msg_type == Some(MessageType::Turn) && c.role == Some(MessageRole::Tool))
    {
        if let Some(tkids) = children.get(turn.id.as_str()) {
            return tkids
                .iter()
                .find(|c| c.msg_type == Some(MessageType::Text))
                .copied();
        }
    }
    // 3. 兜底：任意直接 Text(Tool)
    kids.iter()
        .find(|c| c.role == Some(MessageRole::Tool) && c.msg_type == Some(MessageType::Text))
        .copied()
}

// ChatMessage 构造

/// 生成长度短的 ID（8 字符），替代完整 UUID v4
pub fn short_id() -> String {
    uuid::Uuid::new_v4().to_string()[..8].to_string()
}

/// 构造助手消息组（基于 Turn / ToolCall 的分型层级结构）。
///
/// 结构：
/// - `Turn`(根级, `Assistant` 组合)：与 `User` 互为兄弟
///   ├─ `Reasoning`(子)
///   ├─ `Text`(回复, 子)
/// - `ToolCall`(`Assistant` 组合, 子)：自身 `content` 携带请求参数（JSON 文本）
///   └─ `Text`(响应结果, `Tool`, 子)  ← 由 `build_tool_message` 补充
pub fn build_assistant_messages(
    id: &str,
    content: &str,
    tool_calls: &[ToolCallInfo],
    rid: Option<String>,
    reasoning: Option<String>,
) -> Vec<ChatMessage> {
    let mut msgs = Vec::new();
    let timestamp = (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64;

    // ── Turn 消息（根级，与 User 互为兄弟）───────────────────────────────
    msgs.push(ChatMessage {
        id: id.to_string(),
        parent_id: None,
        role: Some(MessageRole::Assistant),
        msg_type: Some(MessageType::Turn),
        content: None,
        status: Some(MessageStatus::Completed),
        timestamp: Some(timestamp),
        ..Default::default()
    });

    // ── Reasoning 消息（parent_id=turn_id）──────────────────────────────
    // 仅当 reasoning 与回复正文为「不同内容」（即存在独立的文本回复）时才单独生成思考子节点。
    // reasoning-only 场景下 `reasoning` 已通过 `content`（effective_text 对「无文本回复」的回退）
    // 承载于下方的 Text 子节点；若此处再生成 Reasoning 子节点，同一段内容会在存储层出现两份
    // （factor=2：表现为历史会话里重复两份、流式期间"层层叠加"）。
    let reasoning_only = reasoning
        .as_ref()
        .map(|r| !r.trim().is_empty() && r.trim() == content.trim())
        .unwrap_or(false);
    if let Some(r) = reasoning {
        if !r.trim().is_empty() && !reasoning_only {
            msgs.push(ChatMessage {
                id: short_id(),
                parent_id: Some(id.to_string()),
                role: Some(MessageRole::Assistant),
                msg_type: Some(MessageType::Reasoning),
                content: Some(MessageContent::Text(r)),
                status: Some(MessageStatus::Completed),
                timestamp: Some(timestamp),
                ..Default::default()
            });
        }
    }

    // ── Response 文本消息（parent_id=turn_id）───────────────────────────
    // 仅在存在非空白文本内容时添加，避免产生仅含 \n\n 的空节点
    if !content.trim().is_empty() {
        msgs.push(ChatMessage {
            id: short_id(),
            parent_id: Some(id.to_string()),
            role: Some(MessageRole::Assistant),
            msg_type: Some(MessageType::Text),
            content: Some(MessageContent::Text(content.into())),
            status: Some(MessageStatus::Completed),
            timestamp: Some(timestamp),
            response_id: rid,
            ..Default::default()
        });
    }

    // ── ToolCall 消息（parent_id=turn_id，组合节点）──────────────────────
    // ToolCall 组合节点自身携带请求参数（content = JSON 文本），不再拆分出独立的请求子节点
    for tc in tool_calls {
        let tc_id = tc.id.clone().unwrap_or_else(short_id);
        msgs.push(ChatMessage {
            id: tc_id.clone(),
            parent_id: Some(id.to_string()),
            role: Some(MessageRole::Assistant),
            msg_type: Some(MessageType::ToolCall),
            name: tc.name.clone(),
            content: Some(MessageContent::Text(tc.arguments.to_string())),
            status: Some(MessageStatus::Completed),
            timestamp: Some(timestamp),
            ..Default::default()
        });
    }

    msgs
}

/// 构造工具执行结果消息（role: Tool，msg_type: Text，parent_id 指向 tool_call）。
/// 响应结果作为 `ToolCall` 的直接 `Text`(`Tool`) 子节点（组合节点可选，故不包 Turn）。
///
/// **重要（修复 Bug 2）**：结果子节点的 `status` 必须与实际执行结果一致——
/// 成功 `Completed`、失败 `Failed`。此前硬编码 `Completed`，导致失败工具的结果
/// 子节点被持久化为 `Completed`；当下一轮 `get_context_messages` 过滤掉 `Failed`
/// 的 `ToolCall` 父节点时，这个"孤儿"`role=Tool` 结果子节点（其 `tool_call_id`
/// 指向已被删除的 tool_call）被保留下来，使新一轮 LLM 请求携带非法
/// `tool_call_id` → 请求包出错（"发给大语言模型的数据包会出错"）。
pub fn build_tool_message(
    tool_call_id: &str,
    content: &str,
    // rid: Option<String>,
    success: Option<bool>,
    msg_id: Option<String>,
) -> ChatMessage {
    let success = success.unwrap_or(true);
    ChatMessage {
        id: msg_id.unwrap_or_else(short_id),
        parent_id: Some(tool_call_id.into()),
        role: Some(MessageRole::Tool),
        msg_type: Some(MessageType::Text),
        content: Some(MessageContent::Text(content.into())),
        status: Some(if success {
            MessageStatus::Completed
        } else {
            MessageStatus::Failed
        }),
        meta: Some(serde_json::json!({ "success": success })),
        timestamp: Some(
            (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as i64,
        ),
        // response_id: rid,
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TURN_ID: &str = "turn-0001";

    /// 统计某类型子节点（parent_id == TURN_ID）数量
    fn count_children(msgs: &[ChatMessage], ty: MessageType) -> usize {
        msgs.iter()
            .filter(|m| m.parent_id.as_deref() == Some(TURN_ID) && m.msg_type == Some(ty.clone()))
            .count()
    }

    fn child_texts(msgs: &[ChatMessage], ty: MessageType) -> Vec<String> {
        msgs.iter()
            .filter(|m| m.parent_id.as_deref() == Some(TURN_ID) && m.msg_type == Some(ty.clone()))
            .map(|m| m.content.as_ref().map(|c| c.to_text()).unwrap_or_default())
            .collect()
    }

    fn tool_call(name: &str) -> ToolCallInfo {
        ToolCallInfo {
            id: Some("tc-1".to_string()),
            name: Some(name.to_string()),
            arguments: serde_json::json!({ "k": "v" }),
        }
    }

    /// 用例 A（回归：storage factor≈2 重复）
    ///
    /// reasoning-only 场景（无独立文本回复，`effective_text` 回退为 reasoning）：
    /// 只能落 `Turn` + 一个 `Text` 子节点；**绝不能**再生成 `Reasoning` 子节点，
    /// 否则同一段 reasoning 在存储层出现两份（历史会话打开显示重复两份）。
    #[test]
    fn reasoning_only_writes_single_text_child_without_reasoning_node() {
        let reasoning = "我先分析一下用户的问题，然后给出结论。";

        let msgs = build_assistant_messages(TURN_ID, reasoning, &[], None, Some(reasoning.into()));

        // 恰好 2 个节点：Turn(根) + 1 个 Text 子节点
        assert_eq!(msgs.len(), 2, "reasoning-only 应只落 Turn + Text 两个节点");

        // 根节点
        assert_eq!(msgs[0].id, TURN_ID);
        assert_eq!(msgs[0].msg_type, Some(MessageType::Turn));
        assert_eq!(msgs[0].parent_id, None);
        assert!(msgs[0].content.is_none(), "Turn 为组合节点，不携带内容");

        // 关键防复发断言：不存在任何 Reasoning 子节点
        assert_eq!(
            count_children(&msgs, MessageType::Reasoning),
            0,
            "reasoning-only 不得生成 Reasoning 子节点（否则与 Text 子节点内容重复）"
        );

        // 唯一的 Text 子节点承载 reasoning 内容，且只出现一次
        let texts = child_texts(&msgs, MessageType::Text);
        assert_eq!(texts.len(), 1, "Text 子节点应恰好一个");
        assert_eq!(texts[0], reasoning);
    }

    /// 用例 A2：reasoning 与正文仅首尾空白不同，仍应判定为 reasoning-only（trim 比较）
    #[test]
    fn reasoning_only_ignores_surrounding_whitespace() {
        let reasoning = "思考内容";
        let content = "\n  思考内容  \n";

        let msgs = build_assistant_messages(TURN_ID, content, &[], None, Some(reasoning.into()));

        assert_eq!(msgs.len(), 2);
        assert_eq!(count_children(&msgs, MessageType::Reasoning), 0);
        assert_eq!(count_children(&msgs, MessageType::Text), 1);
    }

    /// 用例 B：普通场景（reasoning + 独立文本回复）→ Reasoning 与 Text 各一份，内容不同
    #[test]
    fn reasoning_with_distinct_reply_keeps_both_children() {
        let msgs = build_assistant_messages(
            TURN_ID,
            "正常回复",
            &[],
            Some("resp-1".into()),
            Some("思考过程".into()),
        );

        // Turn + Reasoning + Text
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].msg_type, Some(MessageType::Turn));

        let reasonings = child_texts(&msgs, MessageType::Reasoning);
        let texts = child_texts(&msgs, MessageType::Text);
        assert_eq!(reasonings, vec!["思考过程".to_string()]);
        assert_eq!(texts, vec!["正常回复".to_string()]);
        assert_ne!(reasonings[0], texts[0], "两个子节点内容必须不同");

        // response_id 只挂在 Text 响应节点上
        let text_child = msgs
            .iter()
            .find(|m| m.msg_type == Some(MessageType::Text))
            .unwrap();
        assert_eq!(text_child.response_id.as_deref(), Some("resp-1"));
        assert_eq!(text_child.role, Some(MessageRole::Assistant));
        assert_eq!(text_child.status, Some(MessageStatus::Completed));
    }

    /// 用例 C：无 reasoning 的纯文本回复 → Turn + Text
    #[test]
    fn plain_text_reply_has_no_reasoning_child() {
        let msgs = build_assistant_messages(TURN_ID, "你好", &[], None, None);
        assert_eq!(msgs.len(), 2);
        assert_eq!(count_children(&msgs, MessageType::Reasoning), 0);
        assert_eq!(child_texts(&msgs, MessageType::Text), vec!["你好"]);
    }

    /// 用例 D：有 reasoning + 无文本 + 有工具调用（非 reasoning-only）
    /// → 保留 Reasoning 子节点，且不生成空 Text 节点
    #[test]
    fn reasoning_with_tool_calls_keeps_reasoning_and_skips_empty_text() {
        let tools = vec![tool_call("read_file")];
        let msgs = build_assistant_messages(TURN_ID, "", &tools, None, Some("要先读文件".into()));

        // Turn + Reasoning + ToolCall
        assert_eq!(msgs.len(), 3);
        assert_eq!(count_children(&msgs, MessageType::Reasoning), 1);
        assert_eq!(
            count_children(&msgs, MessageType::Text),
            0,
            "空白正文不得生成 Text 节点"
        );
        assert_eq!(count_children(&msgs, MessageType::ToolCall), 1);
    }

    /// 用例 E：reasoning-only 落库结果扁平化后，LLM 请求里内容只出现一次
    /// （防止重复内容顺着 request 包再放大一次）
    #[test]
    fn reasoning_only_flattens_to_single_assistant_message() {
        let reasoning = "只有思考";
        let msgs = build_assistant_messages(TURN_ID, reasoning, &[], None, Some(reasoning.into()));

        let natives = flatten_chat_messages(&msgs);
        assert_eq!(natives.len(), 1, "应只产生一条 assistant native message");
        assert_eq!(natives[0].role, MessageRole::Assistant);
        assert_eq!(
            natives[0].content.as_ref().map(|c| c.to_text()),
            Some(reasoning.to_string())
        );
        assert!(
            natives[0].reasoning_content.is_none(),
            "reasoning-only 场景不应再额外携带 reasoning_content（内容已在 content 中）"
        );
    }

    /// 用例 F：普通 reasoning + 文本 扁平化后 content 与 reasoning_content 各归其位
    #[test]
    fn reasoning_with_reply_flattens_into_content_and_reasoning_content() {
        let msgs =
            build_assistant_messages(TURN_ID, "正常回复", &[], None, Some("思考过程".into()));

        let natives = flatten_chat_messages(&msgs);
        assert_eq!(natives.len(), 1);
        assert_eq!(
            natives[0].content.as_ref().map(|c| c.to_text()),
            Some("正常回复".to_string())
        );
        assert_eq!(natives[0].reasoning_content.as_deref(), Some("思考过程"));
    }
}
