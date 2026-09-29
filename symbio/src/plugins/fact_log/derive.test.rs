//! fact_log 派生自测 —— **确定性**（A4）、**seq 严格递增**、**I2 溯源**。

use super::*;
use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType,
};

fn msg(
    id: &str,
    role: MessageRole,
    seq: i64,
    ty: Option<MessageType>,
    status: Option<MessageStatus>,
) -> ChatMessage {
    ChatMessage {
        id: id.into(),
        parent_id: None,
        role: Some(role),
        msg_type: ty,
        name: None,
        prompt: None,
        content: Some(MessageContent::Text(format!("m{id}"))),
        delta: None,
        status,
        error: None,
        meta: None,
        timestamp: Some(1_000 + seq),
        seq: Some(seq),
        response_id: None,
        tool_call_id: None,
    }
}

/// 两条会话的典型消息序列：用户 → 工具调用 → 工具结果 → 助手终态。
fn sample() -> Vec<ChatMessage> {
    vec![
        msg("u1", MessageRole::User, 1, None, None),
        msg(
            "t1",
            MessageRole::Assistant,
            2,
            Some(MessageType::ToolCall),
            None,
        ),
        msg("r1", MessageRole::Tool, 3, None, None),
        msg("a1", MessageRole::Assistant, 4, None, None),
    ]
}

fn inputs<'a>(order: &[(&'a str, &'a [ChatMessage])]) -> Vec<SessionFactsInput<'a>> {
    let mut v: Vec<SessionFactsInput<'a>> = order
        .iter()
        .map(|(id, msgs)| SessionFactsInput {
            session_id: id,
            session_ordinal: 0,
            messages: msgs,
        })
        .collect();
    sort_inputs(&mut v);
    v
}

/// 基线：四种消息各产出一格，工具消息产出 artifact 事实。
#[test]
fn maps_message_kinds() {
    let m = sample();
    let facts = derive_facts(&inputs(&[("s1", &m)]), 0);
    let kinds: Vec<&str> = facts.iter().map(|f| f.kind.wire()).collect();
    assert_eq!(
        kinds,
        vec![
            "turn.user_message",
            "artifact.added",
            "artifact.added",
            "turn.assistant_final",
        ]
    );
}

/// **A4 确定性**：同一份输入双跑，逐字节相同。
#[test]
fn deterministic_across_runs() {
    let m = sample();
    let a = derive_facts(&inputs(&[("s1", &m)]), 0);
    let b = derive_facts(&inputs(&[("s1", &m)]), 0);
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        serde_json::to_string(&b).unwrap(),
        "双跑结果必须逐字节相同（A4）"
    );
}

/// **A4 确定性 II**：输入顺序无关——乱序传入，排序后结果相同。
#[test]
fn deterministic_under_input_permutation() {
    let m1 = sample();
    let m2 = sample();
    let fwd = derive_facts(&inputs(&[("s1", &m1), ("s2", &m2)]), 0);
    let rev = derive_facts(&inputs(&[("s2", &m2), ("s1", &m1)]), 0);
    assert_eq!(
        serde_json::to_string(&fwd).unwrap(),
        serde_json::to_string(&rev).unwrap(),
        "输入排列不应改变结果（session_ordinal 由 id 字典序决定）"
    );
}

/// seq 严格递增（回放与溯源的基础）。
#[test]
fn seq_strictly_increasing() {
    let m1 = sample();
    let m2 = sample();
    let facts = derive_facts(&inputs(&[("s1", &m1), ("s2", &m2)]), 0);
    let mut prev = 0u64;
    for f in &facts {
        assert!(f.seq > prev, "seq 未严格递增：{prev} → {}", f.seq);
        prev = f.seq;
    }
}

/// 不同会话落在不同区间（不串号）。
#[test]
fn sessions_do_not_collide() {
    let m1 = sample();
    let m2 = sample();
    let facts = derive_facts(&inputs(&[("s1", &m1), ("s2", &m2)]), 0);
    // 两条会话各 4 条事实，且序号区间不重叠
    let s1: Vec<u64> = facts
        .iter()
        .filter(|f| f.principal.as_str() == "s1")
        .map(|f| f.seq)
        .collect();
    let s2: Vec<u64> = facts
        .iter()
        .filter(|f| f.principal.as_str() == "s2")
        .map(|f| f.seq)
        .collect();
    assert_eq!(s1.len(), 4);
    assert_eq!(s2.len(), 4);
    assert!(
        s1.iter().max() < s2.iter().min(),
        "s1 的 seq 区间应完全早于 s2"
    );
}

/// **I2 溯源**：助手终态指向本会话最近的用户消息；工具事实指向最近的助手消息。
#[test]
fn provenance_points_back() {
    let m = sample();
    let facts = derive_facts(&inputs(&[("s1", &m)]), 0);
    for f in &facts {
        assert!(
            f.provenance_points_back(),
            "溯源必须指向更早的 seq（{} → {:?}）",
            f.seq,
            f.caused_by
        );
    }
    let user_seq = facts
        .iter()
        .find(|f| f.kind == FactKind::TurnUserMessage)
        .unwrap()
        .seq;
    let final_fact = facts
        .iter()
        .find(|f| f.kind == FactKind::TurnAssistantFinal)
        .unwrap();
    assert_eq!(
        final_fact.caused_by,
        Some(user_seq),
        "助手终态应溯源到最近的用户消息"
    );
}

/// 失败状态的助手消息落成 fallback（到点必答的兜底），不是 final。
#[test]
fn failed_assistant_is_fallback() {
    let m = vec![
        msg("u1", MessageRole::User, 1, None, None),
        msg(
            "a1",
            MessageRole::Assistant,
            2,
            None,
            Some(MessageStatus::Failed),
        ),
    ];
    let facts = derive_facts(&inputs(&[("s1", &m)]), 0);
    assert!(
        facts
            .iter()
            .any(|f| f.kind == FactKind::TurnAssistantFallback),
        "失败助手应落成 fallback"
    );
    assert!(
        !facts.iter().any(|f| f.kind == FactKind::TurnAssistantFinal),
        "不应同时落成 final"
    );
}

/// 无 seq 的消息被跳过（不猜号——猜号会破坏确定性）。
#[test]
fn missing_seq_is_skipped() {
    let mut bad = msg("u1", MessageRole::User, 1, None, None);
    bad.seq = None;
    let m = vec![bad];
    let facts = derive_facts(&inputs(&[("s1", &m)]), 0);
    assert!(facts.is_empty(), "无 seq 的消息应被跳过");
}

/// 截断：`max_facts` 生效。
#[test]
fn truncation_respects_limit() {
    let m1 = sample();
    let m2 = sample();
    let all = derive_facts(&inputs(&[("s1", &m1), ("s2", &m2)]), 0);
    assert_eq!(all.len(), 8);
    let cut = derive_facts(&inputs(&[("s1", &m1), ("s2", &m2)]), 3);
    assert_eq!(cut.len(), 3, "max_facts 应截断");
    // 截断保留的是前 3 条（seq 最小的）
    assert_eq!(cut[0].seq, all[0].seq);
    assert_eq!(cut[2].seq, all[2].seq);
}
