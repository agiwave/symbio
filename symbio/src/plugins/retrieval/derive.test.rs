//! 检索派生的单元测试：`turn.*` / `artifact.*` / `memory.*` 三组格子、确定性、溯源。

use super::*;
use crate::symbio_core::schemas::session::chat_message::{ChatMessage, MessageRole, MessageType};

/// 造一条消息（`seq` 由调用方给，模拟存储分配）。
fn msg(seq: i64, role: MessageRole, ty: Option<MessageType>) -> ChatMessage {
    ChatMessage {
        id: format!("m{seq}"),
        parent_id: None,
        role: Some(role),
        msg_type: ty,
        name: None,
        prompt: None,
        content: None,
        delta: None,
        status: None,
        error: None,
        meta: None,
        timestamp: Some(seq * 10),
        seq: Some(seq),
        response_id: None,
        tool_call_id: None,
    }
}

fn derive_one(msgs: &[ChatMessage], memory: &[String]) -> Vec<Fact> {
    let mut inputs = vec![SessionFactsInput {
        session_id: "s1",
        session_ordinal: 0,
        messages: msgs,
        memory_lines: memory,
    }];
    sort_inputs(&mut inputs);
    derive_facts(&inputs, 0)
}

/// 一轮对话：user → assistant。事实类型正确、`seq` 严格递增。
#[test]
fn derives_turn_facts_in_order() {
    let msgs = vec![
        msg(1, MessageRole::User, Some(MessageType::Text)),
        msg(2, MessageRole::Assistant, Some(MessageType::Text)),
    ];
    let facts = derive_one(&msgs, &[]);
    assert_eq!(facts.len(), 2);
    assert_eq!(facts[0].kind, FactKind::TurnUserMessage);
    assert_eq!(facts[1].kind, FactKind::TurnAssistantFinal);
    assert!(facts[0].seq < facts[1].seq);
    // 助手事实溯源到用户事实（I2）
    assert_eq!(facts[1].caused_by, Some(facts[0].seq));
}

/// **无记忆 ⇒ 一条 `memory.*` 都不产生**（平凡值路径）。
#[test]
fn empty_memory_yields_no_memory_facts() {
    let msgs = vec![msg(1, MessageRole::User, Some(MessageType::Text))];
    let facts = derive_one(&msgs, &[]);
    assert!(
        facts.iter().all(|f| f.kind.entity() != "memory"),
        "空 MEMORY.md 不应产生 memory.* 事实"
    );
}

/// **有记忆 ⇒ `memory.encoded` + 每行一条 `memory.recalled`，且召回带溯源（I2）**。
#[test]
fn memory_lines_become_encoded_plus_recalls() {
    let msgs = vec![msg(1, MessageRole::User, Some(MessageType::Text))];
    let memory = vec!["钉住的结论 A".to_string(), "约束 B".to_string()];
    let facts = derive_one(&msgs, &memory);

    let encoded: Vec<&Fact> = facts
        .iter()
        .filter(|f| f.kind == FactKind::MemoryEncoded)
        .collect();
    assert_eq!(encoded.len(), 1, "非空记忆应恰有一条 memory.encoded");
    assert_eq!(encoded[0].caused_by, None);

    let recalled: Vec<&Fact> = facts
        .iter()
        .filter(|f| f.kind == FactKind::MemoryRecalled)
        .collect();
    assert_eq!(recalled.len(), 2, "每一条非空记忆行 ⇒ 一条 memory.recalled");
    for r in &recalled {
        assert_eq!(
            r.caused_by,
            Some(encoded[0].seq),
            "召回必须溯源到记忆编码（I2：断言类不得无溯源）"
        );
        assert!(r.has_provenance(), "memory.recalled 是断言类，必须带溯源");
        assert!(r.provenance_points_back(), "溯源必须指向更早的 seq");
    }
    // 记忆事实的 seq 都在会话区间内、且严格递增
    let mut seqs: Vec<u64> = facts.iter().map(|f| f.seq).collect();
    let sorted = seqs.clone();
    seqs.sort_unstable();
    assert_eq!(seqs, sorted, "派生结果应已按 seq 升序");
    for w in seqs.windows(2) {
        assert!(w[0] < w[1], "seq 必须严格递增");
    }
}

/// 确定性（A4）：同一份输入两次派生，逐字节相同。
#[test]
fn derivation_is_deterministic() {
    let msgs = vec![
        msg(1, MessageRole::User, Some(MessageType::Text)),
        msg(2, MessageRole::Assistant, Some(MessageType::Text)),
    ];
    let memory = vec!["A".to_string(), "B".to_string()];
    let a = derive_one(&msgs, &memory);
    let b = derive_one(&msgs, &memory);
    assert_eq!(
        serde_json::to_string(&a).unwrap(),
        serde_json::to_string(&b).unwrap(),
        "同一份输入两次派生必须逐字节相同"
    );
}

/// 会话间 `seq` 不重叠：两个会话的事实落进各自的步长区间。
#[test]
fn sessions_do_not_overlap_in_seq() {
    let msgs = vec![msg(1, MessageRole::User, Some(MessageType::Text))];
    let mut inputs = vec![
        SessionFactsInput {
            session_id: "b",
            session_ordinal: 0,
            messages: &msgs,
            memory_lines: &[],
        },
        SessionFactsInput {
            session_id: "a",
            session_ordinal: 0,
            messages: &msgs,
            memory_lines: &[],
        },
    ];
    sort_inputs(&mut inputs);
    // 字典序：a 在前
    assert_eq!(inputs[0].session_id, "a");
    assert_eq!(inputs[0].session_ordinal, 1);
    assert_eq!(inputs[1].session_ordinal, 2);
    let facts = derive_facts(&inputs, 0);
    let a_seq = facts
        .iter()
        .find(|f| f.principal.as_str() == "a")
        .unwrap()
        .seq;
    let b_seq = facts
        .iter()
        .find(|f| f.principal.as_str() == "b")
        .unwrap()
        .seq;
    assert!(a_seq < b_seq, "字典序在前的会话 seq 应更小");
    assert!(
        b_seq - a_seq >= SESSION_STRIDE,
        "两会话之间至少隔一个步长，不重叠"
    );
}

/// `max_facts` 截断生效。
#[test]
fn max_facts_truncates() {
    let msgs = vec![
        msg(1, MessageRole::User, Some(MessageType::Text)),
        msg(2, MessageRole::Assistant, Some(MessageType::Text)),
        msg(3, MessageRole::User, Some(MessageType::Text)),
    ];
    let mut inputs = vec![SessionFactsInput {
        session_id: "s1",
        session_ordinal: 0,
        messages: &msgs,
        memory_lines: &[],
    }];
    sort_inputs(&mut inputs);
    let facts = derive_facts(&inputs, 2);
    assert_eq!(facts.len(), 2, "max_facts 应截断");
}

/// 无 `seq` 的消息被跳过（不猜号——猜号会破坏确定性）。
#[test]
fn message_without_seq_is_skipped() {
    let mut m = msg(0, MessageRole::User, Some(MessageType::Text));
    m.seq = None;
    let facts = derive_one(&[m], &[]);
    assert!(facts.is_empty(), "无 seq 的消息不产生事实");
}
