//! `symbio/src/plugins/session/message_build.rs` 的单元测试 —— 与实现同级分文件
//! （约定：`X.rs` + `X.test.rs`）。
//!
//! 本文件是**落库形状**的唯一契约测试（ADR-038 随迁而来的两批用例）：
//! - 原 `symbio_core/llm/turn.test.rs`：`TurnOutput` 三个方法
//!   （`is_reasoning_only` / `effective_text` / `into_messages`）的形状不变式；
//! - 原 `plugins/model/message_builder.test.rs` 的用例 A / A2 / B / C / D 与两条
//!   **流式 id 复用**回归：它们测的一直是**本模块的构造器**，此前只是借住在
//!   model 侧（当时构造器还在 `symbio_core::llm::turn`）。
//!
//! model 侧的 `flatten_chat_messages` 测试改用本地 fixture 构造同形状的树——
//! 插件之间禁止互引（`plugin-entry-audit` E-009），那侧不能再调这里。

use super::*;

const TURN_ID: &str = "turn-0001";

fn out(text: &str, reasoning: &str) -> TurnOutput {
    TurnOutput {
        text: text.to_string(),
        reasoning: reasoning.to_string(),
        ..Default::default()
    }
}

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

fn tool_call(name: &str) -> TurnToolCallInfo {
    TurnToolCallInfo {
        id: Some("tc-1".to_string()),
        wire_id: None,
        name: Some(name.to_string()),
        arguments: serde_json::json!({ "path": "." }),
        parse_error: None,
    }
}

// ── TurnOutput 的读语义（原 core `turn.test.rs`）───────────────────────

#[test]
fn is_reasoning_only_requires_empty_text_and_no_tools() {
    // 只有 reasoning、无正文、无工具 → reasoning-only
    assert!(out("", "思考").is_reasoning_only());
    // 空白正文同样视为「无文本回复」
    assert!(out("  \n ", "思考").is_reasoning_only());
    // 有正文 → 非 reasoning-only
    assert!(!out("回复", "思考").is_reasoning_only());
    // 有工具调用 → 非 reasoning-only（reasoning 需保留为独立子节点）
    let mut with_tool = out("", "思考");
    with_tool.tool_calls.push(tool_call("vdfs_list"));
    assert!(!with_tool.is_reasoning_only());
    // 无 reasoning → 非 reasoning-only
    assert!(!out("", "").is_reasoning_only());
}

#[test]
fn effective_text_falls_back_to_reasoning_only_when_reasoning_only() {
    assert_eq!(out("", "思考").effective_text(), "思考");
    assert_eq!(out("回复", "思考").effective_text(), "回复");
    // 有工具时不回退，正文为空即为空
    let mut with_tool = out("", "思考");
    with_tool.tool_calls.push(tool_call("vdfs_list"));
    assert_eq!(with_tool.effective_text(), "");
}

/// 端到端（纯内存）：reasoning-only 的 TurnOutput 落库消息里
/// 同一段 reasoning 只出现一次，且没有 Reasoning 子节点。
#[test]
fn into_messages_reasoning_only_has_no_duplicate_content() {
    let reasoning = "让我想想这个问题的关键点。";
    let msgs = out("", reasoning).into_messages("turn-x");

    assert_eq!(msgs.len(), 2, "应为 Turn + 单个 Text 子节点");
    assert_eq!(msgs[0].msg_type, Some(MessageType::Turn));
    assert!(
        !msgs
            .iter()
            .any(|m| m.msg_type == Some(MessageType::Reasoning)),
        "reasoning-only 不得产生 Reasoning 子节点"
    );

    let occurrences = msgs
        .iter()
        .filter(|m| {
            m.content
                .as_ref()
                .map(|c| c.to_text().contains(reasoning))
                .unwrap_or(false)
        })
        .count();
    assert_eq!(occurrences, 1, "同一段 reasoning 只能落库一份（factor=1）");
}

#[test]
fn into_messages_reasoning_with_reply_keeps_two_children() {
    let msgs = out("这是回复", "这是思考").into_messages("turn-y");
    assert_eq!(msgs.len(), 3);
    assert_eq!(
        msgs.iter()
            .filter(|m| m.msg_type == Some(MessageType::Reasoning))
            .count(),
        1
    );
    assert_eq!(
        msgs.iter()
            .filter(|m| m.msg_type == Some(MessageType::Text))
            .count(),
        1
    );
}

