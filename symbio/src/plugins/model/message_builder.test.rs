//! `symbio/src/plugins/model/message_builder.rs` 的单元测试 —— 拆自源码末尾的测试模块。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）。
//!
//! ## 落库树在这里是**本地 fixture**
//!
//! 真正的构造器（`llm_build_assistant_messages` / `TurnStreamChildIds` /
//! `TurnOutput::into_messages`）生产上只有 session 一个消费方，已随 ADR-038 从
//! `symbio_core::llm::turn` 下沉到 `plugins/session/message_build.rs`；
//! 插件之间禁止互引（`plugin-entry-audit` E-009），本文件不能再调它。
//!
//! 于是两侧**各锁一半**，形状靠注释互指：
//! - 形状（构造器产出什么）→ `plugins/session/message_build.test.rs` 的
//!   用例 A / A2 / B / C / D 与流式 id 复用回归；
//! - 视图（给定同形状的树怎么扁平化）→ 本文件的用例 E / F 及其余 flatten 用例。

use super::*;

use crate::symbio_core::schemas::session::chat_message::MessageStatus;

const TURN_ID: &str = "turn-0001";

/// 本地构造一棵落库树（Turn 根 + 传入的子节点），形状同
/// `llm_build_assistant_messages` 的产物——形状契约见文件头说明。
fn turn_tree(children: Vec<ChatMessage>) -> Vec<ChatMessage> {
    let mut msgs = vec![ChatMessage {
        id: TURN_ID.to_string(),
        parent_id: None,
        role: Some(MessageRole::Assistant),
        msg_type: Some(MessageType::Turn),
        content: None,
        status: Some(MessageStatus::Completed),
        timestamp: Some(1),
        ..Default::default()
    }];
    msgs.extend(children);
    msgs
}

/// 落库树的一个子节点（`parent_id` 指向 [`TURN_ID`]）。
fn child(id: &str, ty: MessageType, text: &str) -> ChatMessage {
    ChatMessage {
        id: id.to_string(),
        parent_id: Some(TURN_ID.to_string()),
        role: Some(MessageRole::Assistant),
        msg_type: Some(ty),
        content: Some(MessageContent::Text(text.into())),
        status: Some(MessageStatus::Completed),
        timestamp: Some(1),
        ..Default::default()
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

/// 用例 E：reasoning-only 落库结果扁平化后，LLM 请求里内容只出现一次
/// （防止重复内容顺着 request 包再放大一次）
///
/// 输入是本地 fixture：reasoning-only 的落库形状 = Turn + 单个 Text 子节点。
/// 「构造器不生成 Reasoning 子节点」这半边锁在
/// `plugins/session/message_build.test.rs` 的用例 A（见文件头说明）。
#[test]
fn reasoning_only_flattens_to_single_assistant_message() {
    let reasoning = "只有思考";
    let msgs = turn_tree(vec![child(
        "turn-0001-reason",
        MessageType::Text,
        reasoning,
    )]);

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
///
/// 输入是本地 fixture（Turn + Reasoning + Text 三节点）；三节点的产出形状锁在
/// `plugins/session/message_build.test.rs` 的用例 B（见文件头说明）。
#[test]
fn reasoning_with_reply_flattens_into_content_and_reasoning_content() {
    let msgs = turn_tree(vec![
        child("turn-0001-reason", MessageType::Reasoning, "思考过程"),
        child("turn-0001-text", MessageType::Text, "正常回复"),
    ]);

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
