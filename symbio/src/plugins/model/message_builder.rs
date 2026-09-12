//! 消息构造与会话持久化
//!
//! 负责：
//! - 构造 ChatMessage（assistant 消息、tool result 消息）
//! - 将对话轮次产生的新消息保存到 session

use std::collections::{HashMap, HashSet};

use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
};

use super::types::*;

// Flatten ChatMessage to NativeMessage

/// LLM 请求包中保留的最近思考条数。
///
/// 思考链的价值随距离衰减：模型只需最近几条推理来"接上思路"，
/// 全量回传历史思考既浪费 token，也可能诱导模型复述旧结论。
/// 存储层 Reasoning 节点完整保留，此处只裁剪请求视图
/// （Anthropic 协议本就不回传历史思考——缺官方签名，此裁剪对它无副作用）。
const RETAINED_RECENT_REASONING: usize = 2;

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
/// - 失败 `Turn`（status=Failed）：半截输出照常聚合，并附加中断说明段落；
///   无结果的 `ToolCall` 合成占位 tool 结果、失败的工具结果推导 `success=false`
///   （"继续会话"中断可见性）
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
    //    会被跳过，避免 reasoning / tool_call 参数 / tool_result 在 LLM 请求中重复；
    //  - Turn 自身不会被误标记，保证它能被主循环正常聚合（把 Turn 自身 id 也收进
    //    consumed 会让根级 Turn 被跳过、永不聚合）。
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

    // ── 请求视图思考裁剪 ─────────────────────────────────────────────────
    // 按列表序（时间序）收集携带 Reasoning 子节点的根级 Turn，
    // 仅最近 RETAINED_RECENT_REASONING 条允许回传 reasoning_content：
    // 完全清空思考会让模型"失忆"后重复思考；全量回传历史思考既浪费
    // token，也可能诱导模型复述旧结论。只裁请求视图，存储层完整保留。
    let turns_with_reasoning: Vec<&str> = messages
        .iter()
        .filter(|m| {
            is_root(m)
                && m.msg_type == Some(MessageType::Turn)
                && children
                    .get(m.id.as_str())
                    .map(|kids| {
                        kids.iter()
                            .any(|c| c.msg_type == Some(MessageType::Reasoning))
                    })
                    .unwrap_or(false)
        })
        .map(|m| m.id.as_str())
        .collect();
    let retained_reasoning_turns: HashSet<&str> = turns_with_reasoning
        .iter()
        .rev()
        .take(RETAINED_RECENT_REASONING)
        .copied()
        .collect();

    for m in messages {
        // 已被聚合消费（作为某 Turn 的子节点）的节点不单独发出
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
                                // 仅最近 N 条思考进入请求视图
                                if retained_reasoning_turns.contains(m.id.as_str()) {
                                    native.reasoning_content =
                                        child.content.as_ref().map(|c| c.to_text());
                                }
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
                                match find_tool_result(&children, child.id.as_str()) {
                                    Some(res) => {
                                        let mut tool_native: NativeMessage = res.clone().into();
                                        tool_native.role = MessageRole::Tool;
                                        tool_native.tool_call_id = Some(child.id.clone());
                                        // 失败的工具结果推导 success=false：触发 Anthropic
                                        // tool_result 的 is_error=true，工具失败信息才能被
                                        // 模型感知；跨轮工具失败必须可见，不得被过滤丢失
                                        if res.status == Some(MessageStatus::Failed) {
                                            tool_native.success = Some(false);
                                        }
                                        tool_results.push(tool_native);
                                    }
                                    None => {
                                        // ToolCall 无对应结果（执行被打断，未产生结果）：
                                        // 合成占位 tool 结果，避免请求包出现"有 tool_call
                                        // 无 tool_result"触发 provider 400。必须在此处
                                        // 进入 tool_results（先于尾部孤儿剔除），否则会被
                                        // 当作孤儿 tool 结果误删。
                                        let tool_name = child.name.clone().unwrap_or_default();
                                        tool_results.push(NativeMessage {
                                            role: MessageRole::Tool,
                                            tool_call_id: Some(child.id.clone()),
                                            content: Some(MessageContent::Text(format!(
                                                "[工具 {tool_name} 的执行被中断，未产生结果。]"
                                            ))),
                                            ..Default::default()
                                        });
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }

                // 注：本 Turn 的全部子孙 id 已在预处理遍（consumed 预扫描）中收集，
                // 主循环开头的 `if consumed.contains(m.id) { continue; }` 会跳过它们，
                // 因此此处无需再单独标记，避免重复代码与潜在误标记。

                // **中断说明**（"继续会话"中断可见性）：
                // 失败 Turn 的半截输出原样聚合后，附加中断说明段落——模型在上下文中
                // 看到"上次输出 → 中断说明 → 用户新消息"的连贯序列，避免思维链断裂
                // （等价于用户打断了模型说话，然后继续）。
                if m.status == Some(MessageStatus::Failed) {
                    let note = match m.error.as_deref() {
                        Some(err) if !err.trim().is_empty() => format!(
                            "[本轮回复被中断（原因：{err}）。以上是中断前的部分输出，仅供参考；请结合用户接下来的消息继续。]"
                        ),
                        _ => "[本轮回复被中断。以上是中断前的部分输出，仅供参考；请结合用户接下来的消息继续。]"
                            .to_string(),
                    };
                    native.content = Some(match native.content {
                        Some(MessageContent::Text(text)) if !text.is_empty() => {
                            MessageContent::Text(format!("{text}\n\n{note}"))
                        }
                        _ => MessageContent::Text(note),
                    });
                }

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

    // ── 请求包清洗（避免把脏消息发到 provider 触发反序列化失败）────────────
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
        // 2. 丢弃"无对应 tool_call 的 tool 结果"：provider 要求 tool 角色的
        //    content 必须关联到前文某个 assistant 的 tool_calls，否则直接 400。
        if nm.role == MessageRole::Tool {
            if let Some(tid) = &nm.tool_call_id {
                if !tool_call_ids.contains(tid) {
                    continue;
                }
            } else {
                continue;
            }
        }
        // 3. 丢弃"空 assistant 回合"：既无文本/推理内容、也无工具调用，
        //    发到 provider 是无意义且可能触发校验错误的占位消息。
        if nm.role == MessageRole::Assistant
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

// ChatMessage 构造（build_assistant_messages / build_tool_message / short_id /
// StreamChildIds）实现在 symbio_core::turn：本文件仅测试消费，由 tests 模块
// 直接引用 core（避免 lib 侧 unused import 警告）

#[cfg(test)]
mod tests {
    use super::*;
    use crate::symbio_core::schemas::session::chat_message::MessageStatus;
    use crate::symbio_core::turn::{build_assistant_messages, StreamChildIds, ToolCallInfo};

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

        let msgs = build_assistant_messages(
            TURN_ID,
            reasoning,
            &[],
            None,
            Some(reasoning.into()),
            StreamChildIds::default(),
        );

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

        let msgs = build_assistant_messages(
            TURN_ID,
            content,
            &[],
            None,
            Some(reasoning.into()),
            StreamChildIds::default(),
        );

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
            StreamChildIds::default(),
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
        let msgs =
            build_assistant_messages(TURN_ID, "你好", &[], None, None, StreamChildIds::default());
        assert_eq!(msgs.len(), 2);
        assert_eq!(count_children(&msgs, MessageType::Reasoning), 0);
        assert_eq!(child_texts(&msgs, MessageType::Text), vec!["你好"]);
    }

    /// 用例 D：有 reasoning + 无文本 + 有工具调用（非 reasoning-only）
    /// → 保留 Reasoning 子节点，且不生成空 Text 节点
    #[test]
    fn reasoning_with_tool_calls_keeps_reasoning_and_skips_empty_text() {
        let tools = vec![tool_call("read_file")];
        let msgs = build_assistant_messages(
            TURN_ID,
            "",
            &tools,
            None,
            Some("要先读文件".into()),
            StreamChildIds::default(),
        );

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
        let msgs = build_assistant_messages(
            TURN_ID,
            reasoning,
            &[],
            None,
            Some(reasoning.into()),
            StreamChildIds::default(),
        );

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
        let msgs = build_assistant_messages(
            TURN_ID,
            "正常回复",
            &[],
            None,
            Some("思考过程".into()),
            StreamChildIds::default(),
        );

        let natives = flatten_chat_messages(&msgs);
        assert_eq!(natives.len(), 1);
        assert_eq!(
            natives[0].content.as_ref().map(|c| c.to_text()),
            Some("正常回复".to_string())
        );
        assert_eq!(natives[0].reasoning_content.as_deref(), Some("思考过程"));
    }

    /// 请求视图只保留最近 RETAINED_RECENT_REASONING 条思考。
    ///
    /// 3 个携带 Reasoning 子节点的 Turn 依序出现时，最早 1 条的 reasoning_content
    /// 必须被剥离（但正文保留、消息不得整条消失），最近 2 条完整回传——
    /// 防止上下文完全没有思考时模型重复思考，同时避免全量回传浪费 token。
    #[test]
    fn flatten_keeps_only_recent_reasoning_in_request_view() {
        let mk_turn = |turn_id: &str, reasoning: &str, reply: &str| -> Vec<ChatMessage> {
            vec![
                ChatMessage {
                    id: turn_id.into(),
                    parent_id: None,
                    role: Some(MessageRole::Assistant),
                    msg_type: Some(MessageType::Turn),
                    content: None,
                    status: Some(MessageStatus::Completed),
                    timestamp: Some(1),
                    ..Default::default()
                },
                ChatMessage {
                    id: format!("{turn_id}-reason"),
                    parent_id: Some(turn_id.into()),
                    role: Some(MessageRole::Assistant),
                    msg_type: Some(MessageType::Reasoning),
                    content: Some(MessageContent::Text(reasoning.into())),
                    status: Some(MessageStatus::Completed),
                    timestamp: Some(1),
                    ..Default::default()
                },
                ChatMessage {
                    id: format!("{turn_id}-text"),
                    parent_id: Some(turn_id.into()),
                    role: Some(MessageRole::Assistant),
                    msg_type: Some(MessageType::Text),
                    content: Some(MessageContent::Text(reply.into())),
                    status: Some(MessageStatus::Completed),
                    timestamp: Some(1),
                    ..Default::default()
                },
            ]
        };

        let mut msgs = vec![ChatMessage {
            id: "u1".into(),
            parent_id: None,
            role: Some(MessageRole::User),
            msg_type: Some(MessageType::Text),
            content: Some(MessageContent::Text("开始".into())),
            status: Some(MessageStatus::Completed),
            timestamp: Some(0),
            ..Default::default()
        }];
        msgs.extend(mk_turn("turn-1", "思考一", "回复一"));
        msgs.extend(mk_turn("turn-2", "思考二", "回复二"));
        msgs.extend(mk_turn("turn-3", "思考三", "回复三"));

        let natives = flatten_chat_messages(&msgs);

        let assistant: Vec<&NativeMessage> = natives
            .iter()
            .filter(|n| n.role == MessageRole::Assistant)
            .collect();
        assert_eq!(assistant.len(), 3, "三个 Turn 都应聚合为 assistant 消息");

        let by_content = |reply: &str| -> &NativeMessage {
            assistant
                .iter()
                .copied()
                .find(|n| n.content.as_ref().map(|c| c.to_text()) == Some(reply.to_string()))
                .unwrap_or_else(|| panic!("应存在正文为 {reply} 的 assistant 消息"))
        };

        let first = by_content("回复一");
        assert!(
            first.reasoning_content.is_none(),
            "最早一条思考应被裁剪出请求视图"
        );
        assert_eq!(
            by_content("回复二").reasoning_content.as_deref(),
            Some("思考二")
        );
        assert_eq!(
            by_content("回复三").reasoning_content.as_deref(),
            Some("思考三")
        );
    }

    // ── 回归测试：锁定以下高危行为 ────────────────────────────────────────

    /// 落库节点必须复用流式子节点 id。
    ///
    /// 若两处各自 `short_id()`，存储层的定稿节点（id=B，内容全量）与会话层累积的流式节点
    /// （id=A，内容增量合并）会被判定为两条不同消息；失败收尾时 id=A 被当作"尚未落库的
    /// 流式半截"补写进存储 → 同一个 Turn 下出现两份内容相同的文本节点。
    #[test]
    fn persisted_children_reuse_streaming_child_ids() {
        let msgs = build_assistant_messages(
            TURN_ID,
            "正常回复",
            &[],
            None,
            Some("思考过程".into()),
            StreamChildIds {
                text: Some("stream-text-id".into()),
                reasoning: Some("stream-reason-id".into()),
            },
        );

        let text_child = msgs
            .iter()
            .find(|m| m.msg_type == Some(MessageType::Text))
            .expect("应生成 Text 子节点");
        let reasoning_child = msgs
            .iter()
            .find(|m| m.msg_type == Some(MessageType::Reasoning))
            .expect("应生成 Reasoning 子节点");

        assert_eq!(text_child.id, "stream-text-id");
        assert_eq!(reasoning_child.id, "stream-reason-id");
    }

    /// reasoning-only 场景：正文由 reasoning 承载（不生成 Reasoning 子节点），
    /// 落库的 Text 节点应复用 reasoning 的流式 id——流式层定稿的正是该节点。
    #[test]
    fn reasoning_only_reuses_reasoning_stream_id() {
        let reasoning = "只有思考";
        let msgs = build_assistant_messages(
            TURN_ID,
            reasoning,
            &[],
            None,
            Some(reasoning.into()),
            StreamChildIds {
                text: None,
                reasoning: Some("stream-reason-id".into()),
            },
        );

        let text_child = msgs
            .iter()
            .find(|m| m.msg_type == Some(MessageType::Text))
            .expect("reasoning-only 应生成唯一 Text 子节点");
        assert_eq!(text_child.id, "stream-reason-id");
    }

    /// 孤儿 tool 结果（parent 指向已被 get_context_messages 过滤掉的 tool_call）不得进入
    /// LLM 请求包。复现 Bug：父 ToolCall 被过滤后，其 role=Tool 结果子节点若仍被保留，
    /// 会携带非法 tool_call_id → provider 报 400 "messages[N]: data did not match any variant"。
    #[test]
    fn orphan_tool_result_is_dropped() {
        let turn_id = "turn-orphan";
        let dead_tc_id = "tc-already-filtered"; // 该 tool_call 已被上下文过滤，列表中不存在

        let msgs = vec![
            ChatMessage {
                id: "u1".into(),
                parent_id: None,
                role: Some(MessageRole::User),
                msg_type: Some(MessageType::Text),
                content: Some(MessageContent::Text("hello".into())),
                status: Some(MessageStatus::Completed),
                timestamp: Some(1),
                ..Default::default()
            },
            ChatMessage {
                id: turn_id.into(),
                parent_id: None,
                role: Some(MessageRole::Assistant),
                msg_type: Some(MessageType::Turn),
                content: None,
                status: Some(MessageStatus::Completed),
                timestamp: Some(2),
                ..Default::default()
            },
            ChatMessage {
                id: "r1".into(),
                parent_id: Some(turn_id.into()),
                role: Some(MessageRole::Assistant),
                msg_type: Some(MessageType::Text),
                content: Some(MessageContent::Text("ok".into())),
                status: Some(MessageStatus::Completed),
                timestamp: Some(2),
                ..Default::default()
            },
            // 孤儿工具结果：parent_id 指向已不存在的 tool_call
            ChatMessage {
                id: "orphan-tool".into(),
                parent_id: Some(dead_tc_id.into()),
                role: Some(MessageRole::Tool),
                msg_type: Some(MessageType::Text),
                content: Some(MessageContent::Text("stale result".into())),
                status: Some(MessageStatus::Completed),
                timestamp: Some(3),
                ..Default::default()
            },
        ];

        let natives = flatten_chat_messages(&msgs);
        assert!(
            natives.iter().all(|n| n.role != MessageRole::Tool),
            "孤儿 tool 结果不得进入 LLM 请求包"
        );
    }

    /// 反向控制：合法关联（Turn → ToolCall → Tool 结果）的 tool 结果必须保留，
    /// 防止清洗逻辑误伤正常链路。
    #[test]
    fn linked_tool_result_is_retained() {
        let turn_id = "turn-linked";
        let tc_id = "tc-linked";

        let msgs = vec![
            ChatMessage {
                id: "u1".into(),
                parent_id: None,
                role: Some(MessageRole::User),
                msg_type: Some(MessageType::Text),
                content: Some(MessageContent::Text("read?".into())),
                status: Some(MessageStatus::Completed),
                timestamp: Some(1),
                ..Default::default()
            },
            ChatMessage {
                id: turn_id.into(),
                parent_id: None,
                role: Some(MessageRole::Assistant),
                msg_type: Some(MessageType::Turn),
                content: None,
                status: Some(MessageStatus::Completed),
                timestamp: Some(2),
                ..Default::default()
            },
            ChatMessage {
                id: "r1".into(),
                parent_id: Some(turn_id.into()),
                role: Some(MessageRole::Assistant),
                msg_type: Some(MessageType::Text),
                content: Some(MessageContent::Text("let me read".into())),
                status: Some(MessageStatus::Completed),
                timestamp: Some(2),
                ..Default::default()
            },
            ChatMessage {
                id: tc_id.into(),
                parent_id: Some(turn_id.into()),
                role: Some(MessageRole::Assistant),
                msg_type: Some(MessageType::ToolCall),
                name: Some("read_file".into()),
                content: Some(MessageContent::Text("{}".into())),
                status: Some(MessageStatus::Completed),
                timestamp: Some(2),
                ..Default::default()
            },
            ChatMessage {
                id: "res1".into(),
                parent_id: Some(tc_id.into()),
                role: Some(MessageRole::Tool),
                msg_type: Some(MessageType::Text),
                content: Some(MessageContent::Text("file content".into())),
                status: Some(MessageStatus::Completed),
                timestamp: Some(3),
                ..Default::default()
            },
        ];

        let natives = flatten_chat_messages(&msgs);
        let tool_native = natives.iter().find(|n| n.role == MessageRole::Tool);
        assert!(tool_native.is_some(), "合法 tool 结果必须保留");
        assert_eq!(tool_native.unwrap().tool_call_id.as_deref(), Some(tc_id));
    }

    /// 空 assistant 回合（无文本/推理/工具调用）不得进入 LLM 请求包，
    /// 否则触发 provider 校验错误。
    #[test]
    fn empty_assistant_turn_is_dropped() {
        let msgs = vec![
            ChatMessage {
                id: "u1".into(),
                parent_id: None,
                role: Some(MessageRole::User),
                msg_type: Some(MessageType::Text),
                content: Some(MessageContent::Text("hi".into())),
                status: Some(MessageStatus::Completed),
                timestamp: Some(1),
                ..Default::default()
            },
            ChatMessage {
                id: "turn-empty".into(),
                parent_id: None,
                role: Some(MessageRole::Assistant),
                msg_type: Some(MessageType::Turn),
                content: None,
                status: Some(MessageStatus::Completed),
                timestamp: Some(2),
                ..Default::default()
            },
        ];

        let natives = flatten_chat_messages(&msgs);
        assert_eq!(natives.len(), 1, "空 assistant 回合不应进入 LLM 请求");
        assert_eq!(natives[0].role, MessageRole::User);
    }

    /// `to_api_value` 对缺失 content 的消息兜底为空串而非 null：
    /// User 与「无工具调用的 assistant」都不能发 `null` 到 provider（否则 400 反序列化失败）。
    #[test]
    fn to_api_value_uses_empty_string_for_missing_content() {
        let user = NativeMessage {
            role: MessageRole::User,
            content: None,
            ..Default::default()
        };
        assert_eq!(
            user.to_api_value()["content"],
            serde_json::json!(""),
            "User 缺 content 应兜底为空串而非 null"
        );

        let assistant_no_tc = NativeMessage {
            role: MessageRole::Assistant,
            content: None,
            ..Default::default()
        };
        assert_eq!(
            assistant_no_tc.to_api_value()["content"],
            serde_json::json!(""),
            "无工具调用的 assistant 缺 content 应兜底为空串而非 null"
        );
    }

    /// `to_api_value` 对「带工具调用的 assistant」保留 content=null（OpenAI 约定），
    /// 不得误改为空串，否则 provider 会拒绝。
    #[test]
    fn to_api_value_keeps_null_for_assistant_with_tool_calls() {
        let tool_calls = vec![ToolCall {
            id: Some("call_1".into()),
            kind: Some("function".into()),
            name: "read_file".into(),
            arguments: serde_json::json!({}),
        }];
        let m = NativeMessage {
            role: MessageRole::Assistant,
            content: None,
            tool_calls: Some(tool_calls),
            ..Default::default()
        };
        let v = m.to_api_value();
        assert_eq!(
            v["content"],
            serde_json::Value::Null,
            "带工具调用的 assistant 允许 content=null（OpenAI 约定）"
        );
        assert!(v.get("tool_calls").is_some(), "tool_calls 必须保留");
    }

    // ── "继续会话"中断可见性 ─────────────────────────────────────────────

    /// 失败 Turn 的半截输出必须进入 LLM 请求包，并附加中断说明：
    /// 模型应看到「上次输出 → 中断说明 → 用户新消息」的连贯序列
    /// （等价于用户打断了模型说话，然后继续）。
    #[test]
    fn failed_turn_visible_with_interrupt_note() {
        let msgs = vec![
            ChatMessage {
                id: "u1".into(),
                parent_id: None,
                role: Some(MessageRole::User),
                msg_type: Some(MessageType::Text),
                content: Some(MessageContent::Text("讲讲并发".into())),
                status: Some(MessageStatus::Completed),
                timestamp: Some(1),
                ..Default::default()
            },
            ChatMessage {
                id: "turn-x".into(),
                parent_id: None,
                role: Some(MessageRole::Assistant),
                msg_type: Some(MessageType::Turn),
                content: None,
                status: Some(MessageStatus::Failed),
                error: Some("用户中止".into()),
                timestamp: Some(2),
                ..Default::default()
            },
            ChatMessage {
                id: "turn-x-text".into(),
                parent_id: Some("turn-x".into()),
                role: Some(MessageRole::Assistant),
                msg_type: Some(MessageType::Text),
                content: Some(MessageContent::Text("Go 的 channel 是…".into())),
                status: Some(MessageStatus::Completed),
                timestamp: Some(2),
                ..Default::default()
            },
            ChatMessage {
                id: "u2".into(),
                parent_id: None,
                role: Some(MessageRole::User),
                msg_type: Some(MessageType::Text),
                content: Some(MessageContent::Text("换个思路".into())),
                status: Some(MessageStatus::Completed),
                timestamp: Some(3),
                ..Default::default()
            },
        ];

        let natives = flatten_chat_messages(&msgs);
        assert_eq!(natives.len(), 3, "user1 + 失败 assistant + user2");
        assert_eq!(natives[0].role, MessageRole::User);
        assert_eq!(natives[1].role, MessageRole::Assistant);
        assert_eq!(natives[2].role, MessageRole::User);

        let text = natives[1].content.as_ref().map(|c| c.to_text()).unwrap();
        assert!(
            text.starts_with("Go 的 channel 是…"),
            "半截输出必须保留在正文开头"
        );
        assert!(text.contains("本轮回复被中断"), "应附加中断说明");
        assert!(text.contains("用户中止"), "中断说明应包含失败原因");
    }

    /// 中断发生在任何输出之前（失败 Turn 无子节点）：Turn 因中断说明非空而保留，
    /// 不得被「空 assistant 回合」清洗误删——否则模型不知道上一轮发生过中断。
    #[test]
    fn failed_turn_without_children_survives_with_note() {
        let msgs = vec![
            ChatMessage {
                id: "u1".into(),
                parent_id: None,
                role: Some(MessageRole::User),
                msg_type: Some(MessageType::Text),
                content: Some(MessageContent::Text("hi".into())),
                status: Some(MessageStatus::Completed),
                timestamp: Some(1),
                ..Default::default()
            },
            ChatMessage {
                id: "turn-empty".into(),
                parent_id: None,
                role: Some(MessageRole::Assistant),
                msg_type: Some(MessageType::Turn),
                content: None,
                status: Some(MessageStatus::Failed),
                timestamp: Some(2),
                ..Default::default()
            },
        ];

        let natives = flatten_chat_messages(&msgs);
        assert_eq!(natives.len(), 2, "失败 Turn 携带中断说明，不得被清洗丢弃");
        assert_eq!(natives[1].role, MessageRole::Assistant);
        let text = natives[1].content.as_ref().map(|c| c.to_text()).unwrap();
        assert!(text.contains("本轮回复被中断"));
    }

    /// 中止打断工具执行（ToolCall 已持久化、结果未产生）：flatten 合成占位
    /// tool 结果，避免请求包出现「有 tool_call 无 tool_result」触发 provider 400。
    #[test]
    fn interrupted_tool_call_gets_placeholder_result() {
        let msgs = vec![
            ChatMessage {
                id: "u1".into(),
                parent_id: None,
                role: Some(MessageRole::User),
                msg_type: Some(MessageType::Text),
                content: Some(MessageContent::Text("跑下测试".into())),
                status: Some(MessageStatus::Completed),
                timestamp: Some(1),
                ..Default::default()
            },
            ChatMessage {
                id: "turn-tc".into(),
                parent_id: None,
                role: Some(MessageRole::Assistant),
                msg_type: Some(MessageType::Turn),
                content: None,
                status: Some(MessageStatus::Failed),
                error: Some("用户中止".into()),
                timestamp: Some(2),
                ..Default::default()
            },
            ChatMessage {
                id: "tc-x".into(),
                parent_id: Some("turn-tc".into()),
                role: Some(MessageRole::Assistant),
                msg_type: Some(MessageType::ToolCall),
                name: Some("run_tests".into()),
                content: Some(MessageContent::Text("{}".into())),
                status: Some(MessageStatus::Completed),
                timestamp: Some(2),
                ..Default::default()
            },
        ];

        let natives = flatten_chat_messages(&msgs);
        let tool = natives
            .iter()
            .find(|n| n.role == MessageRole::Tool)
            .expect("无结果的 ToolCall 必须合成占位 tool 结果");
        assert_eq!(tool.tool_call_id.as_deref(), Some("tc-x"));
        let text = tool.content.as_ref().map(|c| c.to_text()).unwrap();
        assert!(text.contains("run_tests"), "占位结果应指明工具名");
        assert!(text.contains("被中断"), "占位结果应说明执行被中断");

        let assistant = natives
            .iter()
            .find(|n| n.role == MessageRole::Assistant)
            .expect("失败 Turn 仍应聚合为 assistant");
        assert_eq!(
            assistant.tool_calls.as_ref().map(|c| c.len()),
            Some(1),
            "tool_call 本身必须保留"
        );
    }

    /// 失败的工具结果（status=Failed，工具执行出错）推导 success=Some(false)：
    /// 触发 Anthropic tool_result 的 is_error=true；成功的工具结果保持 None。
    #[test]
    fn failed_tool_result_sets_success_false() {
        let mk_turn_with_tool = |turn_id: &str, tc_id: &str, res_id: &str, failed: bool| {
            vec![
                ChatMessage {
                    id: turn_id.into(),
                    parent_id: None,
                    role: Some(MessageRole::Assistant),
                    msg_type: Some(MessageType::Turn),
                    content: None,
                    status: Some(MessageStatus::Completed),
                    timestamp: Some(1),
                    ..Default::default()
                },
                ChatMessage {
                    id: tc_id.into(),
                    parent_id: Some(turn_id.into()),
                    role: Some(MessageRole::Assistant),
                    msg_type: Some(MessageType::ToolCall),
                    name: Some("read_file".into()),
                    content: Some(MessageContent::Text("{}".into())),
                    status: Some(MessageStatus::Completed),
                    timestamp: Some(1),
                    ..Default::default()
                },
                ChatMessage {
                    id: res_id.into(),
                    parent_id: Some(tc_id.into()),
                    role: Some(MessageRole::Tool),
                    msg_type: Some(MessageType::Text),
                    content: Some(MessageContent::Text("result".into())),
                    status: Some(if failed {
                        MessageStatus::Failed
                    } else {
                        MessageStatus::Completed
                    }),
                    timestamp: Some(2),
                    ..Default::default()
                },
            ]
        };

        let mut msgs = mk_turn_with_tool("turn-f", "tc-f", "res-f", true);
        msgs.extend(mk_turn_with_tool("turn-s", "tc-s", "res-s", false));

        let natives = flatten_chat_messages(&msgs);
        let failed_tool = natives
            .iter()
            .find(|n| n.tool_call_id.as_deref() == Some("tc-f"))
            .expect("失败工具结果应保留");
        assert_eq!(failed_tool.role, MessageRole::Tool);
        assert_eq!(
            failed_tool.success,
            Some(false),
            "失败工具结果必须推导 success=false（Anthropic is_error）"
        );

        let ok_tool = natives
            .iter()
            .find(|n| n.tool_call_id.as_deref() == Some("tc-s"))
            .expect("成功工具结果应保留");
        assert_eq!(ok_tool.success, None, "成功工具结果保持 None");
    }
}
