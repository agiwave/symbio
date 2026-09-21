//! `transcript.rs` 的单元测试（`Transcript::apply` 的图语义与线上帧格式）。
//!
//! 与实现**同级**分文件（约定：`X.rs` + `X.test.rs`）：测试跟着被测试的实现走。

use super::*;
use crate::symbio_core::schemas::session::chat_message::{
    MessageContent, MessageRole, MessageStatus, MessageType,
};

fn text_msg(id: &str, status: MessageStatus, text: &str) -> cm::ChatMessage {
    cm::ChatMessage {
        id: id.to_string(),
        role: Some(MessageRole::Assistant),
        msg_type: Some(MessageType::Text),
        status: Some(status),
        content: Some(MessageContent::Text(text.to_string())),
        ..Default::default()
    }
}

/// seq 严格单调：每个被发布的帧 +1；违例帧（未知 id 追加）不占号。
#[test]
fn seq_is_monotonic_and_violations_consume_no_seq() {
    let mut tr = Transcript::new("s1".into());
    tr.apply(NodeOp::Upsert {
        message: Box::new(text_msg("a", MessageStatus::Streaming, "你")),
    });
    tr.apply(NodeOp::Append {
        message_id: "a".into(),
        delta: "好".into(),
    });
    // 违例：未知 id 追加 → 丢弃、不占 seq
    tr.apply(NodeOp::Append {
        message_id: "ghost".into(),
        delta: "x".into(),
    });
    tr.apply(NodeOp::Upsert {
        message: Box::new(text_msg("a", MessageStatus::Completed, "你好")),
    });
    assert_eq!(tr.seq, 3, "违例帧不得消耗 seq（3 个合法帧）");
    let a = tr.get("a").unwrap();
    match a.content {
        Some(MessageContent::Text(t)) => assert_eq!(t, "你好", "Upsert 是整条替换"),
        _ => panic!("应为 Text"),
    }
}

/// Append 逐字累积到图内节点（Upsert 全量替换）。
#[test]
fn append_accumulates_and_upsert_replaces() {
    let mut tr = Transcript::new("s1".into());
    tr.apply(NodeOp::Upsert {
        message: Box::new(text_msg("a", MessageStatus::Streaming, "")),
    });
    tr.apply(NodeOp::Append {
        message_id: "a".into(),
        delta: "ab".into(),
    });
    tr.apply(NodeOp::Append {
        message_id: "a".into(),
        delta: "cd".into(),
    });
    tr.apply(NodeOp::Upsert {
        message: Box::new(text_msg("a", MessageStatus::Completed, "XYZ")),
    });
    match tr.get("a").unwrap().content {
        Some(MessageContent::Text(t)) => assert_eq!(t, "XYZ", "Upsert 是整条替换"),
        _ => panic!("应为 Text"),
    }
}

/// Remove / Reset / persisted 的图语义。
#[test]
fn remove_reset_and_persisted_evict() {
    let mut tr = Transcript::new("s1".into());
    tr.apply(NodeOp::Upsert {
        message: Box::new(text_msg("a", MessageStatus::Streaming, "1")),
    });
    tr.apply(NodeOp::Upsert {
        message: Box::new(text_msg("b", MessageStatus::Streaming, "2")),
    });
    tr.persisted(&["a".to_string()]);
    assert!(tr.get("a").is_none(), "persisted 后节点离开在途图");
    tr.apply(NodeOp::Remove {
        message_id: "b".into(),
    });
    assert!(tr.is_empty());
    tr.apply(NodeOp::Upsert {
        message: Box::new(text_msg("c", MessageStatus::Streaming, "3")),
    });
    tr.apply(NodeOp::Reset);
    assert!(tr.is_empty(), "Reset 清空在途图");
}

/// 线上格式：NodeEvent 序列化稳定（前端契约）。
#[test]
fn node_event_serializes_with_tagged_op() {
    let event = NodeEvent {
        session_id: "s1".into(),
        seq: 1,
        op: NodeOp::Append {
            message_id: "a".into(),
            delta: "!".into(),
        },
    };
    let v = serde_json::to_value(&event).unwrap();
    assert_eq!(v["session_id"], "s1");
    assert_eq!(v["seq"], 1);
    assert_eq!(v["op"], "append");
    let back: NodeEvent = serde_json::from_value(v).unwrap();
    assert_eq!(back.seq, 1);
}
