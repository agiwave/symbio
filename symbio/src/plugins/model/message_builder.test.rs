//! `symbio/src/plugins/model/message_builder.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。

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
        wire_id: None,
        name: Some(name.to_string()),
        arguments: serde_json::json!({ "k": "v" }),
        parse_error: None,
    }
}

/// 用例（评审 §3.2）：ToolCall 节点 content 已是规范化 JSON 文本（落库时
/// `tc.arguments.to_string()` 产出），flatten 必须**以字符串原样透传**，
/// 让下游 types.rs 走 `is_string()` 快路径、省一次 re-serialize。
/// 同时钉死「不会把合法参数静默改成 `{}`」——这是改动前的行为保真要求。
#[test]
fn tool_call_args_passthrough_as_string_without_reserialize() {
    // ToolCall 必须挂在根级 Turn（assistant）之下才会被聚合成 tool_calls。
    let turn = ChatMessage {
        id: TURN_ID.to_string(),
        parent_id: None,
        role: Some(MessageRole::Assistant),
        msg_type: Some(MessageType::Turn),
        status: Some(MessageStatus::Completed),
        ..Default::default()
    };

    let tc = ChatMessage {
        id: "tc-1".to_string(),
        parent_id: Some(TURN_ID.to_string()),
        role: Some(MessageRole::Assistant),
        msg_type: Some(MessageType::ToolCall),
        name: Some("echo".to_string()),
        content: Some(MessageContent::Text(
            r#"{"text":"mock 回显内容"}"#.to_string(),
        )),
        status: Some(MessageStatus::Completed),
        ..Default::default()
    };

    let native = flatten_chat_messages(&[turn, tc]);
    let tool_calls = native
        .iter()
        .find_map(|m| m.tool_calls.as_ref())
        .expect("应聚合出 tool_calls");
    let call = tool_calls.first().expect("应有一个 tool_call");

    assert!(
        call.arguments.is_string(),
        "arguments 应为字符串透传（评审 §3.2），而非重新物化的 Value 对象"
    );
    assert_eq!(
        call.arguments.as_str().unwrap(),
        r#"{"text":"mock 回显内容"}"#,
        "落库文本应原样透传给 LLM 请求，不得被 re-serialize 改动"
    );
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

/// 压缩节点**不是对话内容**，不得进入请求包。
///
/// 它是系统对历史的一次整理动作，只服务于前端呈现与事后审计。发进请求包
/// 既白占上下文，又会让模型把"系统整理过上下文"当成一条事实陈述读进去。
#[test]
fn compression_nodes_are_excluded_from_request_view() {
    let msgs = vec![
        ChatMessage {
            id: "u1".into(),
            role: Some(MessageRole::User),
            msg_type: Some(MessageType::Text),
            content: Some(MessageContent::Text("问题".into())),
            ..Default::default()
        },
        ChatMessage {
            id: "c1".into(),
            role: Some(MessageRole::Assistant),
            msg_type: Some(MessageType::Compression),
            content: Some(MessageContent::Text("已压缩上下文（12 → 4 条）".into())),
            status: Some(MessageStatus::Completed),
            ..Default::default()
        },
    ];
    let natives = flatten_chat_messages(&msgs);
    assert_eq!(natives.len(), 1, "压缩节点必须被剔除出请求包");
    assert_eq!(natives[0].role, MessageRole::User);
}