/// 形状不变式：`tool_calls` 是**结果形态**的唯一来源——`into_messages` 必须把字段里
/// 的工具调用原样落成 `ToolCall` 子节点（节点 id 与流式帧一致、参数落 JSON 文本）。
///
/// 这条锁的是「过程 → 结果」那次收口的接线：此前 `into_messages` 现调累积器的
/// `get_completed()`，工具调用的 id 由**另一次**调用产生，两处不一致即孤儿节点。
#[test]
fn into_messages_carries_tool_calls_from_the_result_field() {
    let mut o = out("", "");
    o.tool_calls.push(tool_call("vdfs_list"));
    let msgs = o.into_messages("turn-z");

    let calls: Vec<_> = msgs
        .iter()
        .filter(|m| m.msg_type == Some(MessageType::ToolCall))
        .collect();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].id, "tc-1", "落库 id 必须等于产物里的节点 id");
    assert_eq!(calls[0].parent_id.as_deref(), Some("turn-z"));
    assert_eq!(calls[0].name.as_deref(), Some("vdfs_list"));
    assert_eq!(
        calls[0].content.as_ref().map(|c| c.to_text()).as_deref(),
        Some(r#"{"path":"."}"#)
    );
}

// ── 构造器的落库形状（原 model `message_builder.test.rs` 的用例 A–D）──

/// 用例 A（回归：storage factor≈2 重复）
///
/// reasoning-only 场景（无独立文本回复，`effective_text` 回退为 reasoning）：
/// 只能落 `Turn` + 一个 `Text` 子节点；**绝不能**再生成 `Reasoning` 子节点，
/// 否则同一段 reasoning 在存储层出现两份（历史会话打开显示重复两份）。
#[test]
fn reasoning_only_writes_single_text_child_without_reasoning_node() {
    let reasoning = "我先分析一下用户的问题，然后给出结论。";

    let msgs = llm_build_assistant_messages(
        TURN_ID,
        reasoning,
        &[],
        None,
        Some(reasoning.into()),
        TurnStreamChildIds::default(),
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

    let msgs = llm_build_assistant_messages(
        TURN_ID,
        content,
        &[],
        None,
        Some(reasoning.into()),
        TurnStreamChildIds::default(),
    );

    assert_eq!(msgs.len(), 2);
    assert_eq!(count_children(&msgs, MessageType::Reasoning), 0);
    assert_eq!(count_children(&msgs, MessageType::Text), 1);
}

/// 用例 B：普通场景（reasoning + 独立文本回复）→ Reasoning 与 Text 各一份，内容不同
#[test]
fn reasoning_with_distinct_reply_keeps_both_children() {
    let msgs = llm_build_assistant_messages(
        TURN_ID,
        "正常回复",
        &[],
        Some("resp-1".into()),
        Some("思考过程".into()),
        TurnStreamChildIds::default(),
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
    let msgs = llm_build_assistant_messages(
        TURN_ID,
        "你好",
        &[],
        None,
        None,
        TurnStreamChildIds::default(),
    );
    assert_eq!(msgs.len(), 2);
    assert_eq!(count_children(&msgs, MessageType::Reasoning), 0);
    assert_eq!(child_texts(&msgs, MessageType::Text), vec!["你好"]);
}

/// 用例 D：有 reasoning + 无文本 + 有工具调用（非 reasoning-only）
/// → 保留 Reasoning 子节点，且不生成空 Text 节点
#[test]
fn reasoning_with_tool_calls_keeps_reasoning_and_skips_empty_text() {
    let tools = vec![tool_call("read_file")];
    let msgs = llm_build_assistant_messages(
        TURN_ID,
        "",
        &tools,
        None,
        Some("要先读文件".into()),
        TurnStreamChildIds::default(),
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

// ── 流式 id 复用回归（锁「流式层 ⇄ 存储层同一身份」）───────────────────

/// 落库节点必须复用流式子节点 id。
///
/// 若两处各自 `llm_short_id()`，存储层的定稿节点（id=B，内容全量）与会话层累积的流式节点
/// （id=A，内容增量合并）会被判定为两条不同消息；失败收尾时 id=A 被当作"尚未落库的
/// 流式半截"补写进存储 → 同一个 Turn 下出现两份内容相同的文本节点。
#[test]
fn persisted_children_reuse_streaming_child_ids() {
    let msgs = llm_build_assistant_messages(
        TURN_ID,
        "正常回复",
        &[],
        None,
        Some("思考过程".into()),
        TurnStreamChildIds {
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
    let msgs = llm_build_assistant_messages(
        TURN_ID,
        reasoning,
        &[],
        None,
        Some(reasoning.into()),
        TurnStreamChildIds {
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
